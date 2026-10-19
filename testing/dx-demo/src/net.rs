//! Network model: one-way latency between regions and a FIFO upload link per validator.
//!
//! Downlinks are not modelled; validators are assumed to be fully connected, which is how BSC
//! validators peer today.

use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

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
}

impl Topology {
    /// Places `n` validators round-robin over the weighted region list.
    pub fn new(n: usize, seed: u64) -> Self {
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
        Self { validator_region, latency_us }
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
        (LATENCY_MS[region][self.validator_region[to]] * 1000.0) as u64
    }
}

/// Upload links of all validators.
#[derive(Debug)]
pub struct Network {
    /// Time at which each validator's upload link becomes idle.
    uplink_free: Vec<u64>,
    /// Upload speed in bytes per microsecond.
    bytes_per_us: f64,
    /// Bytes sent by each validator after the warm-up.
    pub bytes_sent: Vec<u64>,
    /// Statistics start time; traffic before it is not counted.
    measure_from_us: u64,
}

impl Network {
    /// Creates idle links of `bandwidth_mbps` for `n` validators.
    pub fn new(n: usize, bandwidth_mbps: f64, measure_from_us: u64) -> Self {
        Self {
            uplink_free: vec![0; n],
            bytes_per_us: bandwidth_mbps / 8.0,
            bytes_sent: vec![0; n],
            measure_from_us,
        }
    }

    /// Sends `bytes` from `from` to each of `to`, one copy after another on the upload link.
    /// Returns the arrival time at each recipient, in the order given.
    pub fn send(
        &mut self,
        topology: &Topology,
        now: u64,
        from: usize,
        to: &[usize],
        bytes: u64,
    ) -> Vec<(usize, u64)> {
        let wire = bytes + MESSAGE_OVERHEAD;
        let per_copy = wire as f64 / self.bytes_per_us;
        let mut t = self.uplink_free[from].max(now) as f64;
        let mut out = Vec::with_capacity(to.len());
        for &dst in to {
            t += per_copy;
            out.push((dst, t as u64 + topology.latency_us(from, dst)));
        }
        self.uplink_free[from] = t as u64;
        if now >= self.measure_from_us {
            self.bytes_sent[from] += wire * to.len() as u64;
        }
        out
    }

    /// Sends to every validator except `from`, nearest first.
    pub fn broadcast(
        &mut self,
        topology: &Topology,
        now: u64,
        from: usize,
        bytes: u64,
    ) -> Vec<(usize, u64)> {
        let mut peers: Vec<usize> = (0..self.uplink_free.len()).filter(|&v| v != from).collect();
        peers.sort_by_key(|&v| topology.latency_us(from, v));
        self.send(topology, now, from, &peers, bytes)
    }
}
