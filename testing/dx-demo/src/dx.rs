//! The DX two-phase design (`bsc-decoupled-consensus-design.md` v2.1).
//!
//! - Dissemination: the owner of a sender cuts its transactions into batches every `T_batch`, signs
//!   them with Ed25519 and broadcasts them. Receivers verify, recover senders, store and return a
//!   BLS ack. `f + 1` acks form a PoA, which the author broadcasts. A down owner's transactions are
//!   batched by the next validator after `T_fallback`.
//! - Ordering: Jolteon-style rounds with `TurnLength` consecutive rounds per leader. A proposal
//!   carries PoA digests only. Votes go to the next leader, whose quorum forms the QC. A block is
//!   committed when its child, from the next round, is certified (two-chain rule). A round that
//!   gets no QC times out after `T_base * 2^c`; a Timeout carries its sender's highest QC, `2f + 1`
//!   timeouts form a TC, and a TC skips the rest of that leader's turn (design §12.1). With
//!   `--qc-broadcast` the leader that forms a QC also broadcasts it at once.
//! - Execution: validators execute verified proposals speculatively and committed blocks in order,
//!   sign an ExecVote for committed blocks and broadcast it. `2f + 1` matching ExecVotes form the
//!   ExecCert. A leader may only propose height `h` once it holds the ExecCert of height `h - k`
//!   (backpressure). ExecVotes from a wrongly executing validator never match.
//! - Builder (`--dx-builder-share`, `dx-mev-scheme-a-detailed.md`): a share of the transactions
//!   goes to a builder co-located with the leader instead of the batch layer, and is placed at the
//!   head of the block. Either the proposal carries it inline, or (`--dx-builder-chunk-ms`) the
//!   leader locks a chunk every interval while its round is open and broadcasts the body at once;
//!   the proposal then only references the chunks, votes wait for the bodies, and validators
//!   execute chunks on top of the parent before the proposal arrives.

use std::collections::HashMap;

use crate::{
    exec::BackgroundExec,
    net::Network,
    realnet::Cluster,
    report::{median, percentile_ms, BlockStats, Report, RunOutput, TxTimes},
    sim::{Cpu, EventQueue},
    workload::Tx,
    Env,
};

/// Batch header (author, sequence, expiry, digest) plus Ed25519 signature, in bytes.
const BATCH_OVERHEAD: u64 = 160;
/// BatchAck: digest plus BLS signature.
const ACK_BYTES: u64 = 140;
/// PoA: digest, author, gas, signer bitmap, aggregate BLS signature.
const POA_BYTES: u64 = 170;
/// Proposal header with parent QC and the ExecCert of height `h - k`.
const PROPOSAL_HEADER_BYTES: u64 = 900;
/// TC carried by the first proposal after a timeout.
const TC_BYTES: u64 = 250;
/// QC on its own (broadcast, or carried in a Timeout): round, block hash, signer bitmap, aggregate
/// BLS signature.
const QC_BYTES: u64 = 200;
/// OrderVote, Timeout and ExecVote.
const VOTE_BYTES: u64 = 200;
/// Gas declared per batch at most (`G_batch`).
const G_BATCH: u64 = 10_000_000;
/// Speculative execution depth past the committed head (`D_spec`).
const D_SPEC: u64 = 8;
/// Retry interval while a block waits for a batch body.
const BODY_POLL_US: u64 = 1000;
/// Cap on the timeout doubling exponent.
const MAX_BACKOFF_EXP: u32 = 6;
/// Builder chunk framing: the leader's `ChunkLock` (round, index, prefix hash, digest, builder,
/// value, BLS signature).
const CHUNK_LOCK_BYTES: u64 = 250;
/// `ChunkRef` in a proposal: the lock without an ack aggregate.
const CHUNK_REF_BYTES: u64 = 250;

#[derive(Debug)]
enum Event {
    /// Validator `v` cuts its pending transactions into batches.
    BatchTick(usize),
    /// Batch `b` reaches validator `v`.
    BatchArrive { v: usize, b: usize },
    /// An ack for batch `b` reaches its author.
    Ack(usize),
    /// The PoA of batch `b` reaches validator `v`.
    Poa { v: usize, b: usize },
    /// The leader of round `r` proposes, if it is still in that round.
    Propose(u64),
    /// Proposal `blk` reaches validator `v`.
    Proposal { v: usize, blk: usize },
    /// The QC of round `r`, broadcast by the leader that formed it, reaches validator `v`.
    Qc { v: usize, r: u64 },
    /// An OrderVote for round `r` reaches validator `to`.
    Vote { to: usize, r: u64 },
    /// Validator `v`'s timer for round `r` fires.
    Timer { v: usize, r: u64 },
    /// A Timeout for round `r`, carrying the sender's highest QC (block `high_qc`), reaches
    /// validator `v`.
    Timeout { v: usize, r: u64, high_qc: usize },
    /// Validator `v` may start executing block `blk`.
    ExecReady { v: usize, blk: usize },
    /// Validator `v` finished executing block `blk`.
    Executed { v: usize, blk: usize },
    /// An ExecVote for `blk` from `from` reaches validator `v`.
    ExecVote { v: usize, from: usize, blk: usize },
    /// The builder's leader locks the next chunk.
    ChunkTick,
    /// Builder chunk `c` reaches validator `v`.
    ChunkArrive { v: usize, c: usize },
    /// Validator `v` retries voting for `blk` once it holds every chunk body.
    VoteReady { v: usize, blk: usize },
    /// Validator `v` may start executing builder chunk `c` ahead of its proposal.
    PreExecReady { v: usize, c: usize },
    /// Validator `v` finished executing builder chunk `c` ahead of its proposal.
    PreExecuted { v: usize, c: usize },
}

/// Builder transactions the leader of `round` locked together, at the head of its block.
#[derive(Debug)]
struct BuilderChunk {
    /// Block the chunk was built on.
    parent: usize,
    txs: Vec<usize>,
    gas: u64,
    /// Execution cost of the chunk's transactions, without the per-block cost.
    cost: f64,
    /// Carried inline in the proposal instead of being broadcast on its own.
    inline: bool,
    /// Body stored (lock checked, senders recovered) at each validator.
    stored_at: Vec<Option<u64>>,
    /// Executed on top of `parent` ahead of the proposal at each validator.
    pre_executed_at: Vec<Option<u64>>,
}

#[derive(Debug)]
struct Batch {
    author: usize,
    txs: Vec<usize>,
    gas: u64,
    acks: usize,
    /// Body arrival at each validator.
    body_at: Vec<Option<u64>>,
    /// Senders recovered and body stored at each validator.
    stored_at: Vec<Option<u64>>,
    /// PoA verified and cached at each validator.
    poa_at: Vec<Option<u64>>,
    /// Block holding the batch, while that block may still become canonical.
    included_in: Option<usize>,
}

#[derive(Debug)]
struct Round {
    leader: usize,
    /// Position of this round within the leader's turn.
    pos: u64,
    block: Option<usize>,
    propose_scheduled: bool,
    votes: usize,
    /// Whether the next round's leader is fixed.
    resolved: bool,
    via_tc: bool,
    timeouts: Vec<usize>,
    timed_out: Vec<bool>,
    tc_formed: Vec<bool>,
}

#[derive(Debug)]
struct Block {
    round: u64,
    leader: usize,
    parent: usize,
    height: u64,
    batches: Vec<usize>,
    /// Builder chunks at the head of the block.
    chunks: Vec<usize>,
    proposed_at: u64,
    bytes: u64,
    after_tc: bool,
    exec_cost: f64,
    /// Transactions carried inline in the proposal (fractional: a share of the block).
    inline_txs: f64,
    qc_at: Option<u64>,
    known_at: Vec<Option<u64>>,
    committed_at: Vec<Option<u64>>,
    executed_at: Vec<Option<u64>>,
    exec_voted: Vec<bool>,
    exec_votes: Vec<usize>,
    cert_at: Vec<Option<u64>>,
}

struct State<'a, 'e> {
    env: &'a Env<'e>,
    n: usize,
    q: EventQueue<Event>,
    cpu: Cpu,
    net: Network<Event>,
    engine: Option<BackgroundExec>,
    batches: Vec<Batch>,
    /// Batches with a PoA that no live block holds.
    open_poas: Vec<usize>,
    /// Pending transactions per batching validator, sorted by when it may batch them.
    owner_queue: Vec<Vec<(u64, usize)>>,
    owner_next: Vec<usize>,
    rounds: Vec<Round>,
    blocks: Vec<Block>,
    blocks_at_height: Vec<Vec<usize>>,
    /// Canonical chain by height, as first committed by any validator.
    chain: Vec<usize>,
    cur_round: Vec<u64>,
    last_voted: Vec<u64>,
    high_qc: Vec<usize>,
    consecutive_timeouts: Vec<u32>,
    committed_height: Vec<u64>,
    /// Blocks each validator has executed, by height, along its current branch.
    exec_chain: Vec<Vec<usize>>,
    exec_busy: Vec<bool>,
    engine_next: u64,
    tcs: Vec<u64>,
    safe: bool,
    chunks: Vec<BuilderChunk>,
    /// Builder transactions in creation order; `builder_next` is the first not yet created.
    builder_txs: Vec<usize>,
    builder_next: usize,
    /// Created builder transactions not in a live chunk.
    builder_pool: Vec<usize>,
    /// Builder chunks locked for each round, in order. A round's chunks may be locked before the
    /// round starts, while the previous proposal waits for its QC.
    round_chunks: HashMap<u64, Vec<usize>>,
    /// Round the builder last locked a chunk for.
    chunk_round: Option<u64>,
}

/// Runs the DX simulation and returns its report.
pub fn run(env: &Env<'_>) -> Report {
    let cfg = env.cfg;
    let n = cfg.validators;
    let window = ((cfg.warmup_s * 1e6) as u64, (cfg.duration_s * 1e6) as u64);
    let end = ((cfg.duration_s + env.drain_s) * 1e6) as u64;

    let mut owner_queue: Vec<Vec<(u64, usize)>> = vec![Vec::new(); n];
    let mut builder_txs = Vec::new();
    for (i, tx) in env.workload.txs.iter().enumerate() {
        if from_builder(tx, cfg.dx_builder_share) {
            builder_txs.push(i);
            continue;
        }
        let owner = owner_of(tx, n);
        if cfg.is_up(owner) {
            owner_queue[owner].push((env.workload.arrival_at(i, owner, env.topo), i));
        } else {
            let fallback = (1..n).map(|d| (owner + d) % n).find(|&v| cfg.is_up(v)).expect("up");
            let at = env.workload.arrival_at(i, fallback, env.topo) + cfg.fallback_ms * 1000;
            owner_queue[fallback].push((at, i));
        }
    }
    for q in &mut owner_queue {
        q.sort_unstable();
    }

    let mut s = State {
        env,
        n,
        q: EventQueue::default(),
        cpu: Cpu::new(n, cfg.sig_cores, window),
        net: Network::new(
            n,
            cfg.bandwidth_mbps,
            window.0,
            (cfg.jitter_ms * 1000.0) as u64,
            cfg.seed,
            env.realnet.map(|spec| Cluster::launch(spec, n)),
        ),
        engine: Some(BackgroundExec::new(env.workload.accounts)),
        batches: Vec::new(),
        open_poas: Vec::new(),
        owner_queue,
        owner_next: vec![0; n],
        rounds: Vec::new(),
        blocks: Vec::new(),
        blocks_at_height: vec![vec![0]],
        chain: vec![0],
        cur_round: vec![0; n],
        last_voted: vec![0; n],
        high_qc: vec![0; n],
        consecutive_timeouts: vec![0; n],
        committed_height: vec![0; n],
        exec_chain: vec![vec![0]; n],
        exec_busy: vec![false; n],
        engine_next: 1,
        tcs: Vec::new(),
        safe: true,
        chunks: Vec::new(),
        builder_txs,
        builder_next: 0,
        builder_pool: Vec::new(),
        round_chunks: HashMap::new(),
        chunk_round: None,
    };
    // Genesis: round 0 and block 0 are certified, committed, executed and have an ExecCert.
    let mut genesis_round = s.new_round(n - 1, cfg.turn_length - 1);
    genesis_round.resolved = true;
    s.rounds.push(genesis_round);
    s.rounds.push(s.new_round(0, 0));
    let mut genesis = s.new_block(0, 0, 0, 0, 0);
    genesis.qc_at = Some(0);
    genesis.known_at = vec![Some(0); n];
    genesis.committed_at = vec![Some(0); n];
    genesis.executed_at = vec![Some(0); n];
    genesis.cert_at = vec![Some(0); n];
    s.blocks.push(genesis);

    if let (Some(ms), true) = (cfg.dx_builder_chunk_ms, cfg.dx_builder_share > 0.0) {
        s.q.push(ms * 1000, Event::ChunkTick);
    }
    let t_batch = cfg.t_batch_ms * 1000;
    for v in (0..n).filter(|&v| cfg.is_up(v)) {
        s.q.push(t_batch * v as u64 / n as u64, Event::BatchTick(v));
        s.enter_round(v, 1, 0);
    }

    while let Some((now, ev)) = s.net.next(&mut s.q) {
        if now > end {
            break;
        }
        s.handle(now, ev);
    }
    s.finish()
}

/// Owner validator of a sender: a fixed hash of the sender modulo `n` (design §6.1).
fn owner_of(tx: &Tx, n: usize) -> usize {
    (mix(u64::from(tx.sender)) % n as u64) as usize
}

/// Whether a transaction reaches the builder instead of the batch layer. Decided per sender, so
/// a sender's nonces never split between the two paths.
fn from_builder(tx: &Tx, share: f64) -> bool {
    share > 0.0 && ((mix(u64::from(tx.sender) ^ 0xb11d) % 1_000_000) as f64) < share * 1e6
}

/// SplitMix64 finalizer, so hashed buckets are uniform.
const fn mix(x: u64) -> u64 {
    let mut z = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

impl State<'_, '_> {
    fn new_round(&self, leader: usize, pos: u64) -> Round {
        Round {
            leader,
            pos,
            block: None,
            propose_scheduled: false,
            votes: 0,
            resolved: false,
            via_tc: false,
            timeouts: vec![0; self.n],
            timed_out: vec![false; self.n],
            tc_formed: vec![false; self.n],
        }
    }

    fn new_block(&self, round: u64, leader: usize, parent: usize, height: u64, at: u64) -> Block {
        Block {
            round,
            leader,
            parent,
            height,
            batches: Vec::new(),
            chunks: Vec::new(),
            proposed_at: at,
            bytes: 0,
            after_tc: false,
            exec_cost: 0.0,
            inline_txs: 0.0,
            qc_at: None,
            known_at: vec![None; self.n],
            committed_at: vec![None; self.n],
            executed_at: vec![None; self.n],
            exec_voted: vec![false; self.n],
            exec_votes: vec![0; self.n],
            cert_at: vec![None; self.n],
        }
    }

    /// Leader and turn position of the round after `r`. A TC skips the rest of the turn.
    fn leader_after(&self, r: u64, via_tc: bool) -> (usize, u64) {
        let round = &self.rounds[r as usize];
        if via_tc || round.pos + 1 >= self.env.cfg.turn_length {
            ((round.leader + 1) % self.n, 0)
        } else {
            (round.leader, round.pos + 1)
        }
    }

    /// Fixes the leader of round `r + 1` from how round `r` ended.
    fn resolve(&mut self, r: u64, via_tc: bool, at: u64) {
        if self.rounds[r as usize].resolved {
            return;
        }
        let (leader, pos) = self.leader_after(r, via_tc);
        let round = &mut self.rounds[r as usize];
        round.resolved = true;
        round.via_tc = via_tc;
        if via_tc {
            self.tcs.push(at);
        }
        let next = self.new_round(leader, pos);
        self.rounds.push(next);
    }

    fn handle(&mut self, now: u64, ev: Event) {
        let crypto = self.env.crypto;
        let cfg = self.env.cfg;
        match ev {
            Event::BatchTick(v) => {
                self.cut_batches(now, v);
                self.q.push(now + cfg.t_batch_ms * 1000, Event::BatchTick(v));
            }
            Event::BatchArrive { v, b } => {
                if !cfg.is_up(v) {
                    return;
                }
                let batch = &mut self.batches[b];
                batch.body_at[v] = Some(now);
                let verified = self.cpu.sig(v, now, crypto.ed_verify_batched);
                let recovered =
                    self.cpu.parallel(v, verified, batch.txs.len() as f64 * crypto.ecdsa_recover);
                batch.stored_at[v] = Some(recovered);
                let acked = self.cpu.sig(v, recovered, crypto.bls_sign);
                let author = batch.author;
                self.net.send(self.env.topo, acked, v, &[author], ACK_BYTES, &mut self.q, |_| {
                    Event::Ack(b)
                });
            }
            Event::Ack(b) => self.on_ack(now, b),
            Event::Poa { v, b } => {
                if cfg.is_up(v) && self.batches[b].poa_at[v].is_none() {
                    let verified = self.cpu.sig(v, now, crypto.bls_agg_verify);
                    self.batches[b].poa_at[v] = Some(verified);
                }
            }
            Event::Propose(r) => self.propose(now, r),
            Event::Proposal { v, blk } => self.on_proposal(now, v, blk),
            Event::Vote { to, r } => {
                if cfg.is_up(to) {
                    let done = self.cpu.sig(to, now, crypto.bls_agg_add);
                    self.add_order_vote(to, r, done);
                }
            }
            Event::Qc { v, r } => {
                if cfg.is_up(v) {
                    let done = self.cpu.sig(v, now, crypto.bls_agg_verify);
                    let blk = self.rounds[r as usize].block.expect("a QC certifies a proposal");
                    self.on_qc(v, blk, done);
                    self.enter_round(v, r + 1, done);
                }
            }
            Event::Timer { v, r } => {
                if self.cur_round[v] == r && !self.rounds[r as usize].timed_out[v] {
                    self.local_timeout(v, r, now);
                }
            }
            Event::Timeout { v, r, high_qc } => {
                if cfg.is_up(v) {
                    // The signature plus the carried QC.
                    let done = self.cpu.sig(v, now, crypto.bls_agg_add + crypto.bls_agg_verify);
                    self.on_qc(v, high_qc, done);
                    self.on_timeout(v, r, done);
                }
            }
            Event::ExecReady { v, blk } => self.execute(now, v, blk),
            Event::Executed { v, blk } => {
                self.exec_busy[v] = false;
                let parent = self.blocks[blk].parent;
                if self.exec_chain[v].last() == Some(&parent) {
                    self.exec_chain[v].push(blk);
                    self.blocks[blk].executed_at[v] = Some(now);
                    if self.blocks[blk].committed_at[v].is_some() {
                        self.send_exec_vote(v, blk, now);
                    }
                }
                self.schedule_exec(now, v);
            }
            Event::ExecVote { v, from, blk } => {
                if cfg.is_up(v) {
                    let done = self.cpu.sig(v, now, crypto.bls_agg_add);
                    // A wrong state root never matches the honest ExecVotes.
                    if cfg.is_honest(from) {
                        self.add_exec_vote(v, blk, done);
                    }
                }
            }
            Event::ChunkTick => {
                if let Some((r, leader, parent)) = self.builder_round() {
                    self.lock_chunk(now, r, leader, parent, false);
                }
                let ms = cfg.dx_builder_chunk_ms.expect("ticks only run with chunks");
                self.q.push(now + ms * 1000, Event::ChunkTick);
            }
            Event::ChunkArrive { v, c } => {
                if !cfg.is_up(v) {
                    return;
                }
                let checked = self.cpu.sig(v, now, crypto.bls_verify);
                let txs = self.chunks[c].txs.len() as f64;
                let stored = self.cpu.parallel(v, checked, txs * crypto.ecdsa_recover);
                self.chunks[c].stored_at[v] = Some(stored.max(checked));
                self.schedule_exec(now, v);
            }
            Event::VoteReady { v, blk } => self.try_vote(v, blk, now),
            Event::PreExecReady { v, c } => {
                if self.exec_chain[v].last() == Some(&self.chunks[c].parent) {
                    let done = self.cpu.exec(v, now, self.chunks[c].cost);
                    self.q.push(done, Event::PreExecuted { v, c });
                } else {
                    self.exec_busy[v] = false;
                    self.schedule_exec(now, v);
                }
            }
            Event::PreExecuted { v, c } => {
                self.exec_busy[v] = false;
                if self.exec_chain[v].last() == Some(&self.chunks[c].parent) {
                    self.chunks[c].pre_executed_at[v] = Some(now);
                }
                self.schedule_exec(now, v);
            }
        }
    }

    /// Round the builder fills, with its leader and the block it builds on: the newest round while
    /// its leader is in it and has not proposed, or, once it has, the round after it, built on that
    /// proposal before it is certified (as the builder bids on the parent proposal, §4.1).
    fn builder_round(&self) -> Option<(u64, usize, usize)> {
        let r = self.rounds.len() - 1;
        let round = &self.rounds[r];
        let (r, leader, parent) = match round.block {
            None if self.cur_round[round.leader] == r as u64 => {
                (r as u64, round.leader, self.high_qc[round.leader])
            }
            None => return None,
            Some(blk) => (r as u64 + 1, self.leader_after(r as u64, false).0, blk),
        };
        self.env.cfg.is_up(leader).then_some((r, leader, parent))
    }

    /// The leader of round `r` locks the builder transactions that have reached it into a chunk
    /// on top of `parent` and, unless `inline`, broadcasts the body. Returns the chunk, if there
    /// was anything to lock.
    fn lock_chunk(
        &mut self,
        now: u64,
        r: u64,
        leader: usize,
        parent: usize,
        inline: bool,
    ) -> Option<usize> {
        let cfg = self.env.cfg;
        let txs = &self.env.workload.txs;
        if self.chunk_round != Some(r) {
            // Chunks of a round that ended without a proposal are void; their transactions go
            // back to the builder.
            if let Some(old) = self.chunk_round {
                let proposed = self.rounds.get(old as usize).is_some_and(|x| x.block.is_some());
                if !proposed {
                    self.release_chunks(old, None);
                }
            }
            self.chunk_round = Some(r);
        }
        while self.builder_next < self.builder_txs.len() &&
            txs[self.builder_txs[self.builder_next]].created_us <= now
        {
            self.builder_pool.push(self.builder_txs[self.builder_next]);
            self.builder_next += 1;
        }
        let mut gas: u64 =
            self.round_chunks.get(&r).into_iter().flatten().map(|&c| self.chunks[c].gas).sum();
        let mut chosen = Vec::new();
        let workload = self.env.workload;
        let topo = self.env.topo;
        self.builder_pool.retain(|&i| {
            let tx = &txs[i];
            let arrived = workload.arrival_at(i, leader, topo) <= now;
            if arrived && gas + tx.gas_limit() <= cfg.gas_limit {
                gas += tx.gas_limit();
                chosen.push(i);
                false
            } else {
                true
            }
        });
        if chosen.is_empty() {
            return None;
        }
        chosen.sort_unstable();
        let c = self.chunks.len();
        let chunk_gas = chosen.iter().map(|&i| txs[i].gas_limit()).sum();
        let cost = chosen.iter().map(|&i| self.env.exec_model.tx_cost(&txs[i])).sum();
        let bytes = CHUNK_LOCK_BYTES + chosen.iter().map(|&i| txs[i].size()).sum::<u64>();
        let signed = self.cpu.sig(leader, now, self.env.crypto.bls_sign);
        let mut chunk = BuilderChunk {
            parent,
            txs: chosen,
            gas: chunk_gas,
            cost,
            inline,
            stored_at: vec![None; self.n],
            pre_executed_at: vec![None; self.n],
        };
        chunk.stored_at[leader] = Some(signed);
        self.chunks.push(chunk);
        self.round_chunks.entry(r).or_default().push(c);
        if !inline {
            self.net.broadcast(self.env.topo, signed, leader, bytes, &mut self.q, |v| {
                Event::ChunkArrive { v, c }
            });
        }
        Some(c)
    }

    /// Returns to the builder the transactions of round `r`'s chunks not built on `keep_parent`,
    /// and drops those chunks from the round.
    fn release_chunks(&mut self, r: u64, keep_parent: Option<usize>) {
        let Some(list) = self.round_chunks.remove(&r) else { return };
        let (kept, void): (Vec<usize>, Vec<usize>) =
            list.into_iter().partition(|&c| Some(self.chunks[c].parent) == keep_parent);
        for c in void {
            self.builder_pool.extend_from_slice(&self.chunks[c].txs);
        }
        // Keep the pool in creation order, so a sender's nonces are locked in order.
        self.builder_pool.sort_unstable();
        if !kept.is_empty() {
            self.round_chunks.insert(r, kept);
        }
    }

    fn cut_batches(&mut self, now: u64, v: usize) {
        let cfg = self.env.cfg;
        let txs = &self.env.workload.txs;
        loop {
            let queue = &self.owner_queue[v];
            let mut idx = self.owner_next[v];
            let mut chosen = Vec::new();
            let mut bytes = BATCH_OVERHEAD;
            let mut gas = 0;
            while idx < queue.len() && queue[idx].0 <= now {
                let tx = &txs[queue[idx].1];
                if !chosen.is_empty() &&
                    (bytes + tx.size() > cfg.s_batch as u64 || gas + tx.gas_limit() > G_BATCH)
                {
                    break;
                }
                bytes += tx.size();
                gas += tx.gas_limit();
                chosen.push(queue[idx].1);
                idx += 1;
            }
            if chosen.is_empty() {
                return;
            }
            self.owner_next[v] = idx;
            let b = self.batches.len();
            let signed = self.cpu.sig(v, now, self.env.crypto.ed_sign);
            let mut batch = Batch {
                author: v,
                txs: chosen,
                gas,
                acks: 0,
                body_at: vec![None; self.n],
                stored_at: vec![None; self.n],
                poa_at: vec![None; self.n],
                included_in: None,
            };
            batch.body_at[v] = Some(signed);
            batch.stored_at[v] = Some(signed);
            self.batches.push(batch);
            self.net.broadcast(self.env.topo, signed, v, bytes, &mut self.q, |u| {
                Event::BatchArrive { v: u, b }
            });
            // The author acks its own batch.
            let acked = self.cpu.sig(v, signed, self.env.crypto.bls_sign);
            self.q.push(acked, Event::Ack(b));
        }
    }

    fn on_ack(&mut self, now: u64, b: usize) {
        let crypto = self.env.crypto;
        let author = self.batches[b].author;
        self.batches[b].acks += 1;
        if self.batches[b].acks != self.env.cfg.availability() {
            return;
        }
        // Aggregate first, verify the aggregate once.
        let cost = self.env.cfg.availability() as f64 * crypto.bls_agg_add + crypto.bls_agg_verify;
        let formed = self.cpu.sig(author, now, cost);
        self.batches[b].poa_at[author] = Some(formed);
        self.open_poas.push(b);
        self.net.broadcast(self.env.topo, formed, author, POA_BYTES, &mut self.q, |v| Event::Poa {
            v,
            b,
        });
    }

    /// Validator `v` moves to round `r` and arms its timer.
    fn enter_round(&mut self, v: usize, r: u64, at: u64) {
        if r <= self.cur_round[v] {
            return;
        }
        self.cur_round[v] = r;
        let exp = self.consecutive_timeouts[v].min(MAX_BACKOFF_EXP);
        let timeout = self.env.cfg.round_timeout_ms * 1000 * (1u64 << exp);
        // The timer guards the leader, so it counts from the earliest moment the leader may
        // propose: `T_minInterval` after the parent proposal. Otherwise a `T_minInterval` above
        // `T_base` would time out every round.
        let earliest =
            self.blocks[self.high_qc[v]].proposed_at + self.env.cfg.min_interval_ms * 1000;
        self.q.push(at.max(earliest) + timeout, Event::Timer { v, r });
        if self.rounds[r as usize].leader == v {
            self.try_propose(r, at);
        }
    }

    /// Schedules the leader's proposal for round `r` once `T_minInterval` has passed since the
    /// parent proposal and the leader holds the ExecCert of height `h - k`.
    fn try_propose(&mut self, r: u64, now: u64) {
        let cfg = self.env.cfg;
        let round = &self.rounds[r as usize];
        let leader = round.leader;
        if !cfg.is_up(leader) || round.propose_scheduled || self.cur_round[leader] != r {
            return;
        }
        let parent = self.high_qc[leader];
        let height = self.blocks[parent].height + 1;
        let mut at = now.max(self.blocks[parent].proposed_at + cfg.min_interval_ms * 1000);
        if height > cfg.exec_lag {
            let mut ancestor = parent;
            while self.blocks[ancestor].height > height - cfg.exec_lag {
                ancestor = self.blocks[ancestor].parent;
            }
            match self.blocks[ancestor].cert_at[leader] {
                Some(cert) => at = at.max(cert),
                // Retried when the ExecCert forms at the leader.
                None => return,
            }
        }
        self.rounds[r as usize].propose_scheduled = true;
        self.q.push(at, Event::Propose(r));
    }

    fn propose(&mut self, now: u64, r: u64) {
        let cfg = self.env.cfg;
        let crypto = self.env.crypto;
        let leader = self.rounds[r as usize].leader;
        if self.cur_round[leader] != r {
            return;
        }
        let parent = self.high_qc[leader];
        let height = self.blocks[parent].height + 1;
        if cfg.dx_builder_share > 0.0 {
            // Chunks locked on another parent (the round before ended in a TC) are void. What
            // reached the builder since its last chunk is locked now, ahead of the proposal.
            self.release_chunks(r, Some(parent));
            self.lock_chunk(now, r, leader, parent, cfg.dx_builder_chunk_ms.is_none());
        }
        let chunks = self.round_chunks.get(&r).cloned().unwrap_or_default();
        let mut gas: u64 = chunks.iter().map(|&c| self.chunks[c].gas).sum();
        let mut chosen = Vec::new();
        let mut remaining = Vec::with_capacity(self.open_poas.len());
        for &b in &self.open_poas {
            let batch = &self.batches[b];
            let known = batch.poa_at[leader].is_some_and(|t| t <= now);
            if known && batch.included_in.is_none() && gas + batch.gas <= cfg.gas_limit {
                gas += batch.gas;
                chosen.push(b);
            } else if batch.included_in.is_none() {
                remaining.push(b);
            }
        }
        self.open_poas = remaining;
        let id = self.blocks.len();
        for &b in &chosen {
            self.batches[b].included_in = Some(id);
        }
        let after_tc = self.rounds[r as usize - 1].via_tc;
        let mut block = self.new_block(r, leader, parent, height, now);
        let workload_txs = &self.env.workload.txs;
        let chunk_txs: Vec<usize> =
            chunks.iter().flat_map(|&c| self.chunks[c].txs.iter().copied()).collect();
        let txs =
            || chosen.iter().flat_map(|&b| self.batches[b].txs.iter().map(|&i| &workload_txs[i]));
        block.exec_cost =
            self.env.exec_model.cost_of(chunk_txs.iter().map(|&i| &workload_txs[i]).chain(txs()));
        let inline_bytes =
            (txs().map(|t| t.size()).sum::<u64>() as f64 * cfg.dx_inline_share) as u64;
        block.inline_txs = txs().count() as f64 * cfg.dx_inline_share;
        let mut chunk_bytes = 0;
        for &c in &chunks {
            let chunk = &self.chunks[c];
            if chunk.inline {
                chunk_bytes += chunk.txs.iter().map(|&i| workload_txs[i].size()).sum::<u64>();
                block.inline_txs += chunk.txs.len() as f64;
            } else {
                chunk_bytes += CHUNK_REF_BYTES;
            }
        }
        block.bytes = PROPOSAL_HEADER_BYTES +
            chosen.len() as u64 * POA_BYTES +
            inline_bytes +
            chunk_bytes +
            if after_tc { TC_BYTES } else { 0 };
        block.chunks = chunks;
        block.after_tc = after_tc;
        block.batches = chosen;
        let bytes = block.bytes;
        self.blocks.push(block);
        if self.blocks_at_height.len() <= height as usize {
            self.blocks_at_height.push(Vec::new());
        }
        self.blocks_at_height[height as usize].push(id);
        self.rounds[r as usize].block = Some(id);
        let signed = self.cpu.sig(leader, now, crypto.bls_sign);
        self.net.broadcast(self.env.topo, signed, leader, bytes, &mut self.q, |v| {
            Event::Proposal { v, blk: id }
        });
        self.blocks[id].known_at[leader] = Some(signed);
        self.vote(leader, r, signed);
        self.schedule_exec(signed, leader);
    }

    fn on_proposal(&mut self, now: u64, v: usize, blk: usize) {
        let crypto = self.env.crypto;
        let cfg = self.env.cfg;
        if !cfg.is_up(v) {
            return;
        }
        // Header signature, parent QC, the carried ExecCert and TC.
        let block = &self.blocks[blk];
        let mut cost = crypto.bls_verify + crypto.bls_agg_verify;
        if block.height > cfg.exec_lag {
            cost += crypto.bls_agg_verify;
        }
        if block.after_tc {
            cost += crypto.bls_agg_verify;
        }
        let checked = self.cpu.sig(v, now, cost);
        let checked =
            self.cpu.parallel(v, now, block.inline_txs * crypto.ecdsa_recover).max(checked);
        for i in 0..self.blocks[blk].chunks.len() {
            let c = self.blocks[blk].chunks[i];
            if self.chunks[c].inline && self.chunks[c].stored_at[v].is_none() {
                self.chunks[c].stored_at[v] = Some(checked);
            }
        }
        // PoAs not yet in the cache are verified once, in parallel.
        let unseen = block
            .batches
            .iter()
            .filter(|&&b| self.batches[b].poa_at[v].is_none_or(|t| t > now))
            .count();
        let verified =
            self.cpu.parallel(v, checked, unseen as f64 * crypto.bls_agg_verify).max(checked);
        for i in 0..self.blocks[blk].batches.len() {
            let b = self.blocks[blk].batches[i];
            if self.batches[b].poa_at[v].is_none_or(|t| t > verified) {
                self.batches[b].poa_at[v] = Some(verified);
            }
        }
        self.blocks[blk].known_at[v] = Some(verified);
        let parent = self.blocks[blk].parent;
        self.on_qc(v, parent, verified);
        let r = self.blocks[blk].round;
        if self.rounds[r as usize - 1].via_tc {
            self.enter_round(v, r, verified);
        }
        self.enter_round(v, r, verified);
        self.try_vote(v, blk, verified);
        self.schedule_exec(verified, v);
    }

    /// Validator `v` votes for `blk` from `at` on, once it holds the body of every builder chunk
    /// the proposal references.
    fn try_vote(&mut self, v: usize, blk: usize, at: u64) {
        let r = self.blocks[blk].round;
        if self.cur_round[v] != r || r <= self.last_voted[v] {
            return;
        }
        let mut ready = at;
        for &c in &self.blocks[blk].chunks {
            match self.chunks[c].stored_at[v] {
                Some(t) => ready = ready.max(t),
                None => {
                    self.q.push(at + BODY_POLL_US, Event::VoteReady { v, blk });
                    return;
                }
            }
        }
        self.vote(v, r, ready);
    }

    /// Validator `v` learns at `at` that block `blk` is certified.
    fn on_qc(&mut self, v: usize, blk: usize, at: u64) {
        if self.blocks[blk].round > self.blocks[self.high_qc[v]].round {
            self.high_qc[v] = blk;
            self.consecutive_timeouts[v] = 0;
        }
        // Two-chain rule: a certified block from the round right after its parent commits the
        // parent.
        let parent = self.blocks[blk].parent;
        if blk != 0 && self.blocks[parent].round + 1 == self.blocks[blk].round {
            self.commit(v, parent, at);
        }
    }

    fn vote(&mut self, v: usize, r: u64, ready: u64) {
        self.last_voted[v] = r;
        let signed = self.cpu.sig(v, ready, self.env.crypto.bls_sign);
        let to = if self.rounds[r as usize].resolved {
            self.rounds[r as usize + 1].leader
        } else {
            self.leader_after(r, false).0
        };
        if v == to {
            self.add_order_vote(to, r, signed);
        } else {
            self.net.send(self.env.topo, signed, v, &[to], VOTE_BYTES, &mut self.q, |_| {
                Event::Vote { to, r }
            });
        }
    }

    fn add_order_vote(&mut self, to: usize, r: u64, at: u64) {
        let quorum = self.env.cfg.quorum();
        let round = &mut self.rounds[r as usize];
        round.votes += 1;
        if round.votes != quorum {
            return;
        }
        let blk = round.block.expect("votes are for a proposal");
        let qc = self.cpu.sig(to, at, self.env.crypto.bls_agg_verify);
        self.blocks[blk].qc_at = Some(qc);
        self.on_qc(to, blk, qc);
        self.resolve(r, false, qc);
        if self.env.cfg.qc_broadcast {
            self.net
                .broadcast(self.env.topo, qc, to, QC_BYTES, &mut self.q, |v| Event::Qc { v, r });
        }
        self.enter_round(to, r + 1, qc);
    }

    fn local_timeout(&mut self, v: usize, r: u64, at: u64) {
        self.rounds[r as usize].timed_out[v] = true;
        self.last_voted[v] = self.last_voted[v].max(r);
        self.consecutive_timeouts[v] += 1;
        let signed = self.cpu.sig(v, at, self.env.crypto.bls_sign);
        self.net.broadcast(self.env.topo, signed, v, VOTE_BYTES + QC_BYTES, &mut self.q, |u| {
            Event::Timeout { v: u, r, high_qc: self.high_qc[v] }
        });
        self.on_timeout(v, r, signed);
    }

    fn on_timeout(&mut self, v: usize, r: u64, at: u64) {
        let cfg = self.env.cfg;
        let round = &mut self.rounds[r as usize];
        round.timeouts[v] += 1;
        let count = round.timeouts[v];
        // f + 1 timeouts mean at least one honest validator gave up on the round.
        if count == cfg.f() + 1 && self.cur_round[v] == r && !round.timed_out[v] {
            self.local_timeout(v, r, at);
        }
        let round = &mut self.rounds[r as usize];
        if count == cfg.quorum() && !round.tc_formed[v] {
            round.tc_formed[v] = true;
            let tc = self.cpu.sig(v, at, self.env.crypto.bls_agg_verify);
            self.resolve(r, true, tc);
            self.enter_round(v, r + 1, tc);
        }
    }

    /// Validator `v` learns at `at` that `blk` and its ancestors are committed.
    fn commit(&mut self, v: usize, blk: usize, at: u64) {
        let mut path = Vec::new();
        let mut cur = blk;
        while self.blocks[cur].committed_at[v].is_none() {
            path.push(cur);
            cur = self.blocks[cur].parent;
        }
        for &x in path.iter().rev() {
            self.blocks[x].committed_at[v] = Some(at);
            let height = self.blocks[x].height;
            self.committed_height[v] = self.committed_height[v].max(height);
            if self.chain.len() as u64 == height {
                self.on_canonical(x);
            } else if self.chain[height as usize] != x {
                self.safe = false;
            }
            // Roll back speculation that took another branch.
            let exec = &mut self.exec_chain[v];
            if exec.len() as u64 > height && exec[height as usize] != x {
                exec.truncate(height as usize);
            }
            if self.blocks[x].executed_at[v].is_some() &&
                self.exec_chain[v].get(height as usize) == Some(&x)
            {
                self.send_exec_vote(v, x, at);
            }
        }
        self.schedule_exec(at, v);
    }

    /// Block `x` is the first committed block at its height.
    fn on_canonical(&mut self, x: usize) {
        let height = self.blocks[x].height as usize;
        self.chain.push(x);
        // Siblings can no longer be committed; their batches go back to the pool.
        for i in 0..self.blocks_at_height[height].len() {
            let sibling = self.blocks_at_height[height][i];
            if sibling == x {
                continue;
            }
            for j in 0..self.blocks[sibling].batches.len() {
                let b = self.blocks[sibling].batches[j];
                if self.batches[b].included_in == Some(sibling) {
                    self.batches[b].included_in = None;
                    self.open_poas.push(b);
                }
            }
            for j in 0..self.blocks[sibling].chunks.len() {
                let c = self.blocks[sibling].chunks[j];
                self.builder_pool.extend_from_slice(&self.chunks[c].txs);
            }
            self.builder_pool.sort_unstable();
        }
        // Execute canonical blocks for real, in order, so gas and failures are real.
        while (self.engine_next as usize) < self.chain.len() {
            let blk = self.chain[self.engine_next as usize];
            let txs = &self.env.workload.txs;
            let block = &self.blocks[blk];
            let order: Vec<usize> = block
                .chunks
                .iter()
                .flat_map(|&c| self.chunks[c].txs.iter().copied())
                .chain(block.batches.iter().flat_map(|&b| self.batches[b].txs.iter().copied()))
                .collect();
            self.engine.as_ref().expect("running").submit(order.iter().map(|&i| &txs[i]));
            self.engine_next += 1;
        }
    }

    fn send_exec_vote(&mut self, v: usize, blk: usize, ready: u64) {
        if self.blocks[blk].exec_voted[v] {
            return;
        }
        self.blocks[blk].exec_voted[v] = true;
        let signed = self.cpu.sig(v, ready, self.env.crypto.bls_sign);
        if self.env.cfg.is_honest(v) {
            self.add_exec_vote(v, blk, signed);
        }
        self.net.broadcast(self.env.topo, signed, v, VOTE_BYTES, &mut self.q, |u| {
            Event::ExecVote { v: u, from: v, blk }
        });
    }

    /// Schedules execution of the next block on `v`'s branch: the committed block at the next
    /// height, or with speculation a verified proposal extending the branch, at most `D_spec`
    /// blocks past the committed head.
    fn schedule_exec(&mut self, now: u64, v: usize) {
        if self.exec_busy[v] {
            return;
        }
        let height = self.exec_chain[v].len() as u64;
        let tip = *self.exec_chain[v].last().expect("genesis");
        let next = if self.committed_height[v] >= height {
            Some(self.chain[height as usize])
        } else if self.env.cfg.speculative && height <= self.committed_height[v] + D_SPEC {
            self.blocks_at_height
                .get(height as usize)
                .into_iter()
                .flatten()
                .copied()
                .filter(|&b| self.blocks[b].parent == tip && self.blocks[b].known_at[v].is_some())
                .max_by_key(|&b| self.blocks[b].round)
        } else {
            None
        };
        let Some(blk) = next else {
            self.schedule_chunk_exec(now, v);
            return;
        };
        let block = &self.blocks[blk];
        let known = block.committed_at[v].or(block.known_at[v]).expect("known or committed");
        let mut ready = known.max(now);
        for &b in &block.batches {
            // A missing body is still in flight (it was broadcast to everyone): poll for it.
            ready = ready.max(self.batches[b].body_at[v].unwrap_or(now + BODY_POLL_US));
        }
        for &c in &block.chunks {
            ready = ready.max(self.chunks[c].stored_at[v].unwrap_or(now + BODY_POLL_US));
        }
        self.exec_busy[v] = true;
        self.q.push(ready, Event::ExecReady { v, blk });
    }

    /// With nothing else to execute, `v` executes the next builder chunk of the open round on top
    /// of its parent, before the proposal arrives.
    fn schedule_chunk_exec(&mut self, now: u64, v: usize) {
        let Some(r) = self.chunk_round else { return };
        if !self.env.cfg.speculative {
            return;
        }
        let block = self.rounds.get(r as usize).and_then(|x| x.block);
        if block.is_some_and(|b| self.blocks[b].known_at[v].is_some()) {
            return;
        }
        let tip = *self.exec_chain[v].last().expect("genesis");
        let Some(list) = self.round_chunks.get(&r) else { return };
        let next = list.iter().copied().find(|&c| {
            let chunk = &self.chunks[c];
            chunk.parent == tip && !chunk.inline && chunk.pre_executed_at[v].is_none()
        });
        let Some(c) = next else { return };
        let Some(stored) = self.chunks[c].stored_at[v] else { return };
        self.exec_busy[v] = true;
        self.q.push(stored.max(now), Event::PreExecReady { v, c });
    }

    fn execute(&mut self, now: u64, v: usize, blk: usize) {
        if self.exec_chain[v].last() != Some(&self.blocks[blk].parent) {
            self.exec_busy[v] = false;
            self.schedule_exec(now, v);
            return;
        }
        let mut ready = now;
        for &b in &self.blocks[blk].batches {
            match self.batches[b].stored_at[v] {
                Some(t) => ready = ready.max(t),
                None => {
                    self.q.push(now + BODY_POLL_US, Event::ExecReady { v, blk });
                    return;
                }
            }
        }
        let parent = self.blocks[blk].parent;
        let mut cost = self.blocks[blk].exec_cost;
        for &c in &self.blocks[blk].chunks {
            match self.chunks[c].stored_at[v] {
                Some(t) => ready = ready.max(t),
                None => {
                    self.q.push(now + BODY_POLL_US, Event::ExecReady { v, blk });
                    return;
                }
            }
            let chunk = &self.chunks[c];
            if chunk.parent == parent && chunk.pre_executed_at[v].is_some() {
                cost -= chunk.cost;
            }
        }
        let done = self.cpu.exec(v, ready, cost.max(0.0));
        self.q.push(done, Event::Executed { v, blk });
    }

    fn add_exec_vote(&mut self, v: usize, blk: usize, at: u64) {
        let block = &mut self.blocks[blk];
        block.exec_votes[v] += 1;
        if block.exec_votes[v] != self.env.cfg.quorum() {
            return;
        }
        let cert = self.cpu.sig(v, at, self.env.crypto.bls_agg_verify);
        self.blocks[blk].cert_at[v] = Some(cert);
        // A leader held back by the missing ExecCert can propose now.
        let r = self.cur_round[v];
        if self.rounds[r as usize].leader == v {
            self.try_propose(r, cert);
        }
    }

    fn finish(mut self) -> Report {
        let failed_txs = self.engine.take().expect("running").finish();
        let cfg = self.env.cfg;
        let txs = &self.env.workload.txs;
        let honest: Vec<usize> = (0..self.n).filter(|&v| cfg.is_honest(v)).collect();
        let at_median = |col: &[Option<u64>]| -> Option<u64> {
            let t: Vec<u64> = honest.iter().filter_map(|&v| col[v]).collect();
            if t.len() < honest.len() {
                None
            } else {
                median(t)
            }
        };
        let mut times = TxTimes::new(txs.len());
        for &blk in self.chain.iter().skip(1) {
            let block = &self.blocks[blk];
            let ordered = at_median(&block.committed_at);
            let executed = at_median(&block.executed_at);
            let finalized = at_median(&block.cert_at);
            let chunk_txs = block.chunks.iter().flat_map(|&c| self.chunks[c].txs.iter());
            let batch_txs = block.batches.iter().flat_map(|&b| self.batches[b].txs.iter());
            for &i in chunk_txs.chain(batch_txs) {
                times.ordered[i] = ordered;
                times.executed[i] = executed;
                times.finalized[i] = finalized;
            }
        }

        let window = ((cfg.warmup_s * 1e6) as u64, (cfg.duration_s * 1e6) as u64);
        let window_s = (window.1 - window.0) as f64 / 1e6;
        let in_window = |t: u64| t >= window.0 && t < window.1;
        let canonical: Vec<&Block> = self
            .chain
            .iter()
            .skip(1)
            .map(|&b| &self.blocks[b])
            .filter(|b| in_window(b.proposed_at))
            .collect();
        let mut gaps = Vec::new();
        let mut qcs = Vec::new();
        let mut sigs = Vec::new();
        let mut irreversible = Vec::new();
        for b in &canonical {
            if let Some(lead) = b.executed_at[b.leader] {
                for &v in honest.iter().filter(|&&v| v != b.leader) {
                    if let Some(t) = b.executed_at[v] {
                        gaps.push(t.saturating_sub(lead));
                    }
                }
            }
            if let Some(qc) = b.qc_at {
                qcs.push(qc - b.proposed_at);
            }
            if let Some(cert) = at_median(&b.cert_at) {
                sigs.push(cert - b.proposed_at);
            }
            let both: Vec<u64> = honest
                .iter()
                .filter_map(|&v| Some(b.committed_at[v]?.max(b.cert_at[v]?)))
                .collect();
            if both.len() == honest.len() {
                irreversible.push(median(both).expect("non-empty") - b.proposed_at);
            }
        }
        let proposals: Vec<u64> = self
            .blocks
            .iter()
            .skip(1)
            .filter(|b| in_window(b.proposed_at))
            .map(|b| b.bytes)
            .collect();
        let block = BlockStats {
            blocks_per_s: canonical.len() as f64 / window_s,
            tps: canonical
                .iter()
                .map(|b| {
                    b.batches.iter().map(|&x| self.batches[x].txs.len()).sum::<usize>() +
                        b.chunks.iter().map(|&c| self.chunks[c].txs.len()).sum::<usize>()
                })
                .sum::<usize>() as f64 /
                window_s,
            critical_bytes: proposals.iter().sum::<u64>() as f64 / proposals.len().max(1) as f64,
            state_gap_p50_ms: percentile_ms(&mut gaps, 0.5),
            quorum_sig_p50_ms: percentile_ms(&mut sigs, 0.5),
            order_qc_p50_ms: percentile_ms(&mut qcs, 0.5),
            irreversible_p50_ms: percentile_ms(&mut irreversible.clone(), 0.5),
            irreversible_p99_ms: percentile_ms(&mut irreversible, 0.99),
            faults: self.tcs.iter().filter(|&&t| in_window(t)).count(),
            safe: self.safe,
        };

        let mut batch_wait = Vec::new();
        for batch in &self.batches {
            if let (Some(formed), Some(signed)) =
                (batch.poa_at[batch.author], batch.body_at[batch.author])
            {
                batch_wait.push(formed - signed);
            }
        }
        let block_times: Vec<u64> =
            self.chain.iter().skip(1).map(|&b| self.blocks[b].proposed_at).collect();
        let mean_poas = canonical.iter().map(|b| b.batches.len()).sum::<usize>() as f64 /
            canonical.len().max(1) as f64;
        let mean_exec =
            canonical.iter().map(|b| b.exec_cost).sum::<f64>() / canonical.len().max(1) as f64;
        let orphaned = self.blocks.len() - self.chain.len();
        let poa_latency =
            median(batch_wait).map_or("-".into(), |t| format!("{:.1}", t as f64 / 1000.0));
        let mut extra = vec![
            ("batches".to_string(), self.batches.len().to_string()),
            ("PoAs per proposal (mean)".to_string(), format!("{mean_poas:.1}")),
            ("batch signed -> PoA, median (ms)".to_string(), poa_latency),
            ("mean block execution (ms)".to_string(), format!("{:.1}", mean_exec / 1000.0)),
            ("blocks not committed".to_string(), orphaned.to_string()),
            ("failed txs".to_string(), failed_txs.to_string()),
        ];
        if cfg.dx_builder_share > 0.0 {
            let blocks = canonical.len().max(1) as f64;
            let chunks = canonical.iter().map(|b| b.chunks.len()).sum::<usize>() as f64;
            let pre: usize = canonical
                .iter()
                .flat_map(|b| b.chunks.iter())
                .map(|&c| {
                    let chunk = &self.chunks[c];
                    honest.iter().filter(|&&v| chunk.pre_executed_at[v].is_some()).count()
                })
                .sum();
            let pre_share = 100.0 * pre as f64 / (chunks * honest.len() as f64).max(1.0);
            extra.push((
                "builder chunks per block (mean)".to_string(),
                format!("{:.1}", chunks / blocks),
            ));
            extra.push((
                "builder chunks executed before the proposal".to_string(),
                format!("{pre_share:.1}%"),
            ));
        }
        Report::build(
            cfg,
            self.env.workload,
            RunOutput {
                protocol: "dx",
                times: &times,
                block_times: &block_times,
                cpu: &self.cpu,
                bytes_sent: &self.net.bytes_sent,
                real: self.net.real_stats(),
                block,
                extra,
            },
        )
    }
}
