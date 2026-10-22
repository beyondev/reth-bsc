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
    /// One-way latency between every pair of validators (and from users) in ms. Replaces the
    /// five-region model when set.
    #[arg(long)]
    pub latency_ms: Option<f64>,
    /// Per-message latency jitter (plus or minus) in ms, used with `--latency-ms`.
    #[arg(long, default_value_t = 2.0)]
    pub jitter_ms: f64,
    /// Execution time of one block in ms at least; the measured cost is padded up to it.
    #[arg(long)]
    pub exec_block_ms: Option<f64>,
    /// Validators that are down: they never propose, vote, ack or batch.
    #[arg(long, value_delimiter = ',')]
    pub crashed: Vec<usize>,
    /// Validators whose execution produces a wrong state root.
    #[arg(long, value_delimiter = ',')]
    pub bad_exec: Vec<usize>,

    /// Carry every message as real bytes over TCP between one agent process per validator,
    /// each in its own network namespace with an HTB-limited uplink, and run the event loop in
    /// wall-clock time. Needs root, `ip` and `tc`. Latency and jitter are still added in user
    /// space.
    #[arg(long)]
    pub real_net: bool,

    /// With `--real-net`: each agent sends its frames one after another, in the order the
    /// protocol queued them, paced at the uplink rate, instead of writing to all peers at once and
    /// letting the qdisc share the link between the flows. This is the FIFO upload link of the
    /// modelled mode, carried over real TCP.
    #[arg(long)]
    pub real_net_serial: bool,

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
    /// Parlia: how far back a header's vote attestation may reach. 1 is the pre-Fermi rule
    /// (parent only); Fermi raised it to 3 (`kAncestorGenerationDepth`), so a block whose votes
    /// arrive after the next seal can still be justified by a later header.
    #[arg(long, default_value_t = 1)]
    pub attest_depth: u64,
    /// Parlia: a header may carry one attestation for every ancestor within `attest_depth` that
    /// has a quorum, applied oldest first, instead of only the nearest one. Not in any fork yet;
    /// it shows what a multi-target attestation would buy when votes arrive late.
    #[arg(long)]
    pub multi_attest: bool,
    /// Parlia: relay blocks as compact blocks, sending this many bytes per transaction (a short
    /// id) instead of the full transaction. Assumes every receiver already holds every
    /// transaction, so it is a lower bound on the wire cost.
    #[arg(long)]
    pub compact_tx_bytes: Option<u64>,
    /// Parlia: stream the block while building it (`streaming-blocks-incremental-exec.md`). Every
    /// this many ms the proposer signs and relays the transactions it appended since the last
    /// chunk; validators that hold the parent execute them speculatively, so after the seal they
    /// only execute what is left. The seal message carries the header and the remaining
    /// transactions.
    #[arg(long)]
    pub stream_chunk_ms: Option<u64>,
    /// Parlia with `--stream-chunk-ms`: share of each block's transactions that come from a
    /// builder bid. The bid goes first in the block and is only known once it is locked, so
    /// nothing can be streamed before then.
    #[arg(long, default_value_t = 0.0)]
    pub bid_share: f64,
    /// Parlia with `--stream-chunk-ms`: the winning bid is locked this long before the block time
    /// and streamed as the block's prefix; later bids may only append. Unset, the bid is chosen
    /// at the last moment (`--miner-reserve-ms`), as geth does today.
    #[arg(long)]
    pub lock_ms: Option<u64>,
    /// Parlia with `--stream-chunk-ms` and `--bid-share`: bid transactions are locked chunk by
    /// chunk as they reach the proposer, like local ones, so no bid arrives late and nothing is
    /// executed again. The builder already executed them, so their chunks do not wait for the
    /// proposer's own execution. Overrides `--lock-ms`.
    #[arg(long)]
    pub bid_stream: bool,
    /// Parlia: delay per backoff rank before another validator fills a missing in-turn block.
    #[arg(long, default_value_t = 1000)]
    pub backoff_ms: u64,

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
    /// DX: base round timeout `T_base` in ms, counted from when the leader may propose; it
    /// doubles with each consecutive timeout. It must exceed a proposal plus vote round trip,
    /// about 230 ms in the default five-region placement.
    #[arg(long, default_value_t = 1000)]
    pub round_timeout_ms: u64,
    /// DX: how long a down owner's transactions wait before another validator batches them
    /// (`T_fallback`), in ms.
    #[arg(long, default_value_t = 2000)]
    pub fallback_ms: u64,
    /// DX: share of each block's transaction bytes the leader carries inline in its proposal
    /// (a builder batch) instead of by PoA reference. Receivers also recover those senders
    /// before voting. The batch layer still carries every transaction, so batch bandwidth is
    /// overstated when this is set.
    #[arg(long, default_value_t = 0.0)]
    pub dx_inline_share: f64,
    /// DX: the leader that forms a QC broadcasts it at once instead of only carrying it in its
    /// next proposal, so validators commit one message hop earlier.
    #[arg(long)]
    pub qc_broadcast: bool,
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

    /// Whether validator `v` is running.
    pub fn is_up(&self, v: usize) -> bool {
        !self.crashed.contains(&v)
    }

    /// Whether validator `v` is running and executes correctly.
    pub fn is_honest(&self, v: usize) -> bool {
        self.is_up(v) && !self.bad_exec.contains(&v)
    }
}
