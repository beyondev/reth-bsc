//! Network model: one-way latency between regions and a FIFO upload link per validator.
//!
//! Downlinks are not modelled; validators are assumed to be fully connected, which is how BSC
//! validators peer today.

use std::{
    collections::{BTreeMap, HashMap},
    time::Duration,
};

use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

use crate::{
    realnet::{Arrival, Cluster},
    sim::EventQueue,
};

/// Regions, the share of validators placed in each, and one-way latency between them in ms.
///
/// BSC validators run mostly in East Asian cloud regions; the latencies are typical public
/// cloud inter-region figures.
const REGIONS: [(&str, usize); 5] =
    [("tokyo", 4), ("singapore", 4), ("hongkong", 3), ("frankfurt", 1), ("virginia", 1)];
const LATENCY_MS: [[f64; 5]; 5] = [
    [1.0, 35.0, 25.0, 115.0, 75.0],
    [35.0, 1.0, 18.0, 80.0, 110.0],
    [25.0, 18.0, 1.0, 95.0, 100.0],
    [115.0, 80.0, 95.0, 1.0, 45.0],
    [75.0, 110.0, 100.0, 45.0, 1.0],
];
/// Bytes of framing added to every message.
const MESSAGE_OVERHEAD: u64 = 64;

/// Static placement of validators and the latency between them.
#[derive(Debug)]
pub struct Topology {
    /// Region of each validator.
    pub validator_region: Vec<usize>,
    /// One-way latency between validators in microseconds, with a fixed per-pair jitter.
    latency_us: Vec<Vec<u64>>,
    /// Uniform latency replacing the region table, in microseconds.
    uniform_us: Option<u64>,
}

impl Topology {
    /// Places `n` validators round-robin over the weighted region list, or puts every pair
    /// `uniform_ms` apart when that is set.
    pub fn new(n: usize, seed: u64, uniform_ms: Option<f64>) -> Self {
        if let Some(ms) = uniform_ms {
            let us = (ms * 1000.0) as u64;
            let latency_us =
                (0..n).map(|a| (0..n).map(|b| if a == b { 0 } else { us }).collect()).collect();
            return Self { validator_region: vec![0; n], latency_us, uniform_us: Some(us) };
        }
        let slots: Vec<usize> =
            REGIONS.iter().enumerate().flat_map(|(r, (_, w))| std::iter::repeat_n(r, *w)).collect();
        let validator_region: Vec<usize> = (0..n).map(|v| slots[v % slots.len()]).collect();
        let mut rng = ChaCha8Rng::seed_from_u64(seed ^ 0x6e65_7477);
        let mut latency_us = vec![vec![0u64; n]; n];
        for a in 0..n {
            for b in (a + 1)..n {
                let base = LATENCY_MS[validator_region[a]][validator_region[b]];
                let jitter: f64 = rng.gen_range(0.9..1.1);
                let us = (base * jitter * 1000.0) as u64;
                latency_us[a][b] = us;
                latency_us[b][a] = us;
            }
        }
        Self { validator_region, latency_us, uniform_us: None }
    }

    /// Number of regions.
    pub const fn region_count(&self) -> usize {
        REGIONS.len()
    }

    /// One-way latency between two validators.
    pub fn latency_us(&self, from: usize, to: usize) -> u64 {
        self.latency_us[from][to]
    }

    /// One-way latency from a region (where a user submits) to a validator.
    pub fn region_latency_us(&self, region: usize, to: usize) -> u64 {
        self.uniform_us.unwrap_or((LATENCY_MS[region][self.validator_region[to]] * 1000.0) as u64)
    }
}

/// Upload links of all validators, and the transport that carries messages over them.
///
/// In the default (modelled) mode a message's arrival time is computed from a FIFO upload link
/// and the latency table. With a [`Cluster`] the message is sent as real bytes between agent
/// processes and delivered when the last byte has arrived plus the link latency; the event loop
/// then runs in wall-clock time (see [`Network::next`]).
#[derive(Debug)]
pub struct Network<E> {
    /// Time at which each validator's upload link becomes idle.
    uplink_free: Vec<u64>,
    /// Upload speed in bytes per microsecond.
    bytes_per_us: f64,
    /// Bytes sent by each validator after the warm-up.
    pub bytes_sent: Vec<u64>,
    /// Statistics start time; traffic before it is not counted.
    measure_from_us: u64,
    /// Per-message jitter (plus or minus) in microseconds.
    jitter_us: u64,
    rng: ChaCha8Rng,
    real: Option<RealTransport<E>>,
}

/// State of the real-network mode.
#[derive(Debug)]
struct RealTransport<E> {
    cluster: Cluster,
    next_msg: u64,
    /// Event to deliver for each (message, recipient), with the link latency to add.
    pending: HashMap<(u64, usize), (E, u64)>,
    /// Sends the protocol scheduled for a later time (after modelled CPU work).
    deferred: BTreeMap<(u64, u64), DeferredSend>,
    /// Latest event time handed out, so the event loop never goes back in time.
    last_event: u64,
    stats: RealStats,
}

/// A send held back until the wall clock reaches its time.
#[derive(Debug)]
struct DeferredSend {
    from: usize,
    to: Vec<usize>,
    wire: u64,
}

/// How faithfully the wall-clock event loop kept up with the protocol.
#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct RealStats {
    /// How late each event was handled relative to its scheduled time, in microseconds.
    pub lag_us: Vec<u64>,
    /// Messages delivered over the real network.
    pub messages: u64,
    /// Deliveries that arrived after a later event had already been handled.
    pub reordered: u64,
}

impl<E> Network<E> {
    /// Creates idle links of `bandwidth_mbps` for `n` validators. Each message gets a random
    /// extra delay in `[-jitter_us, +jitter_us]`. With `cluster` set, messages travel over the
    /// real network between the cluster's agents.
    pub fn new(
        n: usize,
        bandwidth_mbps: f64,
        measure_from_us: u64,
        jitter_us: u64,
        seed: u64,
        cluster: Option<Cluster>,
    ) -> Self {
        Self {
            uplink_free: vec![0; n],
            bytes_per_us: bandwidth_mbps / 8.0,
            bytes_sent: vec![0; n],
            measure_from_us,
            jitter_us,
            rng: ChaCha8Rng::seed_from_u64(seed ^ 0x6a69_7474),
            real: cluster.map(|cluster| RealTransport {
                cluster,
                next_msg: 0,
                pending: HashMap::new(),
                deferred: BTreeMap::new(),
                last_event: 0,
                stats: RealStats::default(),
            }),
        }
    }

    /// Event-loop statistics of the real-network mode.
    pub fn real_stats(&self) -> Option<&RealStats> {
        self.real.as_ref().map(|r| &r.stats)
    }

    fn latency(&mut self, topology: &Topology, from: usize, dst: usize) -> u64 {
        let mut latency = topology.latency_us(from, dst) as i64;
        if self.jitter_us > 0 {
            let j = self.jitter_us as i64;
            latency = (latency + self.rng.gen_range(-j..=j)).max(0);
        }
        latency as u64
    }

    /// Sends `bytes` from `from` to each of `to` at time `now` and schedules `event(recipient)`
    /// at each recipient's arrival. In the modelled mode the copies go one after another on the
    /// upload link.
    #[allow(clippy::too_many_arguments)]
    pub fn send(
        &mut self,
        topology: &Topology,
        now: u64,
        from: usize,
        to: &[usize],
        bytes: u64,
        q: &mut EventQueue<E>,
        event: impl Fn(usize) -> E,
    ) {
        let wire = bytes + MESSAGE_OVERHEAD;
        if now >= self.measure_from_us {
            self.bytes_sent[from] += wire * to.len() as u64;
        }
        if self.real.is_some() {
            let latencies: Vec<u64> = to.iter().map(|&d| self.latency(topology, from, d)).collect();
            let real = self.real.as_mut().expect("real mode");
            let msg = real.next_msg;
            real.next_msg += 1;
            for (&dst, latency) in to.iter().zip(latencies) {
                real.pending.insert((msg, dst), (event(dst), latency));
            }
            if now <= real.cluster.now() {
                real.cluster.dispatch(from, msg, to, wire);
            } else {
                real.deferred.insert((now, msg), DeferredSend { from, to: to.to_vec(), wire });
            }
            return;
        }
        let per_copy = wire as f64 / self.bytes_per_us;
        let mut t = self.uplink_free[from].max(now) as f64;
        for &dst in to {
            t += per_copy;
            let latency = self.latency(topology, from, dst);
            q.push(t as u64 + latency, event(dst));
        }
        self.uplink_free[from] = t as u64;
    }

    /// Sends to every validator except `from`, nearest first.
    #[allow(clippy::too_many_arguments)]
    pub fn broadcast(
        &mut self,
        topology: &Topology,
        now: u64,
        from: usize,
        bytes: u64,
        q: &mut EventQueue<E>,
        event: impl Fn(usize) -> E,
    ) {
        let mut peers: Vec<usize> = (0..self.uplink_free.len()).filter(|&v| v != from).collect();
        peers.sort_by_key(|&v| topology.latency_us(from, v));
        self.send(topology, now, from, &peers, bytes, q, event);
    }

    /// Next event to handle. In the modelled mode this is simply the earliest queued event. In
    /// the real-network mode it waits until the earliest event is due in wall-clock time,
    /// meanwhile dispatching deferred sends and turning reported arrivals into events.
    pub fn next(&mut self, q: &mut EventQueue<E>) -> Option<(u64, E)> {
        let Some(real) = self.real.as_mut() else { return q.pop() };
        loop {
            while let Ok(a) = real.cluster.arrivals.try_recv() {
                real.deliver(a, q);
            }
            let now = real.cluster.now();
            while let Some((&(t, msg), _)) = real.deferred.first_key_value() {
                if t > now {
                    break;
                }
                let send = real.deferred.remove(&(t, msg)).expect("present");
                real.cluster.dispatch(send.from, msg, &send.to, send.wire);
            }
            if let Some(t) = q.peek_time() {
                if t <= now {
                    let (t, e) = q.pop().expect("peeked");
                    real.stats.lag_us.push(now - t);
                    let t = t.max(real.last_event);
                    real.last_event = t;
                    return Some((t, e));
                }
            }
            let wake = [q.peek_time(), real.deferred.first_key_value().map(|(&(t, _), _)| t)]
                .into_iter()
                .flatten()
                .min();
            if wake.is_none() && real.pending.is_empty() {
                return None;
            }
            let wait = wake.map_or(5_000, |t| t.saturating_sub(now).min(5_000));
            if let Ok(a) = real.cluster.arrivals.recv_timeout(Duration::from_micros(wait)) {
                real.deliver(a, q);
            }
        }
    }
}

impl<E> RealTransport<E> {
    fn deliver(&mut self, a: Arrival, q: &mut EventQueue<E>) {
        if let Some((event, latency)) = self.pending.remove(&(a.msg, a.dst)) {
            let at = self.cluster.to_run_time(a.recv_us) + latency;
            if at < self.last_event {
                self.stats.reordered += 1;
            }
            self.stats.messages += 1;
            q.push(at.max(self.last_event), event);
        }
    }
}
