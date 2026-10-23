# dx-demo

In-process simulator that runs today's Parlia flow and the DX two-phase design
(`bsc-decoupled-consensus-design.md` v2.1) on the same transaction stream and prints a
side-by-side comparison.

It is a demo of the protocol flow, not a networked node: all validators live in one process
and a discrete-event clock stands in for real time.
With `--real-net` the messages instead travel as real bytes between per-validator processes (see
[Real network mode](#real-network-mode)).

## What is real and what is modelled

| Part | How |
| --- | --- |
| Transaction execution | Every block is executed once, in chain order, with revm 38 on an in-memory state (native transfers plus a minimal ERC20). Gas and failures are real. A state root is computed over the keys each block touched. |
| Execution time charged | Per-transaction cost measured with that engine at startup. `--exec-us-per-tx` rescales it, for example to a mainnet-like 100 µs; `--exec-block-ms` pads every block up to a fixed time. |
| Signatures | Ed25519 (`ed25519-dalek`, batch verification), BLS12-381 (`blst` min_pk) and secp256k1 costs are measured at startup and charged to each validator's virtual CPU. |
| CPU | Each validator has one serial execution lane plus `--sig-cores` cores for signatures and sender recovery. |
| Network | Validators are placed in Tokyo, Singapore, Hong Kong, Frankfurt and Virginia with typical one-way cloud latencies, or all `--latency-ms` apart. Every message gets `--jitter-ms` of random delay either way. Each validator has a FIFO upload link of `--bandwidth-mbps`. |
| Faults | `--crashed` validators neither propose nor vote. `--bad-exec` validators run the protocol but compute a wrong state root: Parlia rejects their blocks (and they reject everyone else's), DX ExecVotes from them never match. |
| Parlia | In-turn proposer with `--turn-length`, block built on top of the imported parent, full block relayed through sqrt(n) peers, every validator imports (executes) before voting, BLS votes verified one by one, attestation of the nearest of the last `--attest-depth` ancestors with a quorum before sealing (1 = pre-Fermi parent-only rule, 3 = Fermi), finality when an attestation targets the block right after its source, as in geth. `--multi-attest` lets one header justify every ancestor in that window with a quorum (a hypothetical fork). `--compact-tx-bytes` relays blocks as short transaction ids. `--stream-chunk-ms` streams the block while it is built: every chunk of newly appended transactions is signed and relayed at once, validators holding the parent execute it speculatively and only execute the rest after the seal; `--bid-share` puts a builder bid at the head of the block, which can only stream from `--lock-ms` before the block time on, or with `--bid-stream` is locked chunk by chunk as it reaches the proposer, like local transactions. `--dx-inline-share` makes DX proposals carry that share of the block's transactions inline (a builder batch). `--dx-builder-share` routes that share of senders to a builder co-located with the leader instead of the batch layer and puts their transactions at the head of the block: inline in the proposal by default, or with `--dx-builder-chunk-ms` locked chunk by chunk (from the parent proposal on) and broadcast at once, so the proposal only references the chunks, votes wait for the chunk bodies and validators execute the chunks on top of the parent before the proposal arrives (scheme A in `dx-mev-scheme-a-detailed.md`). When the in-turn validator is down or its block is rejected, the next one by backoff rank seals after rank × `--backoff-ms`. |
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
with three seeds, for Parlia under the pre-Fermi (`--attest-depth 1`) and Fermi (`3`) attestation
rules and for DX with and without `--qc-broadcast`. [SCENARIOS-realnet.md](SCENARIOS-realnet.md) is
the same set run with `--real-net`.

## Real network mode

`--real-net` (on `run` and `scenarios`) carries every protocol message as real bytes instead of
computing its arrival time:

- Each validator gets its own network namespace (`dxnet<i>`), joined to a bridge by a veth pair
  whose uplink is limited to `--bandwidth-mbps` with an HTB qdisc.
- One `dx-demo agent` process per validator runs inside its namespace and keeps a TCP
  connection to every other agent. A message of `n` bytes is written to the socket as an
  `n`-byte frame and reported back to the controller when its last byte arrives.
- The controller runs the protocol state machines in wall-clock time. A message is delivered
  when the agent reports it, plus the one-way latency and jitter of the topology, which are
  added in user space because many container kernels (this one included) ship without
  `sch_netem`. TCP therefore never sees the WAN round trip.
- CPU work (execution, signatures, sender recovery) stays modelled as in the default mode;
  revm runs on a background thread so it does not delay the event loop.

The report gains three rows: messages carried, how late the event loop handled events
(p50 / p99 / max), and how many deliveries arrived after a later event had already been handled.
It needs root, `ip` and `tc` (`apt-get install iproute2`).

By default each agent writes to all its peers at once, so concurrent messages share the uplink
the way parallel TCP flows do. `--real-net-serial` makes each agent send frames one after
another, paced at the uplink rate, which is the FIFO upload link of the modelled mode carried
over real TCP. The two differ when a validator's uplink is saturated: a leader broadcasting a
large proposal reaches every peer only at the end of the whole fan-out when the flows share the
link, but reaches the nearest quorum early when it sends to one peer after another.

```bash
sudo target/release/dx-demo run --validators 21 --tps 8000 --gas-limit 300000000 \
  --exec-us-per-tx 100 --attest-depth 3 --real-net
sudo target/release/dx-demo scenarios --real-net --out SCENARIOS-realnet.md
```
