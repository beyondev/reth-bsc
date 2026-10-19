//! The DX two-phase design (`bsc-decoupled-consensus-design.md` v2.1), without faults.
//!
//! - Dissemination: the owner of a sender cuts its transactions into batches every `T_batch`, signs
//!   them with Ed25519 and broadcasts them. Receivers verify, recover senders, store and return a
//!   BLS ack. `f + 1` acks form a PoA, which the author broadcasts.
//! - Ordering: Jolteon-style rounds with `TurnLength` consecutive rounds per leader. A proposal
//!   carries PoA digests only. Votes go to the next leader, whose quorum forms the QC. A block is
//!   committed when the QC of its child is known (two-chain rule).
//! - Execution: every validator executes committed blocks in order, signs an ExecVote and
//!   broadcasts it. `2f + 1` ExecVotes form the ExecCert. A leader may only propose round `r` once
//!   it holds the ExecCert of block `r - k` (backpressure).
//!
//! Timeouts, view changes and batch fetching are not exercised because no validator fails.

use crate::{
    exec::ExecEngine,
    net::Network,
    report::{median, Report, RunOutput, TxTimes},
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
/// Proposal header with parent QC and the ExecCert of block `r - k`.
const PROPOSAL_HEADER_BYTES: u64 = 900;
/// OrderVote and ExecVote.
const VOTE_BYTES: u64 = 200;
/// Gas declared per batch at most (`G_batch`).
const G_BATCH: u64 = 10_000_000;
/// Speculative execution depth past the committed head (`D_spec`).
const D_SPEC: u64 = 8;
/// Retry interval while a block waits for a batch body.
const BODY_POLL_US: u64 = 1000;

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
    /// The leader of round `r` proposes.
    Propose(u64),
    /// The proposal of round `r` reaches validator `v`.
    Proposal { v: usize, r: u64 },
    /// An OrderVote for round `r` reaches the next leader.
    Vote(u64),
    /// Validator `v` may start executing committed block `r`.
    ExecReady { v: usize, r: u64 },
    /// Validator `v` finished executing block `r`.
    Executed { v: usize, r: u64 },
    /// An ExecVote for block `r` reaches validator `v`.
    ExecVote { v: usize, r: u64 },
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
    included: bool,
}

#[derive(Debug, Default)]
struct Round {
    proposed_at: u64,
    batches: Vec<usize>,
    /// Aggregated OrderVotes at the next leader.
    votes: usize,
    /// When the next leader formed the QC.
    qc_at: Option<u64>,
    /// When each validator verified the proposal.
    known_at: Vec<Option<u64>>,
    /// When each validator committed this block.
    committed_at: Vec<Option<u64>>,
    executed_at: Vec<Option<u64>>,
    exec_votes: Vec<usize>,
    cert_at: Vec<Option<u64>>,
    exec_cost: Option<f64>,
}

struct State<'a, 'e> {
    env: &'a Env<'e>,
    n: usize,
    q: EventQueue<Event>,
    cpu: Cpu,
    net: Network,
    engine: ExecEngine,
    batches: Vec<Batch>,
    /// Batches with a PoA that no proposal has included yet.
    open_poas: Vec<usize>,
    /// Pending transactions per owner, sorted by arrival at the owner.
    owner_queue: Vec<Vec<(u64, usize)>>,
    owner_next: Vec<usize>,
    rounds: Vec<Round>,
    /// Highest round whose proposal has been scheduled.
    scheduled: u64,
    /// Next block to execute on the shared engine.
    engine_next: u64,
    /// Next block each validator has to execute.
    exec_next: Vec<u64>,
    /// Whether a validator has an execution scheduled or running.
    exec_busy: Vec<bool>,
    /// Highest committed block per validator.
    committed_upto: Vec<u64>,
    /// Highest round up to which a validator has verified every proposal.
    known_upto: Vec<u64>,
    failed_txs: usize,
}

/// Runs the DX simulation and returns its report.
pub fn run(env: &Env<'_>) -> Report {
    let cfg = env.cfg;
    let n = cfg.validators;
    let window = ((cfg.warmup_s * 1e6) as u64, (cfg.duration_s * 1e6) as u64);
    let end = ((cfg.duration_s + env.drain_s) * 1e6) as u64;

    let mut owner_queue: Vec<Vec<(u64, usize)>> = vec![Vec::new(); n];
    for (i, tx) in env.workload.txs.iter().enumerate() {
        let owner = owner_of(tx, n);
        owner_queue[owner].push((env.workload.arrival_at(i, owner, env.topo), i));
    }
    for q in &mut owner_queue {
        q.sort_unstable();
    }

    let mut s = State {
        env,
        n,
        q: EventQueue::default(),
        cpu: Cpu::new(n, cfg.sig_cores, window),
        net: Network::new(n, cfg.bandwidth_mbps, window.0),
        engine: ExecEngine::new(env.workload.accounts),
        batches: Vec::new(),
        open_poas: Vec::new(),
        owner_queue,
        owner_next: vec![0; n],
        rounds: vec![Round::default()],
        scheduled: 0,
        engine_next: 1,
        exec_next: vec![1; n],
        exec_busy: vec![false; n],
        committed_upto: vec![0; n],
        known_upto: vec![0; n],
        failed_txs: 0,
    };
    // Genesis: round 0 is certified, committed, executed and has an ExecCert everywhere.
    s.rounds[0] = s.new_round(0);
    s.rounds[0].qc_at = Some(0);
    s.rounds[0].known_at = vec![Some(0); n];
    s.rounds[0].committed_at = vec![Some(0); n];
    s.rounds[0].executed_at = vec![Some(0); n];
    s.rounds[0].cert_at = vec![Some(0); n];

    let t_batch = cfg.t_batch_ms * 1000;
    for v in 0..n {
        s.q.push(t_batch * v as u64 / n as u64, Event::BatchTick(v));
    }
    s.try_propose(1);

    while let Some((now, ev)) = s.q.pop() {
        if now > end {
            break;
        }
        s.handle(now, ev);
    }
    s.finish()
}

/// Owner validator of a sender: a fixed hash of the sender modulo `n` (design §6.1).
fn owner_of(tx: &Tx, n: usize) -> usize {
    // SplitMix64 finalizer, so owners are uniform for any `n`.
    let mut z = u64::from(tx.sender).wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    ((z ^ (z >> 31)) % n as u64) as usize
}

impl State<'_, '_> {
    fn new_round(&self, proposed_at: u64) -> Round {
        Round {
            proposed_at,
            known_at: vec![None; self.n],
            committed_at: vec![None; self.n],
            executed_at: vec![None; self.n],
            exec_votes: vec![0; self.n],
            cert_at: vec![None; self.n],
            ..Default::default()
        }
    }

    fn leader(&self, r: u64) -> usize {
        ((r / self.env.cfg.turn_length) as usize) % self.n
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
                let batch = &mut self.batches[b];
                batch.body_at[v] = Some(now);
                let verified = self.cpu.sig(v, now, crypto.ed_verify_batched);
                let recovered =
                    self.cpu.parallel(v, verified, batch.txs.len() as f64 * crypto.ecdsa_recover);
                batch.stored_at[v] = Some(recovered);
                let acked = self.cpu.sig(v, recovered, crypto.bls_sign);
                let author = batch.author;
                for (_, at) in self.net.send(self.env.topo, acked, v, &[author], ACK_BYTES) {
                    self.q.push(at, Event::Ack(b));
                }
            }
            Event::Ack(b) => self.on_ack(now, b),
            Event::Poa { v, b } => {
                if self.batches[b].poa_at[v].is_none() {
                    let verified = self.cpu.sig(v, now, crypto.bls_agg_verify);
                    self.batches[b].poa_at[v] = Some(verified);
                }
            }
            Event::Propose(r) => self.propose(now, r),
            Event::Proposal { v, r } => self.on_proposal(now, v, r),
            Event::Vote(r) => {
                let next = self.leader(r + 1);
                let done = self.cpu.sig(next, now, crypto.bls_agg_add);
                self.add_order_vote(next, r, done);
            }
            Event::ExecReady { v, r } => self.execute(now, v, r),
            Event::Executed { v, r } => {
                self.rounds[r as usize].executed_at[v] = Some(now);
                // A speculatively executed block is only voted on once it is committed.
                if self.committed_upto[v] >= r {
                    self.send_exec_vote(v, r, now);
                }
                self.exec_next[v] = r + 1;
                self.exec_busy[v] = false;
                self.schedule_exec(now, v);
            }
            Event::ExecVote { v, r } => {
                let done = self.cpu.sig(v, now, crypto.bls_agg_add);
                self.add_exec_vote(v, r, done);
            }
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
                included: false,
            };
            batch.body_at[v] = Some(signed);
            batch.stored_at[v] = Some(signed);
            self.batches.push(batch);
            for (u, at) in self.net.broadcast(self.env.topo, signed, v, bytes) {
                self.q.push(at, Event::BatchArrive { v: u, b });
            }
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
        for (v, at) in self.net.broadcast(self.env.topo, formed, author, POA_BYTES) {
            self.q.push(at, Event::Poa { v, b });
        }
    }

    /// Schedules the proposal of round `r` once its leader has the QC of `r - 1`, the ExecCert
    /// of `r - k`, and `T_minInterval` has passed since the parent proposal.
    fn try_propose(&mut self, r: u64) {
        if r != self.scheduled + 1 {
            return;
        }
        let cfg = self.env.cfg;
        let leader = self.leader(r);
        let Some(parent) = self.rounds.get(r as usize - 1) else { return };
        let Some(qc) = parent.qc_at else { return };
        let mut at = qc.max(parent.proposed_at + cfg.min_interval_ms * 1000);
        if r > cfg.exec_lag {
            let certified = (r - cfg.exec_lag) as usize;
            match self.rounds.get(certified).and_then(|round| round.cert_at[leader]) {
                Some(cert) => at = at.max(cert),
                None => return,
            }
        }
        self.scheduled = r;
        self.q.push(at, Event::Propose(r));
    }

    fn propose(&mut self, now: u64, r: u64) {
        let cfg = self.env.cfg;
        let crypto = self.env.crypto;
        let leader = self.leader(r);
        let mut gas = 0;
        let mut chosen = Vec::new();
        let mut remaining = Vec::with_capacity(self.open_poas.len());
        for &b in &self.open_poas {
            let batch = &self.batches[b];
            let known = batch.poa_at[leader].is_some_and(|t| t <= now);
            if known && gas + batch.gas <= cfg.gas_limit {
                gas += batch.gas;
                chosen.push(b);
            } else {
                remaining.push(b);
            }
        }
        self.open_poas = remaining;
        for &b in &chosen {
            self.batches[b].included = true;
        }
        let mut round = self.new_round(now);
        round.batches = chosen;
        let bytes = PROPOSAL_HEADER_BYTES + round.batches.len() as u64 * POA_BYTES;
        self.rounds.push(round);
        let signed = self.cpu.sig(leader, now, crypto.bls_sign);
        for (v, at) in self.net.broadcast(self.env.topo, signed, leader, bytes) {
            self.q.push(at, Event::Proposal { v, r });
        }
        self.record_proposal(leader, r, signed);
        self.vote(leader, r, signed);
    }

    fn on_proposal(&mut self, now: u64, v: usize, r: u64) {
        let crypto = self.env.crypto;
        let cfg = self.env.cfg;
        // Header signature, parent QC and the carried ExecCert.
        let mut cost = crypto.bls_verify + crypto.bls_agg_verify;
        if r > cfg.exec_lag {
            cost += crypto.bls_agg_verify;
        }
        let checked = self.cpu.sig(v, now, cost);
        // PoAs not yet in the cache are verified once, in parallel.
        let mut unseen = 0usize;
        for &b in &self.rounds[r as usize].batches {
            if self.batches[b].poa_at[v].is_none_or(|t| t > now) {
                unseen += 1;
            }
        }
        let verified =
            self.cpu.parallel(v, checked, unseen as f64 * crypto.bls_agg_verify).max(checked);
        for i in 0..self.rounds[r as usize].batches.len() {
            let b = self.rounds[r as usize].batches[i];
            if self.batches[b].poa_at[v].is_none_or(|t| t > verified) {
                self.batches[b].poa_at[v] = Some(verified);
            }
        }
        self.record_proposal(v, r, verified);
        // The proposal carries QC(r - 1), which commits r - 2.
        if r >= 2 {
            self.commit(v, r - 2, verified);
        }
        self.vote(v, r, verified);
    }

    fn vote(&mut self, v: usize, r: u64, ready: u64) {
        let signed = self.cpu.sig(v, ready, self.env.crypto.bls_sign);
        let next = self.leader(r + 1);
        if v == next {
            self.add_order_vote(next, r, signed);
        } else {
            for (_, at) in self.net.send(self.env.topo, signed, v, &[next], VOTE_BYTES) {
                self.q.push(at, Event::Vote(r));
            }
        }
    }

    fn add_order_vote(&mut self, next: usize, r: u64, at: u64) {
        let quorum = self.env.cfg.quorum();
        let round = &mut self.rounds[r as usize];
        round.votes += 1;
        if round.votes != quorum {
            return;
        }
        let qc = self.cpu.sig(next, at, self.env.crypto.bls_agg_verify);
        self.rounds[r as usize].qc_at = Some(qc);
        // QC(r) at the next leader commits r - 1.
        if r >= 1 {
            self.commit(next, r - 1, qc);
        }
        self.try_propose(r + 1);
    }

    /// Validator `v` learns at `at` that blocks up to `r` are committed.
    fn commit(&mut self, v: usize, r: u64, at: u64) {
        while self.committed_upto[v] < r {
            self.committed_upto[v] += 1;
            let c = self.committed_upto[v];
            self.rounds[c as usize].committed_at[v] = Some(at);
            if self.rounds[c as usize].executed_at[v].is_some() {
                self.send_exec_vote(v, c, at);
            }
        }
        self.schedule_exec(at, v);
    }

    fn send_exec_vote(&mut self, v: usize, r: u64, ready: u64) {
        let signed = self.cpu.sig(v, ready, self.env.crypto.bls_sign);
        self.add_exec_vote(v, r, signed);
        for (u, at) in self.net.broadcast(self.env.topo, signed, v, VOTE_BYTES) {
            self.q.push(at, Event::ExecVote { v: u, r });
        }
    }

    /// Validator `v` has verified the proposal of round `r` at `at`.
    fn record_proposal(&mut self, v: usize, r: u64, at: u64) {
        self.rounds[r as usize].known_at[v] = Some(at);
        while let Some(round) = self.rounds.get(self.known_upto[v] as usize + 1) {
            if round.known_at[v].is_none() {
                break;
            }
            self.known_upto[v] += 1;
        }
        self.schedule_exec(at, v);
    }

    /// Schedules execution of the next block once its batch bodies have arrived. Without
    /// speculation only committed blocks run; with it, any verified proposal up to `D_spec`
    /// blocks past the committed head.
    fn schedule_exec(&mut self, now: u64, v: usize) {
        let r = self.exec_next[v];
        let limit = if self.env.cfg.speculative {
            self.known_upto[v].min(self.committed_upto[v] + D_SPEC)
        } else {
            self.committed_upto[v]
        };
        if self.exec_busy[v] || r > limit {
            return;
        }
        let round = &self.rounds[r as usize];
        let known = round.committed_at[v].or(round.known_at[v]).expect("proposal verified");
        let mut ready = known.max(now);
        for &b in &round.batches {
            // A missing body is still in flight (it was broadcast to everyone): poll for it.
            ready = ready.max(self.batches[b].body_at[v].unwrap_or(now + BODY_POLL_US));
        }
        self.exec_busy[v] = true;
        self.q.push(ready, Event::ExecReady { v, r });
    }

    fn execute(&mut self, now: u64, v: usize, r: u64) {
        let mut ready = now;
        for &b in &self.rounds[r as usize].batches {
            match self.batches[b].stored_at[v] {
                Some(t) => ready = ready.max(t),
                None => {
                    self.q.push(now + BODY_POLL_US, Event::ExecReady { v, r });
                    return;
                }
            }
        }
        let cost = self.exec_cost(r);
        let done = self.cpu.exec(v, ready, cost);
        self.q.push(done, Event::Executed { v, r });
    }

    /// Executes blocks on the shared engine up to `r` and returns the cost of block `r`.
    fn exec_cost(&mut self, r: u64) -> f64 {
        while self.engine_next <= r {
            let x = self.engine_next as usize;
            let txs = &self.env.workload.txs;
            let order: Vec<usize> = self.rounds[x]
                .batches
                .iter()
                .flat_map(|&b| self.batches[b].txs.iter().copied())
                .collect();
            let res = self.engine.execute(order.iter().map(|&i| &txs[i]));
            self.failed_txs += res.failed;
            self.rounds[x].exec_cost = Some(self.env.exec_model.block_cost(&res));
            self.engine_next += 1;
        }
        self.rounds[r as usize].exec_cost.expect("executed above")
    }

    fn add_exec_vote(&mut self, v: usize, r: u64, at: u64) {
        let quorum = self.env.cfg.quorum();
        let round = &mut self.rounds[r as usize];
        round.exec_votes[v] += 1;
        if round.exec_votes[v] != quorum {
            return;
        }
        let cert = self.cpu.sig(v, at, self.env.crypto.bls_agg_verify);
        self.rounds[r as usize].cert_at[v] = Some(cert);
        let next = r + self.env.cfg.exec_lag;
        if self.leader(next) == v {
            self.try_propose(next);
        }
    }

    fn finish(self) -> Report {
        let cfg = self.env.cfg;
        let txs = &self.env.workload.txs;
        let mut times = TxTimes::new(txs.len());
        let all = |col: &[Option<u64>]| -> Option<u64> {
            let t: Vec<u64> = col.iter().flatten().copied().collect();
            if t.len() < self.n {
                None
            } else {
                median(t)
            }
        };
        let mut batch_wait = Vec::new();
        for round in self.rounds.iter().skip(1) {
            let ordered = all(&round.committed_at);
            let executed = all(&round.executed_at);
            let finalized = all(&round.cert_at);
            for &b in &round.batches {
                for &i in &self.batches[b].txs {
                    times.ordered[i] = ordered;
                    times.executed[i] = executed;
                    times.finalized[i] = finalized;
                }
            }
        }
        for batch in &self.batches {
            if let Some(formed) = batch.poa_at[batch.author] {
                if let Some(signed) = batch.body_at[batch.author] {
                    batch_wait.push(formed - signed);
                }
            }
        }
        let block_times: Vec<u64> = self.rounds.iter().skip(1).map(|r| r.proposed_at).collect();
        let proposals = self.rounds.len().saturating_sub(1).max(1);
        let mean_poas = self.rounds.iter().skip(1).map(|r| r.batches.len()).sum::<usize>() as f64 /
            proposals as f64;
        let window = ((cfg.warmup_s * 1e6) as u64, (cfg.duration_s * 1e6) as u64);
        let executed: Vec<f64> = self
            .rounds
            .iter()
            .skip(1)
            .filter(|r| r.proposed_at >= window.0 && r.proposed_at < window.1)
            .filter_map(|r| r.exec_cost)
            .collect();
        let mean_exec = executed.iter().sum::<f64>() / executed.len().max(1) as f64;
        let poa_latency =
            median(batch_wait).map_or("-".into(), |t| format!("{:.1}", t as f64 / 1000.0));
        let unproposed = self.open_poas.len();
        let extra = vec![
            ("batches".to_string(), self.batches.len().to_string()),
            ("PoAs per proposal (mean)".to_string(), format!("{mean_poas:.1}")),
            ("batch signed -> PoA, median (ms)".to_string(), poa_latency),
            ("mean block execution (ms)".to_string(), format!("{:.1}", mean_exec / 1000.0)),
            ("PoAs never proposed".to_string(), unproposed.to_string()),
            ("failed txs".to_string(), self.failed_txs.to_string()),
        ];
        Report::build(
            cfg,
            self.env.workload,
            RunOutput {
                protocol: "dx",
                times: &times,
                block_times: &block_times,
                cpu: &self.cpu,
                net: &self.net,
                extra,
            },
        )
    }
}
