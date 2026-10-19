# dx-demo

In-process simulator that runs today's Parlia flow and the DX two-phase design
(`bsc-decoupled-consensus-design.md` v2.1) on the same transaction stream and prints a
side-by-side comparison.

It is a demo of the protocol flow, not a networked node: all validators live in one process
and a discrete-event clock stands in for real time.

## What is real and what is modelled

| Part | How |
| --- | --- |
| Transaction execution | Every block is executed once, in chain order, with revm 38 on an in-memory state (native transfers plus a minimal ERC20). Gas and failures are real. A state root is computed over the keys each block touched. |
| Execution time charged | Per-transaction cost measured with that engine at startup. `--exec-us-per-tx` rescales it, for example to a mainnet-like 100 µs; `--exec-block-ms` pads every block up to a fixed time. |
| Signatures | Ed25519 (`ed25519-dalek`, batch verification), BLS12-381 (`blst` min_pk) and secp256k1 costs are measured at startup and charged to each validator's virtual CPU. |
| CPU | Each validator has one serial execution lane plus `--sig-cores` cores for signatures and sender recovery. |
| Network | Validators are placed in Tokyo, Singapore, Hong Kong, Frankfurt and Virginia with typical one-way cloud latencies, or all `--latency-ms` apart. Every message gets `--jitter-ms` of random delay either way. Each validator has a FIFO upload link of `--bandwidth-mbps`. |
| Faults | `--crashed` validators neither propose nor vote. `--bad-exec` validators run the protocol but compute a wrong state root: Parlia rejects their blocks (and they reject everyone else's), DX ExecVotes from them never match. |
| Parlia | In-turn proposer with `--turn-length`, block built on top of the imported parent, full block relayed through sqrt(n) peers, every validator imports (executes) before voting, BLS votes verified one by one, parent attestation if a quorum arrived before sealing, finality when two consecutive blocks are attested. When the in-turn validator is down or its block is rejected, the next one by backoff rank seals after rank × `--backoff-ms`. |
| DX | Sender-sharded batches every `--t-batch-ms`, Ed25519 batch signatures, BLS acks aggregated into PoAs (f + 1), Jolteon two-chain commit with `--turn-length` rounds per leader and `--min-interval-ms`, speculative execution of verified proposals, ExecVote and ExecCert (2f + 1), backpressure through the ExecCert of block `r - k`. Rounds time out after `--round-timeout-ms` × 2^c counted from when the leader may propose; Timeouts carry the sender's highest QC, f + 1 of them make a validator time out too, 2f + 1 form a TC, and a TC skips the rest of that leader's turn. `--qc-broadcast` makes the leader that forms a QC broadcast it at once. A down owner's transactions are batched by the next validator after `--fallback-ms`. |

Not modelled: Byzantine behaviour beyond a wrong state root, network partitions, MEV, system
contracts, disk I/O beyond the per-transaction scale factor, and the downlink. Parlia votes are
counted off-chain.

## Running

```bash
cargo run --release -p dx-demo -- calibrate                  # measured costs on this machine
cargo run --release -p dx-demo -- run --exec-us-per-tx 100    # one scenario, both protocols
cargo run --release -p dx-demo -- run --tps 8000 --gas-limit 300000000 --min-interval-ms 50
cargo run --release -p dx-demo -- sweep --out RESULTS.md      # the built-in scenario set
cargo run --release -p dx-demo -- scenarios --out SCENARIOS.md # fault and load scenarios
```

`--help` on each subcommand lists every parameter. Defaults follow the design doc: 45
validators, `TurnLength` 16, 450 ms blocks, `T_batch` 100 ms, `k` 3.

[RESULTS.md](RESULTS.md) holds the output of `sweep` on a 4-core cloud VM. Signature costs
there (BLS verify about 1.5 ms) are slower than on a typical validator server, so the DX
signature CPU figures are on the high side.

[SCENARIOS.md](SCENARIOS.md) holds the output of `scenarios`: 4 validators, 50 ms blocks, every
block taking 20 ms to execute and 5 ms latency, under a baseline, 4000 tx/s, 100 ms execution,
20 ms latency, one validator down and one validator computing a wrong state root. Each is run
with three seeds, for Parlia and for DX with and without `--qc-broadcast`.
