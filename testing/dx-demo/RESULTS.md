# dx-demo results

Rows: `ordered` is when the block holding the transaction was sealed (Parlia) or its order was committed at the median validator (DX). `executed` is when the median validator had executed it (for DX this includes speculative execution of not yet committed blocks). `final` is state finality: fast finality for Parlia, ExecCert at the median validator for DX. CPU figures are per validator over the measurement window.

## Measured signature costs (µs)

```json
{
  "ed_sign": 23.02652,
  "ed_verify": 50.122415000000004,
  "ed_verify_batched": 27.633650781250005,
  "bls_sign": 440.6706,
  "bls_verify": 1516.84568,
  "bls_agg_verify": 1282.9506333333334,
  "bls_agg_add": 1.6671172413793105,
  "ecdsa_sign": 37.271365,
  "ecdsa_recover": 46.38753500000001
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
  "block_interval_ms": 450,
  "turn_length": 16,
  "gas_limit": 100000000,
  "miner_reserve_ms": 120,
  "parlia_direct": false,
  "t_batch_ms": 100,
  "s_batch": 262144,
  "min_interval_ms": 450,
  "exec_lag": 3,
  "speculative": true
}
```

## S1 2000 tx/s, in-memory revm execution

Light load; execution cost as measured with in-memory revm.

Execution cost charged: transfer 5.6 µs, ERC20 9.1 µs (in-memory revm measured 5.6 / 9.1 µs).

| metric | parlia | dx |
| --- | --- | --- |
| offered load (tx/s) | 2000 | 2000 |
| finalized (tx/s) | 2001 (100.0%) | 2001 (100.0%) |
| block interval (ms) | 450 | 450 |
| submit -> ordered p50 / p95 (ms) | 396 / 610 | 1330 / 1600 |
| submit -> executed p50 / p95 (ms) | 435 / 650 | 438 / 708 |
| submit -> final p50 / p95 (ms) | 1335 / 1549 | 1364 / 1635 |
| exec lane busy avg / max | 2% / 2% | 2% / 2% |
| sig + recovery cores avg / max | 0.24 / 0.24 | 0.88 / 0.88 |
| upload Mbit/s avg / max | 3 / 9 | 6 / 11 |
| blocks sealed late (>1ms) | 0 / 71 | - |
| blocks with parent attestation | 71 / 71 | - |
| mean block execution (ms) | 7.4 | 7.4 |
| failed txs | 0 | 0 |
| batches | - | 17837 |
| PoAs per proposal (mean) | - | 146.2 |
| batch signed -> PoA, median (ms) | - | 34.9 |
| PoAs never proposed | - | 0 |

## S2 2000 tx/s, 100 µs per tx

Execution scaled to roughly mainnet cost per transaction.

Execution cost charged: transfer 79.4 µs, ERC20 120.6 µs (in-memory revm measured 5.6 / 8.5 µs).

| metric | parlia | dx |
| --- | --- | --- |
| offered load (tx/s) | 2000 | 2000 |
| finalized (tx/s) | 2001 (100.0%) | 2001 (100.0%) |
| block interval (ms) | 450 | 450 |
| submit -> ordered p50 / p95 (ms) | 396 / 610 | 1330 / 1600 |
| submit -> executed p50 / p95 (ms) | 519 / 733 | 521 / 791 |
| submit -> final p50 / p95 (ms) | 1417 / 1632 | 1364 / 1635 |
| exec lane busy avg / max | 20% / 20% | 20% / 20% |
| sig + recovery cores avg / max | 0.24 / 0.24 | 0.88 / 0.88 |
| upload Mbit/s avg / max | 3 / 9 | 6 / 11 |
| blocks sealed late (>1ms) | 0 / 71 | - |
| blocks with parent attestation | 71 / 71 | - |
| mean block execution (ms) | 91.0 | 90.9 |
| failed txs | 0 | 0 |
| batches | - | 17837 |
| PoAs per proposal (mean) | - | 146.2 |
| batch signed -> PoA, median (ms) | - | 34.9 |
| PoAs never proposed | - | 0 |

## S3 5000 tx/s, 100 µs per tx

Execution takes about a third of each 450 ms block.

Execution cost charged: transfer 78.4 µs, ERC20 121.6 µs (in-memory revm measured 6.0 / 9.3 µs).

| metric | parlia | dx |
| --- | --- | --- |
| offered load (tx/s) | 5000 | 5000 |
| finalized (tx/s) | 5005 (100.0%) | 5005 (100.0%) |
| block interval (ms) | 451 | 450 |
| submit -> ordered p50 / p95 (ms) | 397 / 612 | 1330 / 1601 |
| submit -> executed p50 / p95 (ms) | 668 / 884 | 656 / 927 |
| submit -> final p50 / p95 (ms) | 1565 / 1786 | 1365 / 1636 |
| exec lane busy avg / max | 50% / 51% | 50% / 50% |
| sig + recovery cores avg / max | 0.38 / 0.38 | 1.03 / 1.03 |
| upload Mbit/s avg / max | 6 / 23 | 9 / 15 |
| blocks sealed late (>1ms) | 4 / 71 | - |
| blocks with parent attestation | 71 / 71 | - |
| mean block execution (ms) | 226.7 | 225.9 |
| failed txs | 0 | 0 |
| batches | - | 18070 |
| PoAs per proposal (mean) | - | 148.1 |
| batch signed -> PoA, median (ms) | - | 35.1 |
| PoAs never proposed | - | 0 |

## S4 8000 tx/s, 100 µs per tx, 300M gas limit

Execution takes about 60% of each block; the gas limit is raised so it is not the bound.

Execution cost charged: transfer 79.6 µs, ERC20 120.4 µs (in-memory revm measured 5.7 / 8.7 µs).

| metric | parlia | dx |
| --- | --- | --- |
| offered load (tx/s) | 8000 | 8000 |
| finalized (tx/s) | 8010 (100.0%) | 8010 (100.0%) |
| block interval (ms) | 481 | 450 |
| submit -> ordered p50 / p95 (ms) | 436 / 808 | 1330 / 1604 |
| submit -> executed p50 / p95 (ms) | 903 / 1397 | 790 / 1066 |
| submit -> final p50 / p95 (ms) | 5365 / 9121 | 1365 / 1639 |
| exec lane busy avg / max | 81% / 82% | 80% / 80% |
| sig + recovery cores avg / max | 0.50 / 0.52 | 1.17 / 1.17 |
| upload Mbit/s avg / max | 10 / 40 | 12 / 18 |
| blocks sealed late (>1ms) | 14 / 67 | - |
| blocks with parent attestation | 10 / 67 | - |
| mean block execution (ms) | 390.3 | 361.1 |
| failed txs | 0 | 0 |
| batches | - | 18077 |
| PoAs per proposal (mean) | - | 148.2 |
| batch signed -> PoA, median (ms) | - | 35.2 |
| PoAs never proposed | - | 0 |

## S5 11000 tx/s, 100 µs per tx, 600M gas limit

Beyond what one execution thread can do (10000 tx/s at 100 µs); both protocols must saturate.

Execution cost charged: transfer 79.6 µs, ERC20 120.4 µs (in-memory revm measured 5.8 / 8.7 µs).

| metric | parlia | dx |
| --- | --- | --- |
| offered load (tx/s) | 11000 | 11000 |
| finalized (tx/s) | 11031 (100.0%) | 11031 (100.0%) |
| block interval (ms) | 1549 | 814 |
| submit -> ordered p50 / p95 (ms) | 4918 / 8093 | 2610 / 4115 |
| submit -> executed p50 / p95 (ms) | 6547 / 9714 | 3557 / 5455 |
| submit -> final p50 / p95 (ms) | 25698 / 40101 | 3587 / 5485 |
| exec lane busy avg / max | 95% / 95% | 100% / 100% |
| sig + recovery cores avg / max | 0.48 / 0.49 | 1.30 / 1.30 |
| upload Mbit/s avg / max | 13 / 85 | 16 / 27 |
| blocks sealed late (>1ms) | 21 / 21 | - |
| blocks with parent attestation | 1 / 21 | - |
| mean block execution (ms) | 1453.7 | 887.0 |
| failed txs | 0 | 0 |
| batches | - | 18079 |
| PoAs per proposal (mean) | - | 220.5 |
| batch signed -> PoA, median (ms) | - | 35.5 |
| PoAs never proposed | - | 0 |

## S6 2000 tx/s, 100 µs per tx, DX T_minInterval 150 ms

Parlia keeps 450 ms blocks; DX lowers its minimum proposal interval.

Execution cost charged: transfer 76.0 µs, ERC20 124.0 µs (in-memory revm measured 6.0 / 9.8 µs).

| metric | parlia | dx |
| --- | --- | --- |
| offered load (tx/s) | 2000 | 2000 |
| finalized (tx/s) | 2001 (100.0%) | 2001 (100.0%) |
| block interval (ms) | 450 | 159 |
| submit -> ordered p50 / p95 (ms) | 396 / 610 | 589 / 908 |
| submit -> executed p50 / p95 (ms) | 519 / 733 | 318 / 569 |
| submit -> final p50 / p95 (ms) | 1417 / 1632 | 625 / 945 |
| exec lane busy avg / max | 20% / 20% | 21% / 21% |
| sig + recovery cores avg / max | 0.24 / 0.24 | 0.91 / 0.91 |
| upload Mbit/s avg / max | 3 / 9 | 6 / 9 |
| blocks sealed late (>1ms) | 0 / 71 | - |
| blocks with parent attestation | 71 / 71 | - |
| mean block execution (ms) | 90.9 | 32.8 |
| failed txs | 0 | 0 |
| batches | - | 17837 |
| PoAs per proposal (mean) | - | 50.4 |
| batch signed -> PoA, median (ms) | - | 34.9 |
| PoAs never proposed | - | 0 |

## S7 2000 tx/s, 100 µs per tx, DX T_minInterval 50 ms

DX rounds advance as soon as a QC forms.

Execution cost charged: transfer 79.3 µs, ERC20 120.7 µs (in-memory revm measured 5.6 / 8.6 µs).

| metric | parlia | dx |
| --- | --- | --- |
| offered load (tx/s) | 2000 | 2000 |
| finalized (tx/s) | 2001 (100.0%) | 2001 (100.0%) |
| block interval (ms) | 450 | 96 |
| submit -> ordered p50 / p95 (ms) | 396 / 610 | 477 / 954 |
| submit -> executed p50 / p95 (ms) | 519 / 733 | 324 / 581 |
| submit -> final p50 / p95 (ms) | 1417 / 1632 | 511 / 993 |
| exec lane busy avg / max | 20% / 20% | 21% / 21% |
| sig + recovery cores avg / max | 0.24 / 0.24 | 0.94 / 0.94 |
| upload Mbit/s avg / max | 3 / 9 | 7 / 9 |
| blocks sealed late (>1ms) | 0 / 71 | - |
| blocks with parent attestation | 71 / 71 | - |
| mean block execution (ms) | 90.9 | 20.2 |
| failed txs | 0 | 0 |
| batches | - | 17837 |
| PoAs per proposal (mean) | - | 29.1 |
| batch signed -> PoA, median (ms) | - | 35.1 |
| PoAs never proposed | - | 0 |

## S8 8000 tx/s, 100 µs per tx, 300M gas limit, DX T_minInterval 50 ms

Heavy load with QC-driven DX rounds.

Execution cost charged: transfer 79.5 µs, ERC20 120.5 µs (in-memory revm measured 5.7 / 8.7 µs).

| metric | parlia | dx |
| --- | --- | --- |
| offered load (tx/s) | 8000 | 8000 |
| finalized (tx/s) | 8010 (100.0%) | 8010 (100.0%) |
| block interval (ms) | 481 | 97 |
| submit -> ordered p50 / p95 (ms) | 436 / 808 | 491 / 955 |
| submit -> executed p50 / p95 (ms) | 903 / 1397 | 427 / 709 |
| submit -> final p50 / p95 (ms) | 5365 / 9121 | 532 / 993 |
| exec lane busy avg / max | 81% / 82% | 81% / 81% |
| sig + recovery cores avg / max | 0.50 / 0.52 | 1.22 / 1.22 |
| upload Mbit/s avg / max | 10 / 40 | 14 / 16 |
| blocks sealed late (>1ms) | 14 / 67 | - |
| blocks with parent attestation | 10 / 67 | - |
| mean block execution (ms) | 390.3 | 78.8 |
| failed txs | 0 | 0 |
| batches | - | 18077 |
| PoAs per proposal (mean) | - | 29.5 |
| batch signed -> PoA, median (ms) | - | 35.3 |
| PoAs never proposed | - | 0 |

## S9 5000 tx/s, 100 µs per tx, 200 Mbit/s uplinks

Constrained validator bandwidth.

Execution cost charged: transfer 79.7 µs, ERC20 120.3 µs (in-memory revm measured 5.7 / 8.7 µs).

| metric | parlia | dx |
| --- | --- | --- |
| offered load (tx/s) | 5000 | 5000 |
| finalized (tx/s) | 5005 (100.0%) | 5005 (100.0%) |
| block interval (ms) | 454 | 450 |
| submit -> ordered p50 / p95 (ms) | 404 / 624 | 1355 / 1628 |
| submit -> executed p50 / p95 (ms) | 739 / 964 | 681 / 955 |
| submit -> final p50 / p95 (ms) | 1641 / 1874 | 1398 / 1670 |
| exec lane busy avg / max | 50% / 51% | 50% / 50% |
| sig + recovery cores avg / max | 0.38 / 0.39 | 1.03 / 1.03 |
| upload Mbit/s avg / max | 6 / 23 | 9 / 15 |
| blocks sealed late (>1ms) | 4 / 71 | - |
| blocks with parent attestation | 71 / 71 | - |
| mean block execution (ms) | 228.4 | 225.9 |
| failed txs | 0 | 0 |
| batches | - | 18070 |
| PoAs per proposal (mean) | - | 148.1 |
| batch signed -> PoA, median (ms) | - | 36.2 |
| PoAs never proposed | - | 0 |

## S10 5000 tx/s, 100 µs per tx, both at 250 ms

Parlia block interval and DX T_minInterval both 250 ms, gas limit scaled to 56M.

Execution cost charged: transfer 79.4 µs, ERC20 120.6 µs (in-memory revm measured 6.4 / 9.7 µs).

| metric | parlia | dx |
| --- | --- | --- |
| offered load (tx/s) | 5000 | 5000 |
| finalized (tx/s) | 5005 (100.0%) | 5005 (100.0%) |
| block interval (ms) | 251 | 250 |
| submit -> ordered p50 / p95 (ms) | 295 / 436 | 815 / 1059 |
| submit -> executed p50 / p95 (ms) | 455 / 596 | 441 / 684 |
| submit -> final p50 / p95 (ms) | 955 / 1098 | 852 / 1096 |
| exec lane busy avg / max | 51% / 51% | 50% / 50% |
| sig + recovery cores avg / max | 0.50 / 0.51 | 1.04 / 1.04 |
| upload Mbit/s avg / max | 6 / 16 | 9 / 12 |
| blocks sealed late (>1ms) | 7 / 128 | - |
| blocks with parent attestation | 128 / 128 | - |
| mean block execution (ms) | 126.5 | 125.9 |
| failed txs | 0 | 0 |
| batches | - | 18070 |
| PoAs per proposal (mean) | - | 82.1 |
| batch signed -> PoA, median (ms) | - | 35.1 |
| PoAs never proposed | - | 0 |

## S11 2000 tx/s, 100 µs per tx, 21 validators, DX T_minInterval 50 ms

Smaller validator set.

Execution cost charged: transfer 78.7 µs, ERC20 121.3 µs (in-memory revm measured 5.7 / 8.7 µs).

| metric | parlia | dx |
| --- | --- | --- |
| offered load (tx/s) | 2000 | 2000 |
| finalized (tx/s) | 2001 (100.0%) | 2001 (100.0%) |
| block interval (ms) | 450 | 79 |
| submit -> ordered p50 / p95 (ms) | 395 / 610 | 363 / 879 |
| submit -> executed p50 / p95 (ms) | 517 / 731 | 238 / 527 |
| submit -> final p50 / p95 (ms) | 1414 / 1629 | 398 / 916 |
| exec lane busy avg / max | 20% / 20% | 21% / 21% |
| sig + recovery cores avg / max | 0.19 / 0.20 | 0.57 / 0.57 |
| upload Mbit/s avg / max | 2 / 6 | 4 / 5 |
| blocks sealed late (>1ms) | 0 / 71 | - |
| blocks with parent attestation | 71 / 71 | - |
| mean block execution (ms) | 91.0 | 16.8 |
| failed txs | 0 | 0 |
| batches | - | 8432 |
| PoAs per proposal (mean) | - | 12.6 |
| batch signed -> PoA, median (ms) | - | 4.1 |
| PoAs never proposed | - | 0 |

