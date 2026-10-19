//! Today's flow: Parlia block production with fast-finality votes.
//!
//! The in-turn proposer builds and executes a full block, broadcasts it, and every validator
//! re-executes it before voting. The next proposer attests the parent if it has a quorum of
//! verified votes when it seals. A block is final once it and its child are both attested.
//! When the turn passes to another validator, that validator must import the parent first.
//! Validators never fail and out-of-turn blocks are not modelled.

use std::collections::HashMap;

use crate::{
    exec::ExecEngine,
    net::Network,
    report::{median, Report, RunOutput, TxTimes},
    sim::{Cpu, EventQueue},
    Env,
};

/// Header plus seal and attestation, in bytes.
const HEADER_BYTES: u64 = 700;
/// One fast-finality vote message, in bytes.
const VOTE_BYTES: u64 = 200;

#[derive(Debug)]
enum Event {
    /// The proposer of block `h` starts building it.
    Build(u64),
    /// The proposer finishes block `h` and broadcasts it.
    Seal(u64),
    /// Block `h` reaches validator `v`.
    Arrive { v: usize, h: u64 },
    /// Validator `v` finished importing block `h`.
    Imported { v: usize, h: u64 },
    /// A vote for block `h` reaches validator `v`.
    Vote { v: usize, h: u64 },
}

#[derive(Debug, Default)]
struct Block {
    proposer: usize,
    txs: Vec<usize>,
    target: u64,
    seal: u64,
    exec_cost: f64,
    attests_parent: bool,
    /// Peers each validator forwards the block to after receiving it.
    relay: Vec<Vec<usize>>,
}

/// Runs the Parlia simulation and returns its report.
pub fn run(env: &Env<'_>) -> Report {
    let cfg = env.cfg;
    let n = cfg.validators;
    let txs = &env.workload.txs;
    let interval = cfg.block_interval_ms * 1000;
    let end = ((cfg.duration_s + env.drain_s) * 1e6) as u64;
    let window = ((cfg.warmup_s * 1e6) as u64, (cfg.duration_s * 1e6) as u64);

    let mut q = EventQueue::default();
    let mut cpu = Cpu::new(n, cfg.sig_cores, window);
    let mut net = Network::new(n, cfg.bandwidth_mbps, window.0);
    let mut engine = ExecEngine::new(env.workload.accounts);

    let proposer = |h: u64| ((h / cfg.turn_length) as usize) % n;
    // blocks[h], with blocks[0] the genesis.
    let mut blocks: Vec<Block> = vec![Block::default()];
    // imported[h][v] is when validator v finished importing block h.
    let mut imported: Vec<Vec<Option<u64>>> = vec![vec![Some(0); n]];
    // Out-of-order arrivals waiting for their parent: (v, h) -> arrival time.
    let mut waiting: HashMap<(usize, u64), u64> = HashMap::new();
    // Verified vote times for block h at the proposer of h + 1.
    let mut votes: Vec<Vec<u64>> = vec![Vec::new()];
    let mut included = vec![false; txs.len()];
    let mut first_pending = 0usize;
    let mut failed_txs = 0usize;

    q.push(0, Event::Build(1));

    while let Some((now, ev)) = q.pop() {
        if now > end {
            break;
        }
        match ev {
            Event::Build(h) => {
                let p = proposer(h);
                let target = blocks[h as usize - 1].seal + interval;
                let cutoff = now.max(target.saturating_sub(cfg.miner_reserve_ms * 1000));
                let mut gas = 0u64;
                let mut chosen = Vec::new();
                let mut i = first_pending;
                while i < txs.len() && txs[i].created_us <= cutoff {
                    if !included[i] && env.workload.arrival_at(i, p, env.topo) <= cutoff {
                        let g = txs[i].gas_limit();
                        if gas + g > cfg.gas_limit {
                            break;
                        }
                        gas += g;
                        included[i] = true;
                        chosen.push(i);
                    }
                    i += 1;
                }
                while first_pending < txs.len() && included[first_pending] {
                    first_pending += 1;
                }
                let res = engine.execute(chosen.iter().map(|&i| &txs[i]));
                failed_txs += res.failed;
                let exec_cost = env.exec_model.block_cost(&res);
                // The miner executes transactions as they arrive, so building overlaps with the
                // wait for the block time; it can only finish late if execution takes longer
                // than the time left.
                let built = cpu.exec(p, now, exec_cost);
                let seal = cpu.sig(p, built.max(target), env.crypto.ecdsa_sign);
                blocks.push(Block {
                    proposer: p,
                    txs: chosen,
                    target,
                    seal,
                    exec_cost,
                    ..Default::default()
                });
                imported.push(vec![None; n]);
                votes.push(Vec::new());
                q.push(seal, Event::Seal(h));
            }
            Event::Seal(h) => {
                let p = blocks[h as usize].proposer;
                let parent_votes = votes[h as usize - 1].iter().filter(|&&t| t <= now).count();
                blocks[h as usize].attests_parent = h > 1 && parent_votes >= cfg.quorum();
                let size = HEADER_BYTES +
                    blocks[h as usize].txs.iter().map(|&i| txs[i].size()).sum::<u64>();
                // Like geth's block propagation: the full block goes to sqrt(peers) validators,
                // nearest first, and each of them forwards it to its share of the rest once the
                // header checks out.
                let mut peers: Vec<usize> = (0..n).filter(|&v| v != p).collect();
                peers.sort_by_key(|&v| env.topo.latency_us(p, v));
                let fanout = if cfg.parlia_direct {
                    peers.len()
                } else {
                    (peers.len() as f64).sqrt().ceil() as usize
                };
                let (first, rest) = peers.split_at(fanout.min(peers.len()));
                let mut relay = vec![Vec::new(); n];
                for (i, &v) in rest.iter().enumerate() {
                    relay[first[i % first.len()]].push(v);
                }
                blocks[h as usize].relay = relay;
                for (v, at) in net.send(env.topo, now, p, first, size) {
                    q.push(at, Event::Arrive { v, h });
                }
                imported[h as usize][p] = Some(now);
                q.push(now, Event::Imported { v: p, h });
            }
            Event::Arrive { v, h } => {
                let forward = std::mem::take(&mut blocks[h as usize].relay[v]);
                if !forward.is_empty() {
                    let checked = cpu.sig(v, now, env.crypto.ecdsa_recover);
                    let size = HEADER_BYTES +
                        blocks[h as usize].txs.iter().map(|&i| txs[i].size()).sum::<u64>();
                    for (u, at) in net.send(env.topo, checked, v, &forward, size) {
                        q.push(at, Event::Arrive { v: u, h });
                    }
                }
                if imported[h as usize - 1][v].is_some() {
                    import(env, &mut cpu, &mut q, &blocks, v, h, now);
                } else {
                    waiting.insert((v, h), now);
                }
            }
            Event::Imported { v, h } => {
                imported[h as usize][v] = Some(now);
                if let Some(arrived) = waiting.remove(&(v, h + 1)) {
                    import(env, &mut cpu, &mut q, &blocks, v, h + 1, arrived.max(now));
                }
                let signed = cpu.sig(v, now, env.crypto.bls_sign);
                let next = proposer(h + 1);
                if v == next {
                    votes[h as usize].push(signed);
                }
                for (u, at) in net.broadcast(env.topo, signed, v, VOTE_BYTES) {
                    q.push(at, Event::Vote { v: u, h });
                }
                // The next proposer can build once it has the parent.
                if v == next && blocks.len() as u64 == h + 1 {
                    q.push(now, Event::Build(h + 1));
                }
            }
            Event::Vote { v, h } => {
                let verified = cpu.sig(v, now, env.crypto.bls_verify);
                if v == proposer(h + 1) {
                    votes[h as usize].push(verified);
                }
            }
        }
    }

    // Finality: block h is final once some j >= h is attested by j + 1 and j + 1 by j + 2; the
    // median validator learns it when it imports j + 2.
    let last = blocks.len() as u64 - 1;
    let attested = |j: u64| j < last && blocks[j as usize + 1].attests_parent;
    let median_import = |h: u64| -> Option<u64> {
        let times: Vec<u64> = imported.get(h as usize)?.iter().flatten().copied().collect();
        if times.len() < n {
            return None;
        }
        median(times)
    };
    let mut final_at: Vec<Option<u64>> = vec![None; blocks.len()];
    let mut next_final: Option<u64> = None;
    for j in (1..=last).rev() {
        if attested(j) && attested(j + 1) {
            next_final = median_import(j + 2);
        }
        final_at[j as usize] = next_final;
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
    let block_times: Vec<u64> = blocks.iter().skip(1).map(|b| b.seal).collect();
    let in_window: Vec<&Block> =
        blocks.iter().skip(1).filter(|b| b.seal >= window.0 && b.seal < window.1).collect();
    let late = in_window.iter().filter(|b| b.seal > b.target + 1000).count();
    let attested_blocks = in_window.iter().filter(|b| b.attests_parent).count();
    let mean_exec =
        in_window.iter().map(|b| b.exec_cost).sum::<f64>() / in_window.len().max(1) as f64;
    let last = in_window.len();
    let extra = vec![
        ("blocks sealed late (>1ms)".to_string(), format!("{late} / {last}")),
        ("blocks with parent attestation".to_string(), format!("{attested_blocks} / {last}")),
        ("mean block execution (ms)".to_string(), format!("{:.1}", mean_exec / 1000.0)),
        ("failed txs".to_string(), failed_txs.to_string()),
    ];
    Report::build(
        cfg,
        env.workload,
        RunOutput {
            protocol: "parlia",
            times: &times,
            block_times: &block_times,
            cpu: &cpu,
            net: &net,
            extra,
        },
    )
}

/// Validator `v` imports block `h`: seal check, attestation check and sender recovery run in
/// parallel with execution, as in reth's payload processor.
fn import(
    env: &Env<'_>,
    cpu: &mut Cpu,
    q: &mut EventQueue<Event>,
    blocks: &[Block],
    v: usize,
    h: u64,
    at: u64,
) {
    let b = &blocks[h as usize];
    let mut checks = env.crypto.ecdsa_recover;
    if b.attests_parent {
        checks += env.crypto.bls_agg_verify;
    }
    let checked = cpu.sig(v, at, checks);
    let recovered = cpu.parallel(v, at, b.txs.len() as f64 * env.crypto.ecdsa_recover);
    let executed = cpu.exec(v, at, b.exec_cost);
    q.push(checked.max(recovered).max(executed), Event::Imported { v, h });
}
