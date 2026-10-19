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
        Cmd::Sweep { cfg, out } => {
            let md = sweep(&cfg);
            std::fs::write(&out, md).expect("writable output");
            println!("wrote {}", out.display());
        }
    }
}

fn run_scenario(cfg: &SimConfig, crypto: &CryptoCosts, which: Which) -> Vec<Report> {
    let topo = Topology::new(cfg.validators, cfg.seed);
    let workload = Workload::generate(cfg, &topo);
    let exec_model = ExecCostModel::calibrate(cfg);
    let env = Env {
        cfg,
        workload: &workload,
        topo: &topo,
        crypto,
        exec_model: &exec_model,
        drain_s: DRAIN_S,
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

    #[derive(Debug, Parser)]
    struct Args {
        #[command(flatten)]
        cfg: SimConfig,
    }

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
        Args::parse_from(argv).cfg
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
    fn dx_rounds_follow_the_minimum_interval() {
        let crypto = CryptoCosts::calibrate(small(&[]).quorum());
        let slow = &run_scenario(&small(&["--min-interval-ms", "450"]), &crypto, Which::Dx)[0];
        let fast = &run_scenario(&small(&["--min-interval-ms", "50"]), &crypto, Which::Dx)[0];
        assert!((slow.block_interval_ms - 450.0).abs() < 5.0, "{}", slow.block_interval_ms);
        assert!(fast.block_interval_ms < 450.0);
        assert!(fast.finalized.p50 < slow.finalized.p50);
    }
}
