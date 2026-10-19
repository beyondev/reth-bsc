//! Simulation parameters shared by both protocols.

use clap::Args;
use serde::Serialize;

/// Parameters of one simulation run.
///
/// Defaults follow the DX design doc (`n = 45`, `TurnLength = 16`, `T_batch = 100ms`,
/// `T_minInterval = 450ms`, `k = 3`) and the current BSC mainnet block interval of 450ms.
#[derive(Debug, Clone, Args, Serialize)]
pub struct SimConfig {
    /// Number of validators.
    #[arg(long, default_value_t = 45)]
    pub validators: usize,
    /// Simulated seconds during which transactions are generated.
    #[arg(long, default_value_t = 40.0)]
    pub duration_s: f64,
    /// Transactions created before this many seconds are excluded from the statistics.
    #[arg(long, default_value_t = 8.0)]
    pub warmup_s: f64,
    /// Offered load in transactions per second.
    #[arg(long, default_value_t = 2000.0)]
    pub tps: f64,
    /// Share of ERC20 transfers in the workload; the rest are native transfers.
    #[arg(long, default_value_t = 0.5)]
    pub erc20_share: f64,
    /// Number of funded accounts that send transactions.
    #[arg(long, default_value_t = 50_000)]
    pub accounts: usize,
    /// Upload bandwidth of each validator in Mbit/s.
    #[arg(long, default_value_t = 1000.0)]
    pub bandwidth_mbps: f64,
    /// CPU cores each validator can spend on signatures and sender recovery.
    #[arg(long, default_value_t = 8)]
    pub sig_cores: usize,
    /// Target execution cost per transaction in microseconds. When unset the cost measured with
    /// in-memory revm is used as is.
    #[arg(long)]
    pub exec_us_per_tx: Option<f64>,
    /// Fixed cost of importing one block (header checks, bookkeeping, persistence hand-off).
    #[arg(long, default_value_t = 1.0)]
    pub block_overhead_ms: f64,
    /// Random seed for the workload and the topology jitter.
    #[arg(long, default_value_t = 7)]
    pub seed: u64,

    /// Parlia: block interval in milliseconds.
    #[arg(long, default_value_t = 450)]
    pub block_interval_ms: u64,
    /// Parlia and DX: consecutive blocks (rounds) per proposer.
    #[arg(long, default_value_t = 16)]
    pub turn_length: u64,
    /// Parlia: block gas limit. DX uses the same value as the per-block ordered gas cap.
    #[arg(long, default_value_t = 100_000_000)]
    pub gas_limit: u64,
    /// Parlia: transactions arriving later than this before the block time miss the block.
    #[arg(long, default_value_t = 120)]
    pub miner_reserve_ms: u64,
    /// Parlia: the proposer sends the full block to every validator itself instead of to
    /// sqrt(n) of them that relay it on.
    #[arg(long)]
    pub parlia_direct: bool,

    /// DX: batch cut interval in milliseconds.
    #[arg(long, default_value_t = 100)]
    pub t_batch_ms: u64,
    /// DX: maximum batch size in bytes.
    #[arg(long, default_value_t = 256 * 1024)]
    pub s_batch: usize,
    /// DX: minimum interval between consecutive proposals in milliseconds.
    #[arg(long, default_value_t = 450)]
    pub min_interval_ms: u64,
    /// DX: execution lag; the proposal of round `r` must carry the ExecCert of block `r - k`.
    #[arg(long, default_value_t = 3)]
    pub exec_lag: u64,
    /// DX: execute verified proposals before they commit (up to 8 blocks ahead). ExecVotes are
    /// still only signed for committed blocks.
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    pub speculative: bool,
}

impl SimConfig {
    /// Maximum number of Byzantine validators tolerated, `n >= 3f + 1`.
    pub const fn f(&self) -> usize {
        (self.validators - 1) / 3
    }

    /// Quorum size `2f + 1`.
    pub const fn quorum(&self) -> usize {
        2 * self.f() + 1
    }

    /// Availability threshold `f + 1` used for batch PoAs.
    pub const fn availability(&self) -> usize {
        self.f() + 1
    }
}
