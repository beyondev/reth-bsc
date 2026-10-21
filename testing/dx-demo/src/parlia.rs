//! Today's flow: Parlia block production with fast-finality votes.
//!
//! The in-turn proposer builds and executes a full block, broadcasts it, and every validator
//! re-executes it before voting. When it seals, the next proposer attests the nearest of the
//! last `attest_depth` ancestors above the justified block that has a quorum of verified votes.
//! As in geth's `Snapshot.updateAttestation`, an attestation whose target is the block right
//! after its source finalizes the source; any other attestation only moves the justified block.
//! When the turn passes to another validator, that validator must import the parent first.
//!
//! With `--stream-chunk-ms` the proposer streams the block while building it: every chunk of
//! newly appended transactions is signed and relayed at once, and validators that hold the parent
//! execute it speculatively, so after the seal they only execute what is left. A builder bid
//! (`--bid-share`) goes first in the block and can only be streamed once it is locked
//! (`--lock-ms`).
//!
//! When the in-turn validator is down, or its blocks are rejected because it executes wrongly,
//! the honest validator with the best backoff rank produces the block after `rank * backoff`.
//! A wrongly executing validator still broadcasts its own blocks; honest validators execute and
//! reject them and it never votes for theirs.

use std::collections::{BTreeMap, HashMap};

use crate::{
    exec::BackgroundExec,
    net::Network,
    realnet::Cluster,
    report::{median, percentile_ms, BlockStats, Report, RunOutput, TxTimes},
    sim::{Cpu, EventQueue},
    Env,
};

/// Header plus seal and attestation, in bytes.
const HEADER_BYTES: u64 = 700;
/// One fast-finality vote message, in bytes.
const VOTE_BYTES: u64 = 200;
/// Framing of a stream chunk: sequence number, previous chunk hash, cumulative gas and receipts
/// hash, and the proposer's signature.
const CHUNK_OVERHEAD_BYTES: u64 = 200;

#[derive(Debug)]
enum Event {
    /// Validator `v` starts building canonical block `h`.
    Build { h: u64, v: usize },
    /// Canonical block `h` is sealed and broadcast.
    Seal(u64),
    /// The wrongly executing in-turn validator broadcasts its own block for height `h`.
    BadBlock(u64),
    /// A block reaches validator `v`.
    Arrive { v: usize, block: BlockRef },
    /// Validator `v` finished importing canonical block `h`.
    Imported { v: usize, h: u64 },
    /// The proposer of block `h` sends stream chunk `seq`.
    SendChunk { h: u64, seq: usize },
    /// Stream chunk `seq` of block `h` reaches validator `v`.
    Chunk { v: usize, h: u64, seq: usize },
    /// A vote for block `h` reaches validator `v`.
    Vote { v: usize, h: u64 },
}

#[derive(Debug, Clone, Copy)]
enum BlockRef {
    Canonical(u64),
    Rejected(usize),
}

#[derive(Debug, Default)]
struct Block {
    proposer: usize,
    txs: Vec<usize>,
    target: u64,
    seal: u64,
    exec_cost: f64,
    bytes: u64,
    /// Block justified by this header's attestation, if it carries one.
    attest_target: Option<u64>,
    /// Number of attestations this header carries.
    attestations: u64,
    /// Justified and finalized block after applying this header.
    justified: u64,
    finalized: u64,
    out_of_turn: bool,
    /// Validators the proposer sends the block (and its stream chunks) to itself.
    first_hop: Vec<usize>,
    /// Peers each validator forwards the block (and its stream chunks) to after receiving it.
    relay: Vec<Vec<usize>>,
    /// Stream chunks sent before the seal; the seal message carries the remaining transactions.
    chunks: Vec<Chunk>,
}

/// Transactions the proposer streamed ahead of the seal.
#[derive(Debug)]
struct Chunk {
    send_at: u64,
    txs: usize,
    /// Share of the block's execution cost these transactions account for.
    cost: f64,
    bytes: u64,
}

/// A validator's speculative execution of one block's stream.
#[derive(Debug, Default)]
struct Stream {
    /// Next chunk to execute.
    next: usize,
    /// Chunks received and signature-checked but not executed yet, by sequence number.
    ready: BTreeMap<usize, u64>,
    /// When execution of the chunks so far finishes.
    done: u64,
    cost: f64,
    txs: usize,
    /// Set once the block is imported; later chunks are ignored.
    closed: bool,
}

#[derive(Debug)]
struct RejectedBlock {
    sent: u64,
    exec_cost: f64,
    bytes: u64,
}

/// Runs the Parlia simulation and returns its report.
pub fn run(env: &Env<'_>) -> Report {
    let cfg = env.cfg;
    let n = cfg.validators;
    let txs = &env.workload.txs;
    let interval = cfg.block_interval_ms * 1000;
    let end = ((cfg.duration_s + env.drain_s) * 1e6) as u64;
    let window = ((cfg.warmup_s * 1e6) as u64, (cfg.duration_s * 1e6) as u64);
    let honest: Vec<usize> = (0..n).filter(|&v| cfg.is_honest(v)).collect();

    let mut q = EventQueue::default();
    let mut cpu = Cpu::new(n, cfg.sig_cores, window);
    let cluster = env.realnet.map(|spec| Cluster::launch(spec, n));
    let mut net = Network::new(
        n,
        cfg.bandwidth_mbps,
        window.0,
        (cfg.jitter_ms * 1000.0) as u64,
        cfg.seed,
        cluster,
    );
    let engine = BackgroundExec::new(env.workload.accounts);

    let in_turn = |h: u64| ((h / cfg.turn_length) as usize) % n;
    // Who builds canonical block `h` and how long after the block time it may seal.
    let builder = |h: u64| -> (usize, u64) {
        let p = in_turn(h);
        if cfg.is_honest(p) {
            return (p, 0);
        }
        let rank = |v: usize| splitmix(h ^ ((v as u64) << 32));
        let mut others: Vec<usize> = (0..n).filter(|&v| v != p).collect();
        others.sort_by_key(|&v| rank(v));
        let (pos, v) = others
            .iter()
            .enumerate()
            .find(|(_, &v)| cfg.is_honest(v))
            .expect("an honest validator");
        (*v, (pos as u64 + 1) * cfg.backoff_ms * 1000)
    };

    // blocks[h], with blocks[0] the genesis.
    let mut blocks: Vec<Block> = vec![Block::default()];
    let mut rejected: Vec<RejectedBlock> = Vec::new();
    // imported[h][v] is when validator v finished importing block h.
    let mut imported: Vec<Vec<Option<u64>>> = vec![vec![Some(0); n]];
    // Out-of-order arrivals waiting for their parent: (v, h) -> arrival time.
    let mut waiting: HashMap<(usize, u64), u64> = HashMap::new();
    // vote_times[h][v]: when validator v had each vote for block h verified.
    let mut vote_times: Vec<Vec<Vec<u64>>> = vec![vec![Vec::new(); n]];
    let mut streams: HashMap<(usize, u64), Stream> = HashMap::new();
    let mut included = vec![false; txs.len()];
    let mut first_pending = 0usize;

    let (first, delay) = builder(1);
    q.push(delay, Event::Build { h: 1, v: first });
    if !cfg.is_honest(in_turn(1)) && cfg.is_up(in_turn(1)) {
        q.push(interval, Event::BadBlock(1));
    }

    while let Some((now, ev)) = net.next(&mut q) {
        if now > end {
            break;
        }
        match ev {
            Event::Build { h, v } => {
                if blocks.len() as u64 != h {
                    continue;
                }
                let (_, delay) = builder(h);
                let target = blocks[h as usize - 1].seal + interval;
                let not_before = target + delay;
                let cutoff = now.max(not_before.saturating_sub(cfg.miner_reserve_ms * 1000));
                let chosen = select_txs(env, &included, first_pending, v, cutoff);
                for &i in &chosen {
                    included[i] = true;
                }
                while first_pending < txs.len() && included[first_pending] {
                    first_pending += 1;
                }
                engine.submit(chosen.iter().map(|&i| &txs[i]));
                let exec_cost = env.exec_model.cost_of(chosen.iter().map(|&i| &txs[i]));
                // The miner executes transactions as they arrive, so building overlaps with the
                // wait for the block time; it can only finish late if execution takes longer
                // than the time left.
                let built = cpu.exec(v, now, exec_cost);
                let seal = cpu.sig(v, built.max(not_before), env.crypto.ecdsa_sign);
                let chunks = match cfg.stream_chunk_ms {
                    Some(ms) => {
                        let plan = StreamPlan { now, block_time: not_before, cutoff, seal };
                        plan.chunks(env, &chosen, v, exec_cost, ms * 1000)
                    }
                    None => Vec::new(),
                };
                let streamed: usize = chunks.iter().map(|c| c.txs).sum();
                let tail = &chosen[streamed..];
                let bytes = HEADER_BYTES +
                    match cfg.compact_tx_bytes {
                        Some(per_tx) => per_tx * tail.len() as u64,
                        None => tail.iter().map(|&i| txs[i].size()).sum::<u64>(),
                    };
                for (seq, c) in chunks.iter().enumerate() {
                    q.push(c.send_at, Event::SendChunk { h, seq });
                }
                let (first_hop, relay) = relay_tree(env, v);
                blocks.push(Block {
                    proposer: v,
                    txs: chosen,
                    target,
                    seal,
                    exec_cost,
                    bytes,
                    out_of_turn: delay > 0,
                    first_hop,
                    relay,
                    chunks,
                    ..Default::default()
                });
                imported.push(vec![None; n]);
                vote_times.push(vec![Vec::new(); n]);
                q.push(seal, Event::Seal(h));
            }
            Event::Seal(h) => {
                let p = blocks[h as usize].proposer;
                let (mut justified, mut finalized) =
                    (blocks[h as usize - 1].justified, blocks[h as usize - 1].finalized);
                let has_quorum = |t: u64| {
                    vote_times[t as usize][p].iter().filter(|&&at| at <= now).count() >=
                        cfg.quorum()
                };
                let window = h.saturating_sub(cfg.attest_depth).max(justified + 1)..h;
                let targets: Vec<u64> = if cfg.multi_attest {
                    window.filter(|&t| has_quorum(t)).collect()
                } else {
                    window.rev().find(|&t| has_quorum(t)).into_iter().collect()
                };
                for &t in &targets {
                    if t == justified + 1 {
                        finalized = justified;
                    }
                    justified = t;
                }
                let b = &mut blocks[h as usize];
                b.attestations = targets.len() as u64;
                b.attest_target = targets.last().copied();
                (b.justified, b.finalized) = (justified, finalized);
                let block = BlockRef::Canonical(h);
                let b = &blocks[h as usize];
                net.send(env.topo, now, p, &b.first_hop, b.bytes, &mut q, |v| Event::Arrive {
                    v,
                    block,
                });
                imported[h as usize][p] = Some(now);
                q.push(now, Event::Imported { v: p, h });
                // A wrongly executing in-turn validator for the next height produces its own
                // block on time, which honest validators will execute and reject.
                let next = in_turn(h + 1);
                if cfg.is_up(next) && !cfg.is_honest(next) {
                    q.push(now + interval, Event::BadBlock(h + 1));
                }
            }
            Event::BadBlock(h) => {
                let p = in_turn(h);
                let chosen = select_txs(env, &included, first_pending, p, now);
                let exec_cost = env.exec_model.cost_of(chosen.iter().map(|&i| &txs[i]));
                let bytes = HEADER_BYTES + chosen.iter().map(|&i| txs[i].size()).sum::<u64>();
                let id = rejected.len();
                rejected.push(RejectedBlock { sent: now, exec_cost, bytes });
                net.broadcast(env.topo, now, p, bytes, &mut q, |v| Event::Arrive {
                    v,
                    block: BlockRef::Rejected(id),
                });
            }
            Event::Arrive { v, block } => {
                if !cfg.is_honest(v) {
                    continue;
                }
                match block {
                    BlockRef::Rejected(id) => {
                        // Imported, found to have a wrong state root, dropped without a vote.
                        cpu.exec(v, now, rejected[id].exec_cost);
                    }
                    BlockRef::Canonical(h) => {
                        let b = &blocks[h as usize];
                        if !b.relay[v].is_empty() {
                            let checked = cpu.sig(v, now, env.crypto.ecdsa_recover);
                            net.send(env.topo, checked, v, &b.relay[v], b.bytes, &mut q, |u| {
                                Event::Arrive { v: u, block }
                            });
                        }
                        if imported[h as usize - 1][v].is_some() {
                            let stream = streams.get_mut(&(v, h));
                            import(env, &mut cpu, &mut q, &blocks, stream, v, h, now);
                        } else {
                            waiting.insert((v, h), now);
                        }
                    }
                }
            }
            Event::Imported { v, h } => {
                imported[h as usize][v] = Some(now);
                let mut stream = streams.get_mut(&(v, h + 1));
                if let Some(s) = stream.as_deref_mut() {
                    s.advance(env, &mut cpu, &blocks[h as usize + 1], v, now);
                }
                if let Some(arrived) = waiting.remove(&(v, h + 1)) {
                    import(env, &mut cpu, &mut q, &blocks, stream, v, h + 1, arrived.max(now));
                }
                let signed = cpu.sig(v, now, env.crypto.bls_sign);
                vote_times[h as usize][v].push(signed);
                net.broadcast(env.topo, signed, v, VOTE_BYTES, &mut q, |u| Event::Vote { v: u, h });
                // The next builder can start once it has the parent.
                let (next, delay) = builder(h + 1);
                if v == next && blocks.len() as u64 == h + 1 {
                    let not_before = blocks[h as usize].seal + interval + delay;
                    let start = if delay > 0 { now.max(not_before) } else { now };
                    q.push(start, Event::Build { h: h + 1, v });
                }
            }
            Event::SendChunk { h, seq } => {
                let b = &blocks[h as usize];
                let bytes = b.chunks[seq].bytes;
                net.send(env.topo, now, b.proposer, &b.first_hop, bytes, &mut q, |v| {
                    Event::Chunk { v, h, seq }
                });
            }
            Event::Chunk { v, h, seq } => {
                if !cfg.is_honest(v) {
                    continue;
                }
                let b = &blocks[h as usize];
                let checked = cpu.sig(v, now, env.crypto.ecdsa_recover);
                if !b.relay[v].is_empty() {
                    let bytes = b.chunks[seq].bytes;
                    net.send(env.topo, checked, v, &b.relay[v], bytes, &mut q, |u| Event::Chunk {
                        v: u,
                        h,
                        seq,
                    });
                }
                let s = streams.entry((v, h)).or_default();
                if s.closed {
                    continue;
                }
                s.ready.insert(seq, checked);
                if imported[h as usize - 1][v].is_some() {
                    s.advance(env, &mut cpu, b, v, now);
                }
            }
            Event::Vote { v, h } => {
                if cfg.is_honest(v) {
                    let verified = cpu.sig(v, now, env.crypto.bls_verify);
                    vote_times[h as usize][v].push(verified);
                }
            }
        }
    }

    // Finality through attestations in headers: block b is final once the median validator has
    // imported the first header whose finalized block is at least b.
    let last = blocks.len() as u64 - 1;
    let median_import = |h: u64| -> Option<u64> {
        let times: Vec<u64> = honest.iter().filter_map(|&v| imported.get(h as usize)?[v]).collect();
        if times.len() < honest.len() {
            return None;
        }
        median(times)
    };
    let mut final_at: Vec<Option<u64>> = vec![None; blocks.len()];
    let mut next = 1u64;
    for h in 1..=last {
        while next <= blocks[h as usize].finalized {
            final_at[next as usize] = median_import(h);
            next += 1;
        }
    }

    let mut times = TxTimes::new(txs.len());
    for h in 1..=last {
        let b = &blocks[h as usize];
        let executed = median_import(h);
        for &i in &b.txs {
            times.ordered[i] = Some(b.seal);
            times.executed[i] = executed;
            times.finalized[i] = final_at[h as usize];
        }
    }

    // Block-level figures, with endorsement (2/3 votes) counted off-chain at each validator.
    let quorum = cfg.quorum();
    let endorsed = |h: u64, v: usize| -> Option<u64> {
        let mut t = vote_times.get(h as usize)?[v].clone();
        if t.len() < quorum {
            return None;
        }
        t.sort_unstable();
        Some(t[quorum - 1])
    };
    let in_window: Vec<u64> = (1..=last)
        .filter(|&h| {
            let seal = blocks[h as usize].seal;
            seal >= window.0 && seal < window.1
        })
        .collect();
    let window_s = (window.1 - window.0) as f64 / 1e6;
    let mut gaps = Vec::new();
    let mut sigs = Vec::new();
    let mut irreversible = Vec::new();
    for &h in &in_window {
        let b = &blocks[h as usize];
        for &v in honest.iter().filter(|&&v| v != b.proposer) {
            if let Some(t) = imported[h as usize][v] {
                gaps.push(t - b.seal);
            }
        }
        if let Some(t) = median(honest.iter().filter_map(|&v| endorsed(h, v)).collect()) {
            sigs.push(t - b.seal);
        }
        let both: Vec<u64> =
            honest.iter().filter_map(|&v| Some(endorsed(h, v)?.max(endorsed(h + 1, v)?))).collect();
        if both.len() == honest.len() {
            irreversible.push(median(both).expect("non-empty") - b.seal);
        }
    }
    let sent_in_window: Vec<u64> = in_window
        .iter()
        .map(|&h| blocks[h as usize].bytes)
        .chain(rejected.iter().filter(|r| r.sent >= window.0 && r.sent < window.1).map(|r| r.bytes))
        .collect();
    let block = BlockStats {
        blocks_per_s: in_window.len() as f64 / window_s,
        tps: in_window.iter().map(|&h| blocks[h as usize].txs.len()).sum::<usize>() as f64 /
            window_s,
        critical_bytes: sent_in_window.iter().sum::<u64>() as f64 /
            sent_in_window.len().max(1) as f64,
        state_gap_p50_ms: percentile_ms(&mut gaps, 0.5),
        quorum_sig_p50_ms: percentile_ms(&mut sigs, 0.5),
        order_qc_p50_ms: f64::NAN,
        irreversible_p50_ms: percentile_ms(&mut irreversible.clone(), 0.5),
        irreversible_p99_ms: percentile_ms(&mut irreversible, 0.99),
        faults: in_window.iter().filter(|&&h| blocks[h as usize].out_of_turn).count(),
        // One canonical chain by construction: honest validators import the same block at
        // every height.
        safe: true,
    };

    let block_times: Vec<u64> = blocks.iter().skip(1).map(|b| b.seal).collect();
    let window_blocks: Vec<&Block> = in_window.iter().map(|&h| &blocks[h as usize]).collect();
    let late = window_blocks.iter().filter(|b| b.seal > b.target + 1000).count();
    let attested_blocks = window_blocks.iter().filter(|b| b.attest_target.is_some()).count();
    let lagged = window_blocks
        .iter()
        .zip(&in_window)
        .filter(|(b, &h)| b.attest_target.is_some_and(|t| t + 1 < h))
        .count();
    let mean_exec =
        window_blocks.iter().map(|b| b.exec_cost).sum::<f64>() / window_blocks.len().max(1) as f64;
    let count = window_blocks.len();
    let streamed = window_blocks.iter().flat_map(|b| &b.chunks).map(|c| c.txs).sum::<usize>();
    let total_txs = window_blocks.iter().map(|b| b.txs.len()).sum::<usize>();
    let mut extra = vec![
        ("blocks sealed late (>1ms)".to_string(), format!("{late} / {count}")),
        ("blocks with an attestation".to_string(), format!("{attested_blocks} / {count}")),
        ("attestations below the parent".to_string(), format!("{lagged} / {count}")),
        ("mean block execution (ms)".to_string(), format!("{:.1}", mean_exec / 1000.0)),
        ("failed txs".to_string(), engine.finish().to_string()),
    ];
    if cfg.stream_chunk_ms.is_some() {
        let share = 100.0 * streamed as f64 / total_txs.max(1) as f64;
        extra.push(("txs streamed before the seal".to_string(), format!("{share:.1}%")));
    }
    Report::build(
        cfg,
        env.workload,
        RunOutput {
            protocol: "parlia",
            times: &times,
            block_times: &block_times,
            cpu: &cpu,
            bytes_sent: &net.bytes_sent,
            real: net.real_stats(),
            block,
            extra,
        },
    )
}

/// Transactions validator `v` has seen by `cutoff` and no canonical block holds yet, in arrival
/// order, up to the gas limit.
fn select_txs(
    env: &Env<'_>,
    included: &[bool],
    first_pending: usize,
    v: usize,
    cutoff: u64,
) -> Vec<usize> {
    let txs = &env.workload.txs;
    let mut gas = 0u64;
    let mut chosen = Vec::new();
    let mut i = first_pending;
    while i < txs.len() && txs[i].created_us <= cutoff {
        if !included[i] && env.workload.arrival_at(i, v, env.topo) <= cutoff {
            let g = txs[i].gas_limit();
            if gas + g > env.cfg.gas_limit {
                break;
            }
            gas += g;
            chosen.push(i);
        }
        i += 1;
    }
    chosen
}

/// Validator `v` imports block `h`: seal check, attestation check and sender recovery run in
/// parallel with execution, as in reth's payload processor. Transactions it already executed
/// from the block's stream are not executed again.
#[allow(clippy::too_many_arguments)]
fn import(
    env: &Env<'_>,
    cpu: &mut Cpu,
    q: &mut EventQueue<Event>,
    blocks: &[Block],
    stream: Option<&mut Stream>,
    v: usize,
    h: u64,
    at: u64,
) {
    let b = &blocks[h as usize];
    let mut checks = env.crypto.ecdsa_recover;
    checks += b.attestations as f64 * env.crypto.bls_agg_verify;
    let checked = cpu.sig(v, at, checks);
    let (done, cost, txs) = match stream {
        Some(s) => {
            s.closed = true;
            (s.done, s.cost, s.txs)
        }
        None => (0, 0.0, 0),
    };
    let left = (b.txs.len() - txs) as f64;
    let recovered = cpu.parallel(v, at, left * env.crypto.ecdsa_recover);
    let executed = cpu.exec(v, at.max(done), (b.exec_cost - cost).max(0.0));
    q.push(checked.max(recovered).max(executed), Event::Imported { v, h });
}

impl Stream {
    /// Executes, in order, every received chunk that directly follows the ones executed so far.
    /// The parent must already be imported.
    fn advance(&mut self, env: &Env<'_>, cpu: &mut Cpu, block: &Block, v: usize, at: u64) {
        if self.closed {
            return;
        }
        while let Some(ready) = self.ready.remove(&self.next) {
            let c = &block.chunks[self.next];
            let start = ready.max(at);
            let recovered = cpu.parallel(v, start, c.txs as f64 * env.crypto.ecdsa_recover);
            let executed = cpu.exec(v, start, c.cost);
            self.done = executed.max(recovered);
            self.cost += c.cost;
            self.txs += c.txs;
            self.next += 1;
        }
    }
}

/// When the proposer of a block may stream what: it starts building at `now`, seals at `seal`,
/// takes no transactions after `cutoff` and the block time is `block_time`.
#[derive(Debug)]
struct StreamPlan {
    now: u64,
    block_time: u64,
    cutoff: u64,
    seal: u64,
}

impl StreamPlan {
    /// Splits `chosen` into chunks of `chunk_us` sent ahead of the seal; transactions after the
    /// last chunk go with the seal.
    ///
    /// The first `bid_share` of the block is a builder bid, known only from the lock time on; the
    /// proposer already simulated it, so it streams at once. Local transactions follow it and
    /// have to be executed again on top of it before they can be streamed. Every other transaction
    /// joins the first chunk boundary after it reaches the proposer, and a chunk leaves once
    /// the proposer has executed it. The block's execution cost, less the per-block part, is
    /// split over the transactions by their cost.
    fn chunks(
        &self,
        env: &Env<'_>,
        chosen: &[usize],
        v: usize,
        exec_cost: f64,
        chunk_us: u64,
    ) -> Vec<Chunk> {
        let cfg = env.cfg;
        let txs = &env.workload.txs;
        let bids = (cfg.bid_share * chosen.len() as f64).round() as usize;
        let start = if bids > 0 {
            let lock_ms = cfg.lock_ms.unwrap_or(cfg.miner_reserve_ms);
            self.now.max(self.block_time.saturating_sub(lock_ms * 1000))
        } else {
            self.now
        };
        let weights: Vec<f64> = chosen.iter().map(|&i| env.exec_model.tx_cost(&txs[i])).collect();
        let total: f64 = weights.iter().sum();
        let body = (exec_cost - env.exec_model.block_us).max(0.0);
        let cost = |w: f64| if total > 0.0 { body * w / total } else { 0.0 };

        let mut chunks: Vec<Chunk> = Vec::new();
        let mut proposer_done = start;
        let mut j = 0;
        while j < chosen.len() {
            let avail = |j: usize| {
                if j < bids {
                    start
                } else {
                    env.workload.arrival_at(chosen[j], v, env.topo).max(start)
                }
            };
            let boundary = start + (avail(j) - start).div_ceil(chunk_us) * chunk_us;
            if boundary >= self.cutoff {
                break;
            }
            // The bid goes out as a chunk of its own, so it does not wait for the proposer to
            // re-execute local transactions on top of it.
            let limit = if j < bids { bids } else { chosen.len() };
            let mut end = j;
            let mut own = 0.0;
            let mut share = 0.0;
            while end < limit && avail(end) <= boundary {
                share += cost(weights[end]);
                if end >= bids {
                    own += cost(weights[end]);
                }
                end += 1;
            }
            proposer_done = proposer_done.max(avail(j)) + own.ceil() as u64;
            let send_at = boundary.max(proposer_done);
            if send_at >= self.seal {
                break;
            }
            let bytes =
                CHUNK_OVERHEAD_BYTES + chosen[j..end].iter().map(|&i| txs[i].size()).sum::<u64>();
            chunks.push(Chunk { send_at, txs: end - j, cost: share, bytes });
            j = end;
        }
        chunks
    }
}

/// Like geth's block propagation: the full block goes to sqrt(peers) validators, nearest first,
/// and each of them forwards it to its share of the rest once the header checks out. Validators
/// that reject the chain are served last. Returns the proposer's own recipients and, for each
/// validator, the peers it forwards to.
fn relay_tree(env: &Env<'_>, p: usize) -> (Vec<usize>, Vec<Vec<usize>>) {
    let cfg = env.cfg;
    let n = cfg.validators;
    let mut peers: Vec<usize> = (0..n).filter(|&v| v != p).collect();
    peers.sort_by_key(|&v| (!cfg.is_honest(v), env.topo.latency_us(p, v)));
    let fanout =
        if cfg.parlia_direct { peers.len() } else { (peers.len() as f64).sqrt().ceil() as usize };
    let (first, rest) = peers.split_at(fanout.min(peers.len()));
    let mut relay = vec![Vec::new(); n];
    for (i, &v) in rest.iter().enumerate() {
        relay[first[i % first.len()]].push(v);
    }
    (first.to_vec(), relay)
}

/// Deterministic pseudo-random rank used for the backoff order.
const fn splitmix(x: u64) -> u64 {
    let mut z = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}
