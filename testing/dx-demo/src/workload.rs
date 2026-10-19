//! Synthetic transaction stream: native transfers and ERC20 transfers between funded accounts.

use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

use crate::{config::SimConfig, net::Topology};

/// Microseconds per second.
pub const US_PER_S: f64 = 1e6;

/// Kind of a generated transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxKind {
    /// Native value transfer.
    Transfer,
    /// `transfer(address,uint256)` on the demo token.
    Erc20,
}

/// One generated transaction.
#[derive(Debug, Clone)]
pub struct Tx {
    /// Index of the sending account.
    pub sender: u32,
    /// Index of the receiving account.
    pub recipient: u32,
    /// Sender nonce.
    pub nonce: u64,
    /// Transaction kind.
    pub kind: TxKind,
    /// Simulated time at which the user submits the transaction, in microseconds.
    pub created_us: u64,
}

impl Tx {
    /// Approximate RLP size of the signed transaction in bytes.
    pub const fn size(&self) -> u64 {
        match self.kind {
            TxKind::Transfer => 110,
            TxKind::Erc20 => 180,
        }
    }

    /// Gas limit declared by the sender, used to fill blocks and batches.
    pub const fn gas_limit(&self) -> u64 {
        match self.kind {
            TxKind::Transfer => 21_000,
            TxKind::Erc20 => 60_000,
        }
    }
}

/// The full transaction stream of a run, sorted by creation time.
#[derive(Debug)]
pub struct Workload {
    /// Transactions sorted by `created_us`.
    pub txs: Vec<Tx>,
    /// Region where each account submits its transactions.
    pub sender_region: Vec<usize>,
    /// Number of funded accounts.
    pub accounts: usize,
}

impl Workload {
    /// Generates a Poisson arrival stream at `cfg.tps` for `cfg.duration_s` seconds.
    pub fn generate(cfg: &SimConfig, topology: &Topology) -> Self {
        let mut rng = ChaCha8Rng::seed_from_u64(cfg.seed);
        let sender_region =
            (0..cfg.accounts).map(|_| rng.gen_range(0..topology.region_count())).collect();
        let mut nonces = vec![0u64; cfg.accounts];
        let mut txs = Vec::with_capacity((cfg.tps * cfg.duration_s) as usize + 16);
        let end_us = cfg.duration_s * US_PER_S;
        let mut t = 0.0;
        loop {
            let u: f64 = rng.gen_range(f64::EPSILON..1.0);
            t += -u.ln() / cfg.tps * US_PER_S;
            if t >= end_us {
                break;
            }
            let sender = rng.gen_range(0..cfg.accounts);
            let mut recipient = rng.gen_range(0..cfg.accounts);
            if recipient == sender {
                recipient = (recipient + 1) % cfg.accounts;
            }
            let kind = if rng.gen_bool(cfg.erc20_share) { TxKind::Erc20 } else { TxKind::Transfer };
            txs.push(Tx {
                sender: sender as u32,
                recipient: recipient as u32,
                nonce: nonces[sender],
                kind,
                created_us: t as u64,
            });
            nonces[sender] += 1;
        }
        Self { txs, sender_region, accounts: cfg.accounts }
    }

    /// Time at which transaction `idx` reaches validator `v` through transaction gossip.
    pub fn arrival_at(&self, idx: usize, v: usize, topology: &Topology) -> u64 {
        let tx = &self.txs[idx];
        tx.created_us + topology.region_latency_us(self.sender_region[tx.sender as usize], v)
    }
}
