# dx-demo fault and load scenarios

Each row is the run with the median block rate out of the seeds; ranges are over all seeds. Latencies are in ms and measured from the moment a block leaves its proposer, at the median honest validator. `critical B` is the mean size of the block message (Parlia: full block, rejected blocks included; DX: proposal). `state gap` is how much later an honest validator has the block's execution result than its proposer. `2/3 signed state` is when 2/3 have signed the execution result (Parlia: votes; DX: ExecCert); `DX QC` is when the order QC forms. `irreversible` is Parlia: block and child both have 2/3 votes; DX: committed and ExecCert. `faults` counts Parlia blocks produced after a backoff and DX rounds ended by a TC.

| scenario | protocol | blocks/s (range) | tx/s | critical B | state gap p50 | 2/3 signed state p50 | DX QC p50 | irreversible p50 (range) | irreversible p99 | faults | safe |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| baseline (n = 4, 2000 tx/s, E = 20 ms, d = 5 ms) | Parlia, pre-Fermi | 20.0 (20.0–20.0) | 1998 | 15170 | 26.2 | 32.0 | NaN | 82.1 (82.0–82.2) | 85 | 0 | yes |
| baseline (n = 4, 2000 tx/s, E = 20 ms, d = 5 ms) | Parlia, Fermi | 20.0 (20.0–20.0) | 1998 | 15170 | 26.2 | 32.0 | NaN | 82.1 (82.0–82.2) | 85 | 0 | yes |
| baseline (n = 4, 2000 tx/s, E = 20 ms, d = 5 ms) | DX, QC broadcast | 20.0 (20.0–20.0) | 1996 | 1240 | 8.1 | 76.8 | 14.4 | 76.8 (76.8–77.0) | 80 | 0 | yes |
| baseline (n = 4, 2000 tx/s, E = 20 ms, d = 5 ms) | DX, QC in next proposal | 20.0 (20.0–20.0) | 1996 | 1240 | 7.8 | 114.2 | 14.5 | 114.2 (114.2–114.3) | 117 | 0 | yes |
| high load (4000 tx/s) | Parlia, pre-Fermi | 20.0 (20.0–20.0) | 4004 | 29756 | 26.4 | 32.2 | NaN | 82.3 (82.2–82.4) | 85 | 0 | yes |
| high load (4000 tx/s) | Parlia, Fermi | 20.0 (20.0–20.0) | 4004 | 29756 | 26.4 | 32.2 | NaN | 82.3 (82.2–82.4) | 85 | 0 | yes |
| high load (4000 tx/s) | DX, QC broadcast | 20.0 (20.0–20.0) | 4000 | 1240 | 8.1 | 76.8 | 14.4 | 76.8 (76.8–77.0) | 80 | 0 | yes |
| high load (4000 tx/s) | DX, QC in next proposal | 20.0 (20.0–20.0) | 4000 | 1240 | 7.8 | 114.3 | 14.5 | 114.3 (114.3–114.3) | 117 | 0 | yes |
| slow execution (E = 100 ms, longer than the block interval) | Parlia, pre-Fermi | 8.0 (8.0–8.0) | 2032 | 37492 | 107.1 | 114.1 | NaN | 214.4 (214.1–214.6) | 327 | 0 | yes |
| slow execution (E = 100 ms, longer than the block interval) | Parlia, Fermi | 8.0 (8.0–8.0) | 2032 | 37492 | 107.1 | 114.1 | NaN | 214.4 (214.1–214.6) | 327 | 0 | yes |
| slow execution (E = 100 ms, longer than the block interval) | DX, QC broadcast | 10.0 (10.0–10.0) | 1992 | 1578 | 0.0 | 300.5 | 14.5 | 300.5 (300.1–300.8) | 303 | 0 | yes |
| slow execution (E = 100 ms, longer than the block interval) | DX, QC in next proposal | 10.0 (10.0–10.0) | 1997 | 1580 | 0.0 | 300.4 | 14.3 | 300.4 (300.3–300.8) | 303 | 0 | yes |
| slow network (d = 20 ms) | Parlia, pre-Fermi | 18.6 (18.6–18.6) | 2006 | 16314 | 41.1 | 62.1 | NaN | 112.6 (112.6–112.7) | 143 | 0 | yes |
| slow network (d = 20 ms) | Parlia, Fermi | 18.6 (18.6–18.6) | 2006 | 16314 | 41.1 | 62.1 | NaN | 112.6 (112.6–112.7) | 143 | 0 | yes |
| slow network (d = 20 ms) | DX, QC broadcast | 20.0 (20.0–20.0) | 1992 | 1239 | 22.9 | 136.6 | 44.4 | 136.6 (136.6–136.9) | 140 | 0 | yes |
| slow network (d = 20 ms) | DX, QC in next proposal | 20.0 (20.0–20.0) | 1992 | 1239 | 23.0 | 144.3 | 44.4 | 144.3 (144.3–144.5) | 146 | 0 | yes |
| one validator down (validator 1 neither proposes nor votes) | Parlia, pre-Fermi | 3.3 (3.3–3.3) | 2171 | 95982 | 25.6 | 32.3 | NaN | 82.4 (82.4–82.9) | 1109 | 9 | yes |
| one validator down (validator 1 neither proposes nor votes) | Parlia, Fermi | 3.3 (3.3–3.3) | 2171 | 95982 | 25.6 | 32.3 | NaN | 82.4 (82.4–82.9) | 1109 | 9 | yes |
| one validator down (validator 1 neither proposes nor votes) | DX, QC broadcast | 12.4 (12.4–12.4) | 1932 | 1317 | 7.4 | 79.2 | 15.7 | 79.2 (78.9–79.2) | 447 | 24 | yes |
| one validator down (validator 1 neither proposes nor votes) | DX, QC in next proposal | 11.2 (11.2–11.2) | 1948 | 1358 | 8.8 | 115.8 | 15.8 | 115.8 (115.5–115.8) | 589 | 20 | yes |
| one validator executes wrongly (validator 3 reports a wrong state root) | Parlia, pre-Fermi | 3.2 (3.2–3.2) | 1955 | 76068 | 26.3 | 32.5 | NaN | 82.5 (82.3–82.5) | 1108 | 8 | yes |
| one validator executes wrongly (validator 3 reports a wrong state root) | Parlia, Fermi | 3.2 (3.2–3.2) | 1955 | 76068 | 26.3 | 32.5 | NaN | 82.5 (82.3–82.5) | 1108 | 8 | yes |
| one validator executes wrongly (validator 3 reports a wrong state root) | DX, QC broadcast | 20.0 (20.0–20.0) | 1996 | 1240 | 8.0 | 77.5 | 14.4 | 77.5 (77.5–77.6) | 81 | 0 | yes |
| one validator executes wrongly (validator 3 reports a wrong state root) | DX, QC in next proposal | 20.0 (20.0–20.0) | 1996 | 1240 | 7.8 | 115.1 | 14.5 | 115.1 (115.0–115.3) | 118 | 0 | yes |

## Configuration

```json
{
  "validators": 4,
  "duration_s": 11.0,
  "warmup_s": 1.0,
  "tps": 2000.0,
  "erc20_share": 0.5,
  "accounts": 50000,
  "bandwidth_mbps": 1000.0,
  "sig_cores": 8,
  "exec_us_per_tx": null,
  "block_overhead_ms": 1.0,
  "seed": 7,
  "latency_ms": 5.0,
  "jitter_ms": 2.0,
  "exec_block_ms": 20.0,
  "crashed": [],
  "bad_exec": [],
  "real_net": false,
  "block_interval_ms": 50,
  "turn_length": 4,
  "gas_limit": 100000000,
  "miner_reserve_ms": 120,
  "parlia_direct": false,
  "attest_depth": 1,
  "multi_attest": false,
  "compact_tx_bytes": null,
  "backoff_ms": 1000,
  "t_batch_ms": 100,
  "s_batch": 262144,
  "min_interval_ms": 50,
  "exec_lag": 3,
  "speculative": true,
  "round_timeout_ms": 100,
  "fallback_ms": 2000,
  "dx_inline_share": 0.0,
  "qc_broadcast": false
}
```

## Measured signature costs (µs)

```json
{
  "ed_sign": 17.362720000000003,
  "ed_verify": 36.80019,
  "ed_verify_batched": 20.272677343749997,
  "bls_sign": 375.58227999999997,
  "bls_verify": 968.63246,
  "bls_agg_verify": 953.8976333333334,
  "bls_agg_add": 0.9213666666666667,
  "ecdsa_sign": 25.231835,
  "ecdsa_recover": 34.442835
}
```
