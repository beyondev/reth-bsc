# dx-demo fault and load scenarios

Each row is the run with the median block rate out of the seeds; ranges are over all seeds. Latencies are in ms and measured from the moment a block leaves its proposer, at the median honest validator. `critical B` is the mean size of the block message (Parlia: full block, rejected blocks included; DX: proposal). `state gap` is how much later an honest validator has the block's execution result than its proposer. `2/3 signed state` is when 2/3 have signed the execution result (Parlia: votes; DX: ExecCert); `DX QC` is when the order QC forms. `irreversible` is Parlia: block and child both have 2/3 votes; DX: committed and ExecCert. `faults` counts Parlia blocks produced after a backoff and DX rounds ended by a TC.

| scenario | protocol | blocks/s (range) | tx/s | critical B | state gap p50 | 2/3 signed state p50 | DX QC p50 | irreversible p50 (range) | irreversible p99 | faults | safe |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| baseline (n = 4, 2000 tx/s, E = 20 ms, d = 5 ms) | Parlia | 20.0 (20.0–20.0) | 1999 | 15174 | 26.2 | 32.6 | NaN | 82.6 (82.6–82.7) | 85 | 0 | yes |
| baseline (n = 4, 2000 tx/s, E = 20 ms, d = 5 ms) | DX, QC broadcast | 20.0 (20.0–20.0) | 1996 | 1240 | 9.8 | 80.8 | 16.9 | 80.8 (80.8–81.0) | 84 | 0 | yes |
| baseline (n = 4, 2000 tx/s, E = 20 ms, d = 5 ms) | DX, QC in next proposal | 20.0 (20.0–20.0) | 1996 | 1240 | 9.5 | 116.7 | 17.0 | 116.7 (116.7–116.8) | 119 | 0 | yes |
| high load (4000 tx/s) | Parlia | 20.0 (20.0–20.0) | 4005 | 29765 | 26.4 | 32.7 | NaN | 82.8 (82.8–82.9) | 85 | 0 | yes |
| high load (4000 tx/s) | DX, QC broadcast | 20.0 (20.0–20.0) | 4000 | 1240 | 9.8 | 80.9 | 17.0 | 80.9 (80.9–81.1) | 84 | 0 | yes |
| high load (4000 tx/s) | DX, QC in next proposal | 20.0 (20.0–20.0) | 4000 | 1240 | 9.5 | 116.8 | 17.1 | 116.8 (116.8–116.9) | 119 | 0 | yes |
| slow execution (E = 100 ms, longer than the block interval) | Parlia | 8.0 (8.0–8.0) | 2032 | 37492 | 107.1 | 114.7 | NaN | 215.1 (214.7–215.2) | 328 | 0 | yes |
| slow execution (E = 100 ms, longer than the block interval) | DX, QC broadcast | 10.0 (10.0–10.0) | 1992 | 1578 | 0.0 | 300.2 | 17.0 | 300.2 (300.2–300.6) | 303 | 0 | yes |
| slow execution (E = 100 ms, longer than the block interval) | DX, QC in next proposal | 10.0 (10.0–10.0) | 1992 | 1578 | 0.0 | 300.3 | 16.9 | 300.3 (300.2–300.4) | 303 | 0 | yes |
| slow network (d = 20 ms) | Parlia | 18.6 (18.6–18.6) | 2007 | 16323 | 41.1 | 62.6 | NaN | 113.2 (113.1–113.2) | 144 | 0 | yes |
| slow network (d = 20 ms) | DX, QC broadcast | 20.0 (20.0–20.0) | 1992 | 1239 | 24.6 | 140.6 | 47.0 | 140.6 (140.6–140.9) | 145 | 0 | yes |
| slow network (d = 20 ms) | DX, QC in next proposal | 20.0 (20.0–20.0) | 1992 | 1239 | 24.7 | 146.8 | 47.0 | 146.8 (146.8–147.1) | 149 | 0 | yes |
| one validator down (validator 1 neither proposes nor votes) | Parlia | 3.3 (3.3–3.3) | 2171 | 95991 | 25.6 | 32.9 | NaN | 83.0 (83.0–83.4) | 1110 | 9 | yes |
| one validator down (validator 1 neither proposes nor votes) | DX, QC broadcast | 12.4 (12.4–12.4) | 1932 | 1317 | 8.8 | 83.1 | 18.0 | 83.1 (82.8–83.3) | 454 | 24 | yes |
| one validator down (validator 1 neither proposes nor votes) | DX, QC in next proposal | 11.1 (11.1–11.1) | 1942 | 1355 | 10.5 | 118.0 | 18.1 | 118.0 (118.0–118.3) | 594 | 20 | yes |
| one validator executes wrongly (validator 3 reports a wrong state root) | Parlia | 3.2 (3.2–3.2) | 1955 | 76088 | 26.3 | 33.0 | NaN | 83.0 (82.8–83.0) | 1109 | 8 | yes |
| one validator executes wrongly (validator 3 reports a wrong state root) | DX, QC broadcast | 20.0 (20.0–20.0) | 1996 | 1240 | 9.7 | 81.5 | 16.9 | 81.5 (81.5–81.7) | 85 | 0 | yes |
| one validator executes wrongly (validator 3 reports a wrong state root) | DX, QC in next proposal | 20.0 (20.0–20.0) | 1996 | 1240 | 9.5 | 117.6 | 17.0 | 117.6 (117.6–117.8) | 121 | 0 | yes |

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
  "block_interval_ms": 50,
  "turn_length": 4,
  "gas_limit": 100000000,
  "miner_reserve_ms": 120,
  "parlia_direct": false,
  "backoff_ms": 1000,
  "t_batch_ms": 100,
  "s_batch": 262144,
  "min_interval_ms": 50,
  "exec_lag": 3,
  "speculative": true,
  "round_timeout_ms": 100,
  "fallback_ms": 2000,
  "qc_broadcast": false
}
```

## Measured signature costs (µs)

```json
{
  "ed_sign": 21.260085000000004,
  "ed_verify": 48.74076,
  "ed_verify_batched": 32.01944296875,
  "bls_sign": 474.6982,
  "bls_verify": 1404.93968,
  "bls_agg_verify": 1583.6520999999998,
  "bls_agg_add": 1.7603000000000002,
  "ecdsa_sign": 44.71117,
  "ecdsa_recover": 45.455465
}
```
