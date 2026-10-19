# dx-demo results

Rows: `ordered` is when the block holding the transaction was sealed (Parlia) or its order was committed at the median validator (DX). `executed` is when the median validator had executed it (for DX this includes speculative execution of not yet committed blocks). `final` is state finality: fast finality for Parlia, ExecCert at the median validator for DX. CPU figures are per validator over the measurement window.

## Measured signature costs (µs)

```json
{
  "ed_sign": 23.669805,
  "ed_verify": 48.229565,
  "ed_verify_batched": 28.959986718750002,
  "bls_sign": 437.745,
  "bls_verify": 1575.6408999999999,
  "bls_agg_verify": 1266.8661,
  "bls_agg_add": 2.4942620689655173,
  "ecdsa_sign": 66.51151999999999,
  "ecdsa_recover": 77.458755
}
```

## Base configuration

```json
{
  "validators": 45,
  "duration_s": 40.0,
  "warmup_s": 8.0,
  "tps": 2000.0,
  "erc20_share": 0.5,
  "accounts": 50000,
  "bandwidth_mbps": 1000.0,
  "sig_cores": 8,
  "exec_us_per_tx": null,
  "block_overhead_ms": 1.0,
  "seed": 7,
  "latency_ms": null,
  "jitter_ms": 2.0,
  "exec_block_ms": null,
  "crashed": [],
  "bad_exec": [],
  "block_interval_ms": 450,
  "turn_length": 16,
  "gas_limit": 100000000,
  "miner_reserve_ms": 120,
  "parlia_direct": false,
  "backoff_ms": 1000,
  "t_batch_ms": 100,
  "s_batch": 262144,
  "min_interval_ms": 450,
  "exec_lag": 3,
  "speculative": true,
  "round_timeout_ms": 1000,
  "fallback_ms": 2000,
  "qc_broadcast": false
}
```

## S1 2000 tx/s, in-memory revm execution

Light load; execution cost as measured with in-memory revm.

Execution cost charged: transfer 9.3 µs, ERC20 12.8 µs (in-memory revm measured 9.3 / 12.8 µs).

| metric | parlia | dx |
| --- | --- | --- |
| offered load (tx/s) | 2000 | 2000 |
| finalized (tx/s) | 2001 (100.0%) | 2001 (100.0%) |
| block interval (ms) | 450 | 450 |
| submit -> ordered p50 / p95 (ms) | 395 / 610 | 1332 / 1601 |
| submit -> executed p50 / p95 (ms) | 440 / 654 | 443 / 712 |
| submit -> final p50 / p95 (ms) | 1339 / 1554 | 1366 / 1635 |
| exec lane busy avg / max | 2% / 3% | 2% / 2% |
| sig + recovery cores avg / max | 0.31 / 0.31 | 0.94 / 0.94 |
| upload Mbit/s avg / max | 3 / 9 | 6 / 11 |
| blocks sealed late (>1ms) | 0 / 71 | - |
| blocks with parent attestation | 71 / 71 | - |
| mean block execution (ms) | 11.2 | 11.2 |
| failed txs | 0 | 0 |
| batches | - | 17837 |
| PoAs per proposal (mean) | - | 200.1 |
| batch signed -> PoA, median (ms) | - | 34.0 |
| blocks not committed | - | 1 |

## S2 2000 tx/s, 100 µs per tx

Execution scaled to roughly mainnet cost per transaction.

Execution cost charged: transfer 79.7 µs, ERC20 120.3 µs (in-memory revm measured 5.4 / 8.2 µs).

| metric | parlia | dx |
| --- | --- | --- |
| offered load (tx/s) | 2000 | 2000 |
| finalized (tx/s) | 2001 (100.0%) | 2001 (100.0%) |
| block interval (ms) | 450 | 450 |
| submit -> ordered p50 / p95 (ms) | 395 / 610 | 1332 / 1601 |
| submit -> executed p50 / p95 (ms) | 519 / 734 | 523 / 792 |
| submit -> final p50 / p95 (ms) | 1417 / 1632 | 1366 / 1635 |
| exec lane busy avg / max | 20% / 20% | 20% / 20% |
| sig + recovery cores avg / max | 0.31 / 0.31 | 0.94 / 0.94 |
| upload Mbit/s avg / max | 3 / 9 | 6 / 11 |
| blocks sealed late (>1ms) | 0 / 71 | - |
| blocks with parent attestation | 71 / 71 | - |
| mean block execution (ms) | 91.0 | 90.9 |
| failed txs | 0 | 0 |
| batches | - | 17837 |
| PoAs per proposal (mean) | - | 200.1 |
| batch signed -> PoA, median (ms) | - | 34.0 |
| blocks not committed | - | 1 |

## S3 5000 tx/s, 100 µs per tx

Execution takes about a third of each 450 ms block.

Execution cost charged: transfer 79.5 µs, ERC20 120.5 µs (in-memory revm measured 5.6 / 8.5 µs).

| metric | parlia | dx |
| --- | --- | --- |
| offered load (tx/s) | 5000 | 5000 |
| finalized (tx/s) | 5005 (100.0%) | 5005 (100.0%) |
| block interval (ms) | 451 | 450 |
| submit -> ordered p50 / p95 (ms) | 397 / 612 | 1331 / 1602 |
| submit -> executed p50 / p95 (ms) | 668 / 884 | 657 / 928 |
| submit -> final p50 / p95 (ms) | 1565 / 1787 | 1366 / 1637 |
| exec lane busy avg / max | 50% / 51% | 50% / 50% |
| sig + recovery cores avg / max | 0.54 / 0.55 | 1.17 / 1.17 |
| upload Mbit/s avg / max | 6 / 23 | 9 / 15 |
| blocks sealed late (>1ms) | 4 / 71 | - |
| blocks with parent attestation | 71 / 71 | - |
| mean block execution (ms) | 226.7 | 225.9 |
| failed txs | 0 | 0 |
| batches | - | 18070 |
| PoAs per proposal (mean) | - | 202.4 |
| batch signed -> PoA, median (ms) | - | 34.3 |
| blocks not committed | - | 1 |

## S4 8000 tx/s, 100 µs per tx, 300M gas limit

Execution takes about 60% of each block; the gas limit is raised so it is not the bound.

Execution cost charged: transfer 78.4 µs, ERC20 121.6 µs (in-memory revm measured 5.9 / 9.2 µs).

| metric | parlia | dx |
| --- | --- | --- |
| offered load (tx/s) | 8000 | 8000 |
| finalized (tx/s) | 8010 (100.0%) | 8010 (100.0%) |
| block interval (ms) | 481 | 450 |
| submit -> ordered p50 / p95 (ms) | 436 / 808 | 1331 / 1605 |
| submit -> executed p50 / p95 (ms) | 903 / 1396 | 792 / 1066 |
| submit -> final p50 / p95 (ms) | 5363 / 9119 | 1366 / 1639 |
| exec lane busy avg / max | 81% / 82% | 80% / 80% |
| sig + recovery cores avg / max | 0.75 / 0.78 | 1.40 / 1.40 |
| upload Mbit/s avg / max | 10 / 40 | 12 / 18 |
| blocks sealed late (>1ms) | 14 / 67 | - |
| blocks with parent attestation | 10 / 67 | - |
| mean block execution (ms) | 390.2 | 361.0 |
| failed txs | 0 | 0 |
| batches | - | 18077 |
| PoAs per proposal (mean) | - | 202.4 |
| batch signed -> PoA, median (ms) | - | 34.6 |
| blocks not committed | - | 1 |

## S5 11000 tx/s, 100 µs per tx, 600M gas limit

Beyond what one execution thread can do (10000 tx/s at 100 µs); both protocols must saturate.

Execution cost charged: transfer 79.7 µs, ERC20 120.3 µs (in-memory revm measured 9.2 / 13.8 µs).

| metric | parlia | dx |
| --- | --- | --- |
| offered load (tx/s) | 11000 | 11000 |
| finalized (tx/s) | 11031 (100.0%) | 11031 (100.0%) |
| block interval (ms) | 1549 | 813 |
| submit -> ordered p50 / p95 (ms) | 4913 / 8085 | 2606 / 4079 |
| submit -> executed p50 / p95 (ms) | 6539 / 9706 | 3551 / 5455 |
| submit -> final p50 / p95 (ms) | 25691 / 40095 | 3580 / 5484 |
| exec lane busy avg / max | 95% / 95% | 100% / 100% |
| sig + recovery cores avg / max | 0.77 / 0.78 | 1.63 / 1.64 |
| upload Mbit/s avg / max | 13 / 85 | 16 / 27 |
| blocks sealed late (>1ms) | 21 / 21 | - |
| blocks with parent attestation | 1 / 21 | - |
| mean block execution (ms) | 1453.2 | 885.4 |
| failed txs | 0 | 0 |
| batches | - | 18079 |
| PoAs per proposal (mean) | - | 361.2 |
| batch signed -> PoA, median (ms) | - | 34.8 |
| blocks not committed | - | 1 |

## S6 2000 tx/s, 100 µs per tx, DX T_minInterval 150 ms

Parlia keeps 450 ms blocks; DX lowers its minimum proposal interval.

Execution cost charged: transfer 77.7 µs, ERC20 122.3 µs (in-memory revm measured 6.2 / 9.8 µs).

| metric | parlia | dx |
| --- | --- | --- |
| offered load (tx/s) | 2000 | 2000 |
| finalized (tx/s) | 2001 (100.0%) | 2001 (100.0%) |
| block interval (ms) | 450 | 159 |
| submit -> ordered p50 / p95 (ms) | 395 / 610 | 590 / 911 |
| submit -> executed p50 / p95 (ms) | 519 / 734 | 319 / 569 |
| submit -> final p50 / p95 (ms) | 1417 / 1632 | 625 / 948 |
| exec lane busy avg / max | 20% / 20% | 21% / 21% |
| sig + recovery cores avg / max | 0.31 / 0.31 | 0.96 / 0.96 |
| upload Mbit/s avg / max | 3 / 9 | 6 / 9 |
| blocks sealed late (>1ms) | 0 / 71 | - |
| blocks with parent attestation | 71 / 71 | - |
| mean block execution (ms) | 91.0 | 32.8 |
| failed txs | 0 | 0 |
| batches | - | 17837 |
| PoAs per proposal (mean) | - | 70.8 |
| batch signed -> PoA, median (ms) | - | 34.1 |
| blocks not committed | - | 1 |

## S7 2000 tx/s, 100 µs per tx, DX T_minInterval 50 ms

DX rounds advance as soon as a QC forms.

Execution cost charged: transfer 81.6 µs, ERC20 118.4 µs (in-memory revm measured 6.1 / 8.8 µs).

| metric | parlia | dx |
| --- | --- | --- |
| offered load (tx/s) | 2000 | 2000 |
| finalized (tx/s) | 2001 (100.0%) | 2001 (100.0%) |
| block interval (ms) | 450 | 97 |
| submit -> ordered p50 / p95 (ms) | 395 / 610 | 480 / 955 |
| submit -> executed p50 / p95 (ms) | 519 / 734 | 326 / 580 |
| submit -> final p50 / p95 (ms) | 1417 / 1632 | 514 / 994 |
| exec lane busy avg / max | 20% / 20% | 21% / 21% |
| sig + recovery cores avg / max | 0.31 / 0.31 | 0.99 / 0.99 |
| upload Mbit/s avg / max | 3 / 9 | 7 / 9 |
| blocks sealed late (>1ms) | 0 / 71 | - |
| blocks with parent attestation | 71 / 71 | - |
| mean block execution (ms) | 91.0 | 20.3 |
| failed txs | 0 | 0 |
| batches | - | 17837 |
| PoAs per proposal (mean) | - | 42.9 |
| batch signed -> PoA, median (ms) | - | 34.1 |
| blocks not committed | - | 2 |

## S8 8000 tx/s, 100 µs per tx, 300M gas limit, DX T_minInterval 50 ms

Heavy load with QC-driven DX rounds.

Execution cost charged: transfer 78.3 µs, ERC20 121.7 µs (in-memory revm measured 5.5 / 8.5 µs).

| metric | parlia | dx |
| --- | --- | --- |
| offered load (tx/s) | 8000 | 8000 |
| finalized (tx/s) | 8010 (100.0%) | 8010 (100.0%) |
| block interval (ms) | 481 | 98 |
| submit -> ordered p50 / p95 (ms) | 436 / 808 | 495 / 960 |
| submit -> executed p50 / p95 (ms) | 903 / 1396 | 432 / 713 |
| submit -> final p50 / p95 (ms) | 5363 / 9119 | 539 / 998 |
| exec lane busy avg / max | 81% / 82% | 81% / 81% |
| sig + recovery cores avg / max | 0.75 / 0.78 | 1.45 / 1.45 |
| upload Mbit/s avg / max | 10 / 40 | 14 / 16 |
| blocks sealed late (>1ms) | 14 / 67 | - |
| blocks with parent attestation | 12 / 67 | - |
| mean block execution (ms) | 390.2 | 79.1 |
| failed txs | 0 | 0 |
| batches | - | 18077 |
| PoAs per proposal (mean) | - | 43.9 |
| batch signed -> PoA, median (ms) | - | 34.7 |
| blocks not committed | - | 2 |

## S9 5000 tx/s, 100 µs per tx, 200 Mbit/s uplinks

Constrained validator bandwidth.

Execution cost charged: transfer 81.5 µs, ERC20 118.5 µs (in-memory revm measured 5.7 / 8.3 µs).

| metric | parlia | dx |
| --- | --- | --- |
| offered load (tx/s) | 5000 | 5000 |
| finalized (tx/s) | 5005 (100.0%) | 5005 (100.0%) |
| block interval (ms) | 454 | 450 |
| submit -> ordered p50 / p95 (ms) | 404 / 625 | 1357 / 1628 |
| submit -> executed p50 / p95 (ms) | 740 / 965 | 683 / 955 |
| submit -> final p50 / p95 (ms) | 1642 / 1875 | 1399 / 1670 |
| exec lane busy avg / max | 50% / 51% | 50% / 50% |
| sig + recovery cores avg / max | 0.53 / 0.55 | 1.17 / 1.17 |
| upload Mbit/s avg / max | 6 / 23 | 9 / 15 |
| blocks sealed late (>1ms) | 4 / 71 | - |
| blocks with parent attestation | 71 / 71 | - |
| mean block execution (ms) | 228.4 | 225.9 |
| failed txs | 0 | 0 |
| batches | - | 18070 |
| PoAs per proposal (mean) | - | 202.4 |
| batch signed -> PoA, median (ms) | - | 35.3 |
| blocks not committed | - | 1 |

## S10 5000 tx/s, 100 µs per tx, both at 250 ms

Parlia block interval and DX T_minInterval both 250 ms, gas limit scaled to 56M.

Execution cost charged: transfer 78.5 µs, ERC20 121.5 µs (in-memory revm measured 5.7 / 8.8 µs).

| metric | parlia | dx |
| --- | --- | --- |
| offered load (tx/s) | 5000 | 5000 |
| finalized (tx/s) | 5005 (100.0%) | 5005 (100.0%) |
| block interval (ms) | 251 | 250 |
| submit -> ordered p50 / p95 (ms) | 295 / 436 | 816 / 1060 |
| submit -> executed p50 / p95 (ms) | 455 / 597 | 442 / 685 |
| submit -> final p50 / p95 (ms) | 955 / 1098 | 852 / 1096 |
| exec lane busy avg / max | 51% / 51% | 50% / 50% |
| sig + recovery cores avg / max | 0.66 / 0.67 | 1.18 / 1.19 |
| upload Mbit/s avg / max | 6 / 16 | 9 / 12 |
| blocks sealed late (>1ms) | 7 / 128 | - |
| blocks with parent attestation | 128 / 128 | - |
| mean block execution (ms) | 126.5 | 126.0 |
| failed txs | 0 | 0 |
| batches | - | 18070 |
| PoAs per proposal (mean) | - | 112.5 |
| batch signed -> PoA, median (ms) | - | 34.3 |
| blocks not committed | - | 2 |

## S11 2000 tx/s, 100 µs per tx, 21 validators, DX T_minInterval 50 ms

Smaller validator set.

Execution cost charged: transfer 86.2 µs, ERC20 113.8 µs (in-memory revm measured 6.9 / 9.1 µs).

| metric | parlia | dx |
| --- | --- | --- |
| offered load (tx/s) | 2000 | 2000 |
| finalized (tx/s) | 2001 (100.0%) | 2001 (100.0%) |
| block interval (ms) | 450 | 80 |
| submit -> ordered p50 / p95 (ms) | 395 / 610 | 365 / 880 |
| submit -> executed p50 / p95 (ms) | 517 / 732 | 238 / 527 |
| submit -> final p50 / p95 (ms) | 1415 / 1630 | 399 / 917 |
| exec lane busy avg / max | 20% / 20% | 21% / 21% |
| sig + recovery cores avg / max | 0.19 / 0.19 | 0.60 / 0.60 |
| upload Mbit/s avg / max | 2 / 6 | 4 / 5 |
| blocks sealed late (>1ms) | 0 / 71 | - |
| blocks with parent attestation | 71 / 71 | - |
| mean block execution (ms) | 91.0 | 16.9 |
| failed txs | 0 | 0 |
| batches | - | 8432 |
| PoAs per proposal (mean) | - | 16.7 |
| batch signed -> PoA, median (ms) | - | 5.9 |
| blocks not committed | - | 2 |

