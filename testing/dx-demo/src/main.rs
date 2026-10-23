//! `dx-demo`: compares Parlia with the DX two-phase design on the same workload.
//!
//! Both protocols run in one process as a discrete-event simulation over a modelled network.
//! Transactions are executed for real with revm, and signature costs are measured with the same
//! libraries reth-bsc uses (`blst`, `ed25519-dalek`, `secp256k1`).

mod config;
mod costs;
mod dx;
mod exec;
mod net;
mod parlia;
mod realnet;
mod report;
mod sim;
mod workload;

use std::{path::PathBuf, time::Instant};

use clap::{Parser, Subcommand, ValueEnum};

use crate::{
    config::SimConfig, costs::CryptoCosts, exec::ExecCostModel, net::Topology, report::Report,
    workload::Workload,
};

/// Everything a protocol run reads.
#[derive(Debug)]
pub struct Env<'a> {
    /// Run parameters.
    pub cfg: &'a SimConfig,
    /// Transaction stream.
    pub workload: &'a Workload,
    /// Validator placement and latencies.
    pub topo: &'a Topology,
    /// Signature costs.
    pub crypto: &'a CryptoCosts,
    /// Execution costs.
    pub exec_model: &'a ExecCostModel,
    /// Simulated seconds after the last transaction is created, so it can reach finality.
    pub drain_s: f64,
    /// Agent launch details when messages go over the real network.
    pub realnet: Option<&'a realnet::RealNetSpec>,
}

#[derive(Debug, Parser)]
#[command(about = "Compare Parlia with the DX two-phase consensus design")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Debug, Subcommand)]
enum Cmd {
    /// Print the measured signature and execution costs.
    Calibrate {
        #[command(flatten)]
        cfg: SimConfig,
    },
    /// Run one scenario and print the comparison table.
    Run {
        #[command(flatten)]
        cfg: SimConfig,
        /// Which protocols to run.
        #[arg(long, value_enum, default_value_t = Which::Both)]
        protocol: Which,
        /// Also write the reports as JSON.
        #[arg(long)]
        json: Option<PathBuf>,
    },
    /// Run the six fault and load scenarios (4 validators, 50 ms blocks) three times each and
    /// write a block-level Markdown table.
    Scenarios {
        /// Markdown output file.
        #[arg(long, default_value = "dx-demo-scenarios.md")]
        out: PathBuf,
        /// Also write every run's report as JSON.
        #[arg(long)]
        json: Option<PathBuf>,
        /// Seeds to run each scenario with; the run with the median block rate is shown.
        #[arg(long, value_delimiter = ',', default_value = "1,2,3")]
        seeds: Vec<u64>,
        /// Run every scenario over the real network (see `run --real-net`).
        #[arg(long)]
        real_net: bool,
    },
    /// Real-network agent for one validator (started by `--real-net` runs, not by hand).
    Agent {
        /// Validator index.
        #[arg(long)]
        id: usize,
        /// Number of validators.
        #[arg(long)]
        n: usize,
        /// Controller's control socket.
        #[arg(long)]
        ctl: PathBuf,
        /// Send frames one after another, paced at this upload rate in Mbit/s.
        #[arg(long)]
        serial_mbps: Option<f64>,
    },
    /// Run the built-in scenario set and write a Markdown report.
    Sweep {
        #[command(flatten)]
        cfg: SimConfig,
        /// Markdown output file.
        #[arg(long, default_value = "dx-demo-results.md")]
        out: PathBuf,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Which {
    Both,
    Parlia,
    Dx,
}

/// Explains the table rows at the top of the sweep output.
const LEGEND: &str =
    "Rows: `ordered` is when the block holding the transaction was sealed (Parlia) or its \
order was committed at the median validator (DX). `executed` is when the median validator had \
executed it (for DX this includes speculative execution of not yet committed blocks). `final` is \
state finality: fast finality for Parlia, ExecCert at the median validator for DX. CPU figures \
are per validator over the measurement window.\n\n";

/// Simulated seconds allowed for in-flight transactions to finalize after generation stops.
const DRAIN_S: f64 = 15.0;

fn main() {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Calibrate { cfg } => {
            let crypto = CryptoCosts::calibrate(cfg.quorum());
            let exec = ExecCostModel::calibrate(&cfg);
            println!("{}", serde_json::to_string_pretty(&crypto).expect("serializable"));
            println!("{}", serde_json::to_string_pretty(&exec).expect("serializable"));
        }
        Cmd::Run { cfg, protocol, json } => {
            let crypto = CryptoCosts::calibrate(cfg.quorum());
            let reports = run_scenario(&cfg, &crypto, protocol);
            println!("{}", Report::table(&reports));
            if let Some(path) = json {
                let body = serde_json::to_string_pretty(&reports).expect("serializable");
                std::fs::write(path, body).expect("writable output");
            }
        }
        Cmd::Scenarios { out, json, seeds, real_net } => {
            let (md, runs) = scenarios(&seeds, real_net);
            std::fs::write(&out, md).expect("writable output");
            println!("wrote {}", out.display());
            if let Some(path) = json {
                let body = serde_json::to_string_pretty(&runs).expect("serializable");
                std::fs::write(path, body).expect("writable output");
            }
        }
        Cmd::Agent { id, n, ctl, serial_mbps } => realnet::agent_main(id, n, &ctl, serial_mbps),
        Cmd::Sweep { cfg, out } => {
            let md = sweep(&cfg);
            std::fs::write(&out, md).expect("writable output");
            println!("wrote {}", out.display());
        }
    }
}

fn run_scenario(cfg: &SimConfig, crypto: &CryptoCosts, which: Which) -> Vec<Report> {
    let topo = Topology::new(cfg.validators, cfg.seed, cfg.latency_ms);
    let workload = Workload::generate(cfg, &topo);
    let exec_model = ExecCostModel::calibrate(cfg);
    let spec = cfg.real_net.then(|| realnet::RealNetSpec {
        dir: std::env::temp_dir().join("dx-realnet"),
        exe: std::env::current_exe().expect("own executable"),
        serial_mbps: cfg.real_net_serial.then_some(cfg.bandwidth_mbps),
    });
    let _namespaces =
        cfg.real_net.then(|| realnet::Namespaces::create(cfg.validators, cfg.bandwidth_mbps));
    let env = Env {
        cfg,
        workload: &workload,
        topo: &topo,
        crypto,
        exec_model: &exec_model,
        drain_s: DRAIN_S,
        realnet: spec.as_ref(),
    };
    let mut reports = Vec::new();
    if which != Which::Dx {
        let start = Instant::now();
        reports.push(parlia::run(&env));
        eprintln!(
            "parlia: {} txs simulated in {:.1}s",
            workload.txs.len(),
            start.elapsed().as_secs_f64()
        );
    }
    if which != Which::Parlia {
        let start = Instant::now();
        reports.push(dx::run(&env));
        eprintln!(
            "dx: {} txs simulated in {:.1}s",
            workload.txs.len(),
            start.elapsed().as_secs_f64()
        );
    }
    reports
}

/// One named variation of the base configuration.
struct Scenario {
    title: &'static str,
    note: &'static str,
    apply: fn(&mut SimConfig),
}

/// Base configuration of the `scenarios` command: 4 validators, 2000 tx/s, every block takes
/// 20 ms to execute, 5 ms ± 2 ms one-way latency, 1 Gbit/s uplinks, 50 ms blocks, 4 blocks per
/// proposer, `k = 3`, 1 s Parlia backoff, 100 ms DX `T_base`, a 10 s measurement window.
const SCENARIO_BASE: &[&str] = &[
    "dx-demo",
    "--validators",
    "4",
    "--tps",
    "2000",
    "--exec-block-ms",
    "20",
    "--latency-ms",
    "5",
    "--jitter-ms",
    "2",
    "--block-interval-ms",
    "50",
    "--min-interval-ms",
    "50",
    "--turn-length",
    "4",
    "--exec-lag",
    "3",
    "--backoff-ms",
    "1000",
    "--round-timeout-ms",
    "100",
    "--duration-s",
    "11",
    "--warmup-s",
    "1",
];

/// One protocol variant of the `scenarios` command.
struct Variant {
    label: &'static str,
    which: Which,
    apply: fn(&mut SimConfig),
}

/// One run of the `scenarios` command, kept for the JSON output.
#[derive(Debug, serde::Serialize)]
struct ScenarioRun {
    scenario: &'static str,
    variant: &'static str,
    seed: u64,
    report: Report,
}

/// Command line holding only a [`SimConfig`], to parse [`SCENARIO_BASE`].
#[derive(Debug, Parser)]
struct ConfigArgs {
    #[command(flatten)]
    cfg: SimConfig,
}

fn scenario_base() -> SimConfig {
    ConfigArgs::parse_from(SCENARIO_BASE).cfg
}

fn scenarios(seeds: &[u64], real_net: bool) -> (String, Vec<ScenarioRun>) {
    let mut base = scenario_base();
    base.real_net = real_net;
    let scenarios = [
        Scenario {
            title: "baseline",
            note: "n = 4, 2000 tx/s, E = 20 ms, d = 5 ms",
            apply: |_| {},
        },
        Scenario { title: "high load", note: "4000 tx/s", apply: |c| c.tps = 4000.0 },
        Scenario {
            title: "slow execution",
            note: "E = 100 ms, longer than the block interval",
            apply: |c| c.exec_block_ms = Some(100.0),
        },
        Scenario { title: "slow network", note: "d = 20 ms", apply: |c| c.latency_ms = Some(20.0) },
        Scenario {
            title: "one validator down",
            note: "validator 1 neither proposes nor votes",
            apply: |c| c.crashed = vec![1],
        },
        Scenario {
            title: "one validator executes wrongly",
            note: "validator 3 reports a wrong state root",
            apply: |c| c.bad_exec = vec![3],
        },
    ];
    let variants = [
        Variant { label: "Parlia, pre-Fermi", which: Which::Parlia, apply: |_| {} },
        Variant { label: "Parlia, Fermi", which: Which::Parlia, apply: |c| c.attest_depth = 3 },
        Variant { label: "DX, QC broadcast", which: Which::Dx, apply: |c| c.qc_broadcast = true },
        Variant { label: "DX, QC in next proposal", which: Which::Dx, apply: |_| {} },
    ];

    let crypto = CryptoCosts::calibrate(base.quorum());
    let mut runs = Vec::new();
    let mut md = String::from("# dx-demo fault and load scenarios\n\n");
    md.push_str(SCENARIO_LEGEND);
    md.push_str(
        "| scenario | protocol | blocks/s (range) | tx/s | critical B | state gap p50 | \
         2/3 signed state p50 | DX QC p50 | irreversible p50 (range) | irreversible p99 | \
         faults | safe |\n",
    );
    md.push_str("| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |\n");
    for s in &scenarios {
        for variant in &variants {
            let mut reports: Vec<(u64, Report)> = seeds
                .iter()
                .map(|&seed| {
                    let mut cfg = base.clone();
                    cfg.seed = seed;
                    (s.apply)(&mut cfg);
                    (variant.apply)(&mut cfg);
                    eprintln!("== {} / {} / seed {seed}", s.title, variant.label);
                    let report = run_scenario(&cfg, &crypto, variant.which).remove(0);
                    (seed, report)
                })
                .collect();
            reports.sort_by(|a, b| a.1.block.blocks_per_s.total_cmp(&b.1.block.blocks_per_s));
            let range = |f: fn(&Report) -> f64| {
                let v: Vec<f64> = reports.iter().map(|(_, r)| f(r)).collect();
                let lo = v.iter().copied().fold(f64::INFINITY, f64::min);
                let hi = v.iter().copied().fold(f64::NEG_INFINITY, f64::max);
                format!("{lo:.1}–{hi:.1}")
            };
            let bps = range(|r| r.block.blocks_per_s);
            let irr = range(|r| r.block.irreversible_p50_ms);
            let b = &reports[reports.len() / 2].1.block;
            md.push_str(&format!(
                "| {} ({}) | {} | {:.1} ({bps}) | {:.0} | {:.0} | {:.1} | {:.1} | {:.1} | {:.1} ({irr}) | {:.0} | {} | {} |\n",
                s.title,
                s.note,
                variant.label,
                b.blocks_per_s,
                b.tps,
                b.critical_bytes,
                b.state_gap_p50_ms,
                b.quorum_sig_p50_ms,
                b.order_qc_p50_ms,
                b.irreversible_p50_ms,
                b.irreversible_p99_ms,
                b.faults,
                if b.safe { "yes" } else { "NO" },
            ));
            runs.extend(reports.into_iter().map(|(seed, report)| ScenarioRun {
                scenario: s.title,
                variant: variant.label,
                seed,
                report,
            }));
        }
    }
    md.push_str("\n## Configuration\n\n```json\n");
    md.push_str(&serde_json::to_string_pretty(&base).expect("serializable"));
    md.push_str("\n```\n\n## Measured signature costs (µs)\n\n```json\n");
    md.push_str(&serde_json::to_string_pretty(&crypto).expect("serializable"));
    md.push_str("\n```\n");
    (md, runs)
}

/// Explains the columns of the `scenarios` table.
const SCENARIO_LEGEND: &str = "Each row is the run with the median block rate out of the seeds; \
ranges are over all seeds. Latencies are in ms and measured from the moment a block leaves its \
proposer, at the median honest validator. `critical B` is the mean size of the block message \
(Parlia: full block, rejected blocks included; DX: proposal). `state gap` is how much later an \
honest validator has the block's execution result than its proposer. `2/3 signed state` is when \
2/3 have signed the execution result (Parlia: votes; DX: ExecCert); `DX QC` is when the order \
QC forms. `irreversible` is Parlia: block and child both have 2/3 votes; DX: committed and \
ExecCert. `faults` counts Parlia blocks produced after a backoff and DX rounds ended by a TC.\n\n";

fn sweep(base: &SimConfig) -> String {
    let scenarios = [
        Scenario {
            title: "S1 2000 tx/s, in-memory revm execution",
            note: "Light load; execution cost as measured with in-memory revm.",
            apply: |_| {},
        },
        Scenario {
            title: "S2 2000 tx/s, 100 µs per tx",
            note: "Execution scaled to roughly mainnet cost per transaction.",
            apply: |c| c.exec_us_per_tx = Some(100.0),
        },
        Scenario {
            title: "S3 5000 tx/s, 100 µs per tx",
            note: "Execution takes about a third of each 450 ms block.",
            apply: |c| {
                c.tps = 5000.0;
                c.exec_us_per_tx = Some(100.0);
            },
        },
        Scenario {
            title: "S4 8000 tx/s, 100 µs per tx, 300M gas limit",
            note: "Execution takes about 60% of each block; the gas limit is raised so it is not the bound.",
            apply: |c| {
                c.tps = 8000.0;
                c.exec_us_per_tx = Some(100.0);
                c.gas_limit = 300_000_000;
            },
        },
        Scenario {
            title: "S5 11000 tx/s, 100 µs per tx, 600M gas limit",
            note: "Beyond what one execution thread can do (10000 tx/s at 100 µs); both protocols must saturate.",
            apply: |c| {
                c.tps = 11000.0;
                c.exec_us_per_tx = Some(100.0);
                c.gas_limit = 600_000_000;
            },
        },
        Scenario {
            title: "S6 2000 tx/s, 100 µs per tx, DX T_minInterval 150 ms",
            note: "Parlia keeps 450 ms blocks; DX lowers its minimum proposal interval.",
            apply: |c| {
                c.exec_us_per_tx = Some(100.0);
                c.min_interval_ms = 150;
            },
        },
        Scenario {
            title: "S7 2000 tx/s, 100 µs per tx, DX T_minInterval 50 ms",
            note: "DX rounds advance as soon as a QC forms.",
            apply: |c| {
                c.exec_us_per_tx = Some(100.0);
                c.min_interval_ms = 50;
            },
        },
        Scenario {
            title: "S8 8000 tx/s, 100 µs per tx, 300M gas limit, DX T_minInterval 50 ms",
            note: "Heavy load with QC-driven DX rounds.",
            apply: |c| {
                c.tps = 8000.0;
                c.exec_us_per_tx = Some(100.0);
                c.gas_limit = 300_000_000;
                c.min_interval_ms = 50;
            },
        },
        Scenario {
            title: "S9 5000 tx/s, 100 µs per tx, 200 Mbit/s uplinks",
            note: "Constrained validator bandwidth.",
            apply: |c| {
                c.tps = 5000.0;
                c.exec_us_per_tx = Some(100.0);
                c.bandwidth_mbps = 200.0;
            },
        },
        Scenario {
            title: "S10 5000 tx/s, 100 µs per tx, both at 250 ms",
            note: "Parlia block interval and DX T_minInterval both 250 ms, gas limit scaled to 56M.",
            apply: |c| {
                c.tps = 5000.0;
                c.exec_us_per_tx = Some(100.0);
                c.block_interval_ms = 250;
                c.min_interval_ms = 250;
                c.gas_limit = 56_000_000;
            },
        },
        Scenario {
            title: "S11 2000 tx/s, 100 µs per tx, 21 validators, DX T_minInterval 50 ms",
            note: "Smaller validator set.",
            apply: |c| {
                c.validators = 21;
                c.exec_us_per_tx = Some(100.0);
                c.min_interval_ms = 50;
            },
        },
    ];

    let crypto = CryptoCosts::calibrate(base.quorum());
    let mut md = String::from("# dx-demo results\n\n");
    md.push_str(LEGEND);
    md.push_str("## Measured signature costs (µs)\n\n```json\n");
    md.push_str(&serde_json::to_string_pretty(&crypto).expect("serializable"));
    md.push_str("\n```\n\n");
    md.push_str("## Base configuration\n\n```json\n");
    md.push_str(&serde_json::to_string_pretty(base).expect("serializable"));
    md.push_str("\n```\n\n");
    for s in scenarios {
        let mut cfg = base.clone();
        (s.apply)(&mut cfg);
        eprintln!("== {}", s.title);
        let exec = ExecCostModel::calibrate(&cfg);
        let crypto = if cfg.validators == base.validators {
            crypto.clone()
        } else {
            CryptoCosts::calibrate(cfg.quorum())
        };
        let reports = run_scenario(&cfg, &crypto, Which::Both);
        md.push_str(&format!("## {}\n\n{}\n\n", s.title, s.note));
        md.push_str(&format!(
            "Execution cost charged: transfer {:.1} µs, ERC20 {:.1} µs (in-memory revm measured {:.1} / {:.1} µs).\n\n",
            exec.transfer_us, exec.erc20_us, exec.measured_transfer_us, exec.measured_erc20_us
        ));
        md.push_str(&Report::table(&reports));
        md.push('\n');
    }
    md
}

#[cfg(test)]
mod tests {
    use super::*;

    fn small(extra: &[&str]) -> SimConfig {
        let mut argv = vec![
            "dx-demo",
            "--validators",
            "7",
            "--duration-s",
            "6",
            "--warmup-s",
            "1",
            "--tps",
            "300",
            "--accounts",
            "2000",
        ];
        argv.extend_from_slice(extra);
        ConfigArgs::parse_from(argv).cfg
    }

    /// The `scenarios` base configuration, shortened and at a lighter load to keep tests fast.
    fn fault_base() -> SimConfig {
        let mut cfg = scenario_base();
        cfg.tps = 500.0;
        cfg.accounts = 2000;
        cfg.duration_s = 5.0;
        cfg
    }

    #[test]
    fn dx_skips_a_crashed_leader_while_parlia_waits_for_backoff() {
        let mut cfg = fault_base();
        cfg.crashed = vec![1];
        let crypto = CryptoCosts::calibrate(cfg.quorum());
        let parlia = &run_scenario(&cfg, &crypto, Which::Parlia)[0].block;
        assert!(parlia.blocks_per_s < 5.0, "{}", parlia.blocks_per_s);
        assert!(parlia.faults > 0);
        for qc_broadcast in [true, false] {
            cfg.qc_broadcast = qc_broadcast;
            let dx = &run_scenario(&cfg, &crypto, Which::Dx)[0].block;
            assert!(dx.safe);
            assert!(dx.faults > 0, "qc_broadcast {qc_broadcast}");
            assert!(dx.blocks_per_s > 10.0, "qc_broadcast {qc_broadcast}: {}", dx.blocks_per_s);
        }
    }

    #[test]
    fn a_wrong_state_root_stalls_parlia_but_not_dx_ordering() {
        let mut cfg = fault_base();
        cfg.bad_exec = vec![3];
        cfg.qc_broadcast = true;
        let crypto = CryptoCosts::calibrate(cfg.quorum());
        let reports = run_scenario(&cfg, &crypto, Which::Both);
        let (parlia, dx) = (&reports[0].block, &reports[1].block);
        assert!(parlia.blocks_per_s < 5.0, "{}", parlia.blocks_per_s);
        assert!(dx.safe);
        assert_eq!(dx.faults, 0);
        assert!(dx.blocks_per_s > 19.0, "{}", dx.blocks_per_s);
        assert!(dx.irreversible_p50_ms < 100.0, "{}", dx.irreversible_p50_ms);
    }

    #[test]
    fn dx_execution_backpressure_bounds_the_block_rate() {
        let mut cfg = fault_base();
        cfg.exec_block_ms = Some(100.0);
        let crypto = CryptoCosts::calibrate(cfg.quorum());
        let dx = &run_scenario(&cfg, &crypto, Which::Dx)[0].block;
        // k = 3 blocks may be unexecuted, so 100 ms blocks allow at most 10 per second.
        assert!((dx.blocks_per_s - 10.0).abs() < 0.5, "{}", dx.blocks_per_s);
    }

    #[test]
    fn both_protocols_finalize_every_transaction_under_light_load() {
        let cfg = small(&[]);
        let crypto = CryptoCosts::calibrate(cfg.quorum());
        for report in run_scenario(&cfg, &crypto, Which::Both) {
            assert!(report.measured_txs > 1000, "{}", report.protocol);
            assert_eq!(report.finalized_ratio, 1.0, "{}", report.protocol);
            let failed =
                report.extra.iter().find(|(k, _)| k == "failed txs").map(|(_, v)| v.as_str());
            assert_eq!(failed, Some("0"), "{}", report.protocol);
            assert!(report.finalized.p50 <= report.finalized.p95, "{}", report.protocol);
        }
    }

    #[test]
    fn streamed_parlia_blocks_are_imported_sooner_after_the_seal() {
        let base = ["--exec-us-per-tx", "1000", "--attest-depth", "3"];
        let crypto = CryptoCosts::calibrate(small(&base).quorum());
        let run = |extra: &[&str]| {
            let cfg = small(&[&base[..], extra].concat());
            run_scenario(&cfg, &crypto, Which::Parlia).remove(0)
        };
        let streamed_share = |r: &Report| {
            r.extra
                .iter()
                .find(|(k, _)| k == "txs streamed before the seal")
                .map(|(_, v)| v.clone())
        };
        let plain = run(&[]);
        let streamed = run(&["--stream-chunk-ms", "50"]);
        assert_eq!(streamed.finalized_ratio, 1.0);
        assert!(
            streamed.block.state_gap_p50_ms + 50.0 < plain.block.state_gap_p50_ms,
            "{} vs {}",
            streamed.block.state_gap_p50_ms,
            plain.block.state_gap_p50_ms
        );
        assert!(streamed.finalized.p50 < plain.finalized.p50);
        // A bid chosen at the last moment heads the block, so nothing can be streamed before it.
        let unlocked = run(&["--stream-chunk-ms", "50", "--bid-share", "0.9"]);
        assert_eq!(streamed_share(&unlocked).as_deref(), Some("0.0%"));
        let locked = run(&["--stream-chunk-ms", "50", "--bid-share", "0.9", "--lock-ms", "150"]);
        assert!(locked.block.state_gap_p50_ms < unlocked.block.state_gap_p50_ms);
        // A bid locked at the stream's pace imports no later than a block without one.
        let paced = run(&["--stream-chunk-ms", "50", "--bid-share", "0.9", "--bid-stream"]);
        assert!(paced.block.state_gap_p50_ms <= streamed.block.state_gap_p50_ms);
    }

    #[test]
    fn streamed_builder_chunks_keep_dx_proposals_small() {
        let base =
            ["--min-interval-ms", "50", "--bandwidth-mbps", "20", "--dx-builder-share", "0.9"];
        let crypto = CryptoCosts::calibrate(small(&base).quorum());
        let run = |extra: &[&str]| {
            let cfg = small(&[&base[..], extra].concat());
            run_scenario(&cfg, &crypto, Which::Dx).remove(0)
        };
        let inline = run(&[]);
        let chunked = run(&["--dx-builder-chunk-ms", "25"]);
        assert_eq!(inline.finalized_ratio, 1.0);
        assert_eq!(chunked.finalized_ratio, 1.0);
        assert!(chunked.block.safe);
        assert!(
            chunked.block.critical_bytes * 2.0 < inline.block.critical_bytes,
            "{} vs {}",
            chunked.block.critical_bytes,
            inline.block.critical_bytes
        );
        assert!(chunked.block.blocks_per_s >= inline.block.blocks_per_s);
    }

    #[test]
    fn dx_rounds_follow_the_minimum_interval() {
        let crypto = CryptoCosts::calibrate(small(&[]).quorum());
        let slow = &run_scenario(&small(&["--min-interval-ms", "450"]), &crypto, Which::Dx)[0];
        let fast = &run_scenario(&small(&["--min-interval-ms", "50"]), &crypto, Which::Dx)[0];
        assert!((slow.block_interval_ms - 450.0).abs() < 5.0, "{}", slow.block_interval_ms);
        assert!(fast.block_interval_ms < 450.0);
        assert!(fast.finalized.p50 < slow.finalized.p50);
    }
}
