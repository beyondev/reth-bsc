# dx-demo fault and load scenarios (real network, `--real-net`)

Each row is the run with the median block rate out of the seeds; ranges are over all seeds. Latencies are in ms and measured from the moment a block leaves its proposer, at the median honest validator. `critical B` is the mean size of the block message (Parlia: full block, rejected blocks included; DX: proposal). `state gap` is how much later an honest validator has the block's execution result than its proposer. `2/3 signed state` is when 2/3 have signed the execution result (Parlia: votes; DX: ExecCert); `DX QC` is when the order QC forms. `irreversible` is Parlia: block and child both have 2/3 votes; DX: committed and ExecCert. `faults` counts Parlia blocks produced after a backoff and DX rounds ended by a TC.

| scenario | protocol | blocks/s (range) | tx/s | critical B | state gap p50 | 2/3 signed state p50 | DX QC p50 | irreversible p50 (range) | irreversible p99 | faults | safe |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| baseline (n = 4, 2000 tx/s, E = 20 ms, d = 5 ms) | Parlia, pre-Fermi | 20.0 (20.0–20.0) | 2000 | 15181 | 26.4 | 33.0 | NaN | 83.1 (83.0–83.1) | 86 | 0 | yes |
| baseline (n = 4, 2000 tx/s, E = 20 ms, d = 5 ms) | Parlia, Fermi | 20.0 (20.0–20.0) | 2000 | 15181 | 26.4 | 33.0 | NaN | 83.2 (83.0–83.2) | 86 | 0 | yes |
| baseline (n = 4, 2000 tx/s, E = 20 ms, d = 5 ms) | DX, QC broadcast | 20.0 (20.0–20.0) | 1996 | 1240 | 8.7 | 78.6 | 15.7 | 78.6 (78.6–78.7) | 82 | 0 | yes |
| baseline (n = 4, 2000 tx/s, E = 20 ms, d = 5 ms) | DX, QC in next proposal | 20.0 (20.0–20.0) | 1996 | 1240 | 8.5 | 115.5 | 15.8 | 115.5 (115.5–115.6) | 118 | 0 | yes |
| high load (4000 tx/s) | Parlia, pre-Fermi | 20.0 (20.0–20.0) | 4006 | 29768 | 26.5 | 33.0 | NaN | 83.1 (82.9–83.2) | 85 | 0 | yes |
| high load (4000 tx/s) | Parlia, Fermi | 20.0 (20.0–20.0) | 4006 | 29769 | 26.4 | 33.0 | NaN | 83.1 (83.0–83.2) | 85 | 0 | yes |
| high load (4000 tx/s) | DX, QC broadcast | 20.0 (20.0–20.0) | 4000 | 1240 | 8.8 | 78.5 | 15.7 | 78.5 (78.5–78.7) | 82 | 0 | yes |
| high load (4000 tx/s) | DX, QC in next proposal | 20.0 (20.0–20.0) | 4000 | 1240 | 8.5 | 115.6 | 15.9 | 115.6 (115.6–115.7) | 118 | 0 | yes |
| slow execution (E = 100 ms, longer than the block interval) | Parlia, pre-Fermi | 8.0 (8.0–8.0) | 2032 | 37492 | 107.0 | 115.0 | NaN | 215.6 (215.0–215.6) | 328 | 0 | yes |
| slow execution (E = 100 ms, longer than the block interval) | Parlia, Fermi | 8.0 (8.0–8.0) | 2032 | 37497 | 107.1 | 115.0 | NaN | 215.4 (215.4–215.6) | 328 | 0 | yes |
| slow execution (E = 100 ms, longer than the block interval) | DX, QC broadcast | 10.0 (10.0–10.0) | 1992 | 1578 | 0.0 | 300.3 | 15.9 | 300.3 (300.2–300.8) | 303 | 0 | yes |
| slow execution (E = 100 ms, longer than the block interval) | DX, QC in next proposal | 10.0 (10.0–10.0) | 1992 | 1578 | 0.0 | 300.4 | 15.8 | 300.4 (300.4–300.5) | 303 | 0 | yes |
| slow network (d = 20 ms) | Parlia, pre-Fermi | 18.6 (18.5–18.6) | 2011 | 16354 | 41.3 | 63.0 | NaN | 113.5 (113.5–113.7) | 145 | 0 | yes |
| slow network (d = 20 ms) | Parlia, Fermi | 18.6 (18.5–18.6) | 2011 | 16351 | 41.3 | 63.0 | NaN | 113.5 (113.5–113.6) | 145 | 0 | yes |
| slow network (d = 20 ms) | DX, QC broadcast | 20.0 (20.0–20.0) | 1992 | 1239 | 23.6 | 138.3 | 45.6 | 138.3 (138.3–138.4) | 142 | 0 | yes |
| slow network (d = 20 ms) | DX, QC in next proposal | 20.0 (20.0–20.0) | 1992 | 1239 | 23.7 | 145.6 | 45.8 | 145.6 (145.6–145.9) | 148 | 0 | yes |
| one validator down (validator 1 neither proposes nor votes) | Parlia, pre-Fermi | 3.3 (3.3–3.3) | 2171 | 95982 | 25.8 | 33.4 | NaN | 83.6 (83.6–83.8) | 1110 | 9 | yes |
| one validator down (validator 1 neither proposes nor votes) | Parlia, Fermi | 3.3 (3.3–3.3) | 2171 | 95982 | 25.9 | 33.1 | NaN | 83.5 (83.5–83.9) | 1108 | 9 | yes |
| one validator down (validator 1 neither proposes nor votes) | DX, QC broadcast | 12.4 (12.2–12.4) | 1937 | 1316 | 7.7 | 80.8 | 16.8 | 80.8 (80.8–81.2) | 449 | 24 | yes |
| one validator down (validator 1 neither proposes nor votes) | DX, QC in next proposal | 11.1 (11.1–11.2) | 1950 | 1359 | 9.2 | 116.9 | 17.0 | 116.9 (116.9–117.3) | 594 | 20 | yes |
| one validator executes wrongly (validator 3 reports a wrong state root) | Parlia, pre-Fermi | 3.2 (3.2–3.2) | 1955 | 76088 | 26.6 | 33.5 | NaN | 83.6 (83.2–83.6) | 1109 | 8 | yes |
| one validator executes wrongly (validator 3 reports a wrong state root) | Parlia, Fermi | 3.2 (3.2–3.2) | 1955 | 76102 | 26.6 | 33.7 | NaN | 83.7 (83.1–83.7) | 1109 | 8 | yes |
| one validator executes wrongly (validator 3 reports a wrong state root) | DX, QC broadcast | 20.0 (20.0–20.0) | 1996 | 1240 | 8.7 | 79.4 | 15.7 | 79.4 (79.4–79.4) | 83 | 0 | yes |
| one validator executes wrongly (validator 3 reports a wrong state root) | DX, QC in next proposal | 20.0 (20.0–20.0) | 1996 | 1240 | 8.5 | 116.4 | 15.8 | 116.4 (116.3–116.5) | 119 | 0 | yes |

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
  "real_net": true,
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
  "ed_sign": 24.620035,
  "ed_verify": 51.62950000000001,
  "ed_verify_batched": 26.90853046875,
  "bls_sign": 497.81878000000006,
  "bls_verify": 1251.30174,
  "bls_agg_verify": 1057.709,
  "bls_agg_add": 1.3966,
  "ecdsa_sign": 44.17678000000001,
  "ecdsa_recover": 52.867914999999996
}
```
