//! Per-run statistics and their rendering.

use serde::Serialize;

use crate::{config::SimConfig, net::RealStats, sim::Cpu, workload::Workload};

/// Absolute times at which each transaction reached a milestone. `None` means never.
#[derive(Debug)]
pub struct TxTimes {
    /// Included in a block (Parlia: block sealed; DX: block order committed at the median
    /// validator).
    pub ordered: Vec<Option<u64>>,
    /// Executed by the median validator, so its receipt is available there.
    pub executed: Vec<Option<u64>>,
    /// State containing the transaction is final (Parlia: fast finality; DX: ExecCert at the
    /// median validator).
    pub finalized: Vec<Option<u64>>,
}

impl TxTimes {
    /// Creates empty records for `n` transactions.
    pub fn new(n: usize) -> Self {
        Self { ordered: vec![None; n], executed: vec![None; n], finalized: vec![None; n] }
    }
}

/// Latency percentiles of one milestone, in milliseconds.
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct Latency {
    /// Median.
    pub p50: f64,
    /// 95th percentile.
    pub p95: f64,
    /// 99th percentile.
    pub p99: f64,
}

impl Latency {
    fn from_samples(mut samples: Vec<u64>) -> Self {
        if samples.is_empty() {
            return Self::default();
        }
        samples.sort_unstable();
        let pick = |q: f64| {
            let idx = ((samples.len() - 1) as f64 * q).round() as usize;
            samples[idx] as f64 / 1000.0
        };
        Self { p50: pick(0.50), p95: pick(0.95), p99: pick(0.99) }
    }
}

/// Block-level figures, measured from the moment a block leaves its proposer.
#[derive(Debug, Clone, Default, Serialize)]
pub struct BlockStats {
    /// Canonical blocks per second in the window.
    pub blocks_per_s: f64,
    /// Transactions per second in those blocks.
    pub tps: f64,
    /// Mean bytes of the message on the critical path per block: the full block for Parlia,
    /// the proposal for DX. Rejected and orphaned blocks count too.
    pub critical_bytes: f64,
    /// How much later an honest validator has the block's execution result than its proposer.
    pub state_gap_p50_ms: f64,
    /// Until 2/3 of the validators have signed the block's execution result at the median
    /// honest validator (Parlia: votes, which cover the state root; DX: ExecCert).
    pub quorum_sig_p50_ms: f64,
    /// DX only: until the QC on the block's order forms at the next leader. NaN for Parlia.
    pub order_qc_p50_ms: f64,
    /// Until the block is irreversible at the median honest validator (Parlia: the block and
    /// its child both have 2/3 votes; DX: committed and ExecCert).
    pub irreversible_p50_ms: f64,
    /// 99th percentile of the above.
    pub irreversible_p99_ms: f64,
    /// Parlia: blocks produced out of turn after a backoff. DX: rounds ended by a TC.
    pub faults: usize,
    /// Honest validators never committed different blocks at the same height, and every
    /// ExecCert matches the honest execution result.
    pub safe: bool,
}

/// Percentile `q` of `values` (microseconds) in milliseconds, NaN when empty.
pub fn percentile_ms(values: &mut [u64], q: f64) -> f64 {
    if values.is_empty() {
        return f64::NAN;
    }
    values.sort_unstable();
    values[((values.len() - 1) as f64 * q).round() as usize] as f64 / 1000.0
}

/// Summary of one run.
#[derive(Debug, Clone, Serialize)]
pub struct Report {
    /// Protocol name.
    pub protocol: String,
    /// Offered load.
    pub offered_tps: f64,
    /// Transactions created inside the measurement window.
    pub measured_txs: usize,
    /// Share of measured transactions that reached finality before the simulation ended.
    pub finalized_ratio: f64,
    /// Finalized transactions per second over the measurement window.
    pub finalized_tps: f64,
    /// Submission to inclusion.
    pub ordered: Latency,
    /// Submission to execution at the median validator.
    pub executed: Latency,
    /// Submission to state finality.
    pub finalized: Latency,
    /// Mean interval between consecutive blocks in the window, in ms.
    pub block_interval_ms: f64,
    /// Blocks produced in the window.
    pub blocks: usize,
    /// Mean execution-lane utilization over validators, as a fraction of one core.
    pub exec_lane_util_avg: f64,
    /// Highest execution-lane utilization of any validator.
    pub exec_lane_util_max: f64,
    /// Mean cores spent on signatures and sender recovery per validator.
    pub sig_cores_avg: f64,
    /// Highest cores spent on signatures and sender recovery by any validator.
    pub sig_cores_max: f64,
    /// Mean upload bandwidth per validator, in Mbit/s.
    pub upload_mbps_avg: f64,
    /// Highest upload bandwidth of any validator, in Mbit/s.
    pub upload_mbps_max: f64,
    /// Block-level figures.
    pub block: BlockStats,
    /// Protocol-specific figures.
    pub extra: Vec<(String, String)>,
}

/// Inputs to [`Report::build`] that every protocol produces.
#[derive(Debug)]
pub struct RunOutput<'a> {
    /// Protocol name.
    pub protocol: &'a str,
    /// Milestone times per transaction.
    pub times: &'a TxTimes,
    /// Production time of every block, in order.
    pub block_times: &'a [u64],
    /// CPU accounting.
    pub cpu: &'a Cpu,
    /// Bytes each validator sent inside the measurement window.
    pub bytes_sent: &'a [u64],
    /// Event-loop statistics when messages went over the real network.
    pub real: Option<&'a RealStats>,
    /// Block-level figures.
    pub block: BlockStats,
    /// Protocol-specific figures.
    pub extra: Vec<(String, String)>,
}

impl Report {
    /// Aggregates a run over the measurement window `[warmup, duration)`.
    pub fn build(cfg: &SimConfig, workload: &Workload, out: RunOutput<'_>) -> Self {
        let from = (cfg.warmup_s * 1e6) as u64;
        let to = (cfg.duration_s * 1e6) as u64;
        let window_s = (to - from) as f64 / 1e6;
        let measured: Vec<usize> = workload
            .txs
            .iter()
            .enumerate()
            .filter(|(_, tx)| tx.created_us >= from && tx.created_us < to)
            .map(|(i, _)| i)
            .collect();
        let samples = |col: &[Option<u64>]| {
            measured
                .iter()
                .filter_map(|&i| col[i].map(|t| t.saturating_sub(workload.txs[i].created_us)))
                .collect::<Vec<_>>()
        };
        let finalized = samples(&out.times.finalized);
        let finalized_ratio =
            if measured.is_empty() { 0.0 } else { finalized.len() as f64 / measured.len() as f64 };

        let in_window: Vec<u64> =
            out.block_times.iter().copied().filter(|&t| t >= from && t < to).collect();
        let block_interval_ms = if in_window.len() > 1 {
            (in_window[in_window.len() - 1] - in_window[0]) as f64 /
                (in_window.len() - 1) as f64 /
                1000.0
        } else {
            f64::NAN
        };

        let window_us = window_s * 1e6;
        let n = out.cpu.exec_busy.len() as f64;
        let util = |v: &[f64]| {
            let avg = v.iter().sum::<f64>() / n / window_us;
            let max = v.iter().copied().fold(0.0, f64::max) / window_us;
            (avg, max)
        };
        let (exec_avg, exec_max) = util(&out.cpu.exec_busy);
        let (sig_avg, sig_max) = util(&out.cpu.sig_busy);
        let mbps: Vec<f64> =
            out.bytes_sent.iter().map(|&b| b as f64 * 8.0 / 1e6 / window_s).collect();

        Self {
            protocol: out.protocol.to_string(),
            offered_tps: cfg.tps,
            measured_txs: measured.len(),
            finalized_ratio,
            finalized_tps: finalized.len() as f64 / window_s,
            ordered: Latency::from_samples(samples(&out.times.ordered)),
            executed: Latency::from_samples(samples(&out.times.executed)),
            finalized: Latency::from_samples(finalized),
            block_interval_ms,
            blocks: in_window.len(),
            exec_lane_util_avg: exec_avg,
            exec_lane_util_max: exec_max,
            sig_cores_avg: sig_avg,
            sig_cores_max: sig_max,
            upload_mbps_avg: mbps.iter().sum::<f64>() / n,
            upload_mbps_max: mbps.iter().copied().fold(0.0, f64::max),
            block: out.block,
            extra: with_real_stats(out.extra, out.real),
        }
    }

    /// Renders reports side by side as a Markdown table.
    pub fn table(reports: &[Self]) -> String {
        let mut rows: Vec<(String, Vec<String>)> = vec![
            ("offered load (tx/s)".into(), reports.iter().map(|r| fmt0(r.offered_tps)).collect()),
            (
                "finalized (tx/s)".into(),
                reports
                    .iter()
                    .map(|r| {
                        format!("{} ({:.1}%)", fmt0(r.finalized_tps), r.finalized_ratio * 100.0)
                    })
                    .collect(),
            ),
            (
                "block interval (ms)".into(),
                reports.iter().map(|r| fmt0(r.block_interval_ms)).collect(),
            ),
        ];
        let lat = |name: &str, f: fn(&Self) -> Latency| {
            (
                format!("{name} p50 / p95 (ms)"),
                reports
                    .iter()
                    .map(|r| format!("{} / {}", fmt0(f(r).p50), fmt0(f(r).p95)))
                    .collect(),
            )
        };
        rows.push(lat("submit -> ordered", |r| r.ordered));
        rows.push(lat("submit -> executed", |r| r.executed));
        rows.push(lat("submit -> final", |r| r.finalized));
        rows.push((
            "exec lane busy avg / max".into(),
            reports
                .iter()
                .map(|r| {
                    format!(
                        "{:.0}% / {:.0}%",
                        r.exec_lane_util_avg * 100.0,
                        r.exec_lane_util_max * 100.0
                    )
                })
                .collect(),
        ));
        rows.push((
            "sig + recovery cores avg / max".into(),
            reports
                .iter()
                .map(|r| format!("{:.2} / {:.2}", r.sig_cores_avg, r.sig_cores_max))
                .collect(),
        ));
        rows.push((
            "upload Mbit/s avg / max".into(),
            reports
                .iter()
                .map(|r| format!("{:.0} / {:.0}", r.upload_mbps_avg, r.upload_mbps_max))
                .collect(),
        ));
        let mut keys: Vec<String> = Vec::new();
        for r in reports {
            for (k, _) in &r.extra {
                if !keys.contains(k) {
                    keys.push(k.clone());
                }
            }
        }
        for k in keys {
            let vals = reports
                .iter()
                .map(|r| {
                    r.extra.iter().find(|(key, _)| *key == k).map_or("-".into(), |(_, v)| v.clone())
                })
                .collect();
            rows.push((k, vals));
        }

        let mut out = String::from("| metric |");
        for r in reports {
            out.push_str(&format!(" {} |", r.protocol));
        }
        out.push_str("\n| --- |");
        for _ in reports {
            out.push_str(" --- |");
        }
        out.push('\n');
        for (name, vals) in rows {
            out.push_str(&format!("| {name} |"));
            for v in vals {
                out.push_str(&format!(" {v} |"));
            }
            out.push('\n');
        }
        out
    }
}

fn fmt0(v: f64) -> String {
    if v.is_nan() {
        "-".into()
    } else {
        format!("{v:.0}")
    }
}

/// Median of a non-empty set of times.
pub fn median(mut values: Vec<u64>) -> Option<u64> {
    if values.is_empty() {
        return None;
    }
    let mid = values.len() / 2;
    Some(*values.select_nth_unstable(mid).1)
}

/// Appends the real-network event-loop figures to a run's protocol-specific rows.
fn with_real_stats(
    mut extra: Vec<(String, String)>,
    real: Option<&RealStats>,
) -> Vec<(String, String)> {
    if let Some(r) = real {
        let mut lag = r.lag_us.clone();
        extra.push(("real-net messages".to_string(), r.messages.to_string()));
        extra.push((
            "event lag p50 / p99 / max (ms)".to_string(),
            format!(
                "{:.2} / {:.2} / {:.2}",
                percentile_ms(&mut lag.clone(), 0.5),
                percentile_ms(&mut lag.clone(), 0.99),
                lag.iter_mut().map(|l| *l as f64 / 1000.0).fold(0.0, f64::max)
            ),
        ));
        extra.push(("deliveries after a later event".to_string(), r.reordered.to_string()));
    }
    extra
}
