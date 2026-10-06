# Process signal shutdown boundary

## Contract

`afs-node` registers SIGTERM/SIGINT on an independent OS thread and I/O reactor before creating its business executor. The first observed signal or service shutdown arms one 15-second native deadline. Further triggers cannot extend it. Normal service signal listeners still initiate drain. The guard is disarmed after business runtime and observability teardown. Error exit1 is a completed failure; watchdog exit124 is forced failure and cannot acknowledge dirty writes or cancel physical I/O.

Only `src/runtime.rs` and `src/bin/afs-node.rs` change. There is no module, RPC, configuration, dependency or durability-contract change. `ShutdownDeadline::new` remains manually triggered; process entry opts into `for_process`.

## Linux source gate

[Coherent report](source/qualified-linux/report.json) and [143 compile inputs](source/qualified-linux/compile-inputs.json) bind ARM64 Ubuntu Linux source and binaries. Formatting, strict all-target/all-feature Clippy,352 library tests,57 interface contracts,four error tests,nine local API tests,five privileged real FUSE tests,three feature checks and binary build pass. Two library environmental ignores remain explicit; the five FUSE tests ignored in the ordinary contract invocation are separately executed as root, not excluded.

[Original regression](source/original-signal-gap.log) fails with `for_process` reduced to the old manual guard: a business reactor blocked for five seconds cannot arm on SIGTERM before the unchanged three-second parent guard. With the fix, a150ms child watchdog exits124 within the unchanged two-second assertion. Separate real SIGTERM and SIGINT cases complete service drain and remain alive beyond the disarmed deadline. [Targeted output](source/target-signals.log) records five passing tests. [Read-only review](source/review.json) finds no concrete blocker; reviewer LSP diagnostics were unavailable, so this is not an LSP-gated approval. Root owns the fresh Linux compiler/test gate. The initial wrong build-directory preflight failure is retained in [its raw log](source/initial-wrong-build-path.log).

## Actual product proof

[Runtime report](runtime-product/guest/report.json), [description](runtime-product/README.md) and [Root independent verification](runtime-product/root-verification.json) identify run `20260930T233048Z-pid719442`. The isolated A ext4 runtime `/mnt/lima-afsadata/afs-delivery/shutdown-signal-v51`, ports18180..18183, uses memory Meta,R1,gRPC/TLS and separate OwnerFs/DFS mounts. Existing A/B v48 processes and binaries remain preserved.

| Action | Evidence |
| --- | --- |
| Normal held-fd dirty write, stop, restart | stop0; receipt/status0; first-read `accepted-dirty` |
| Commit `seed`, pause exact Meta, then accept `next` | same Meta PID/start ticks, observed stateT before four-byte write; write takes5.003s |
| Direct SIGTERM to exact Node | signal-to-receipt5.034s below the unchanged18s probe bound; exit1, `dfs.node_drain_incomplete` and `node.shutdown_failed` |
| Controller observes termination | stop1, failed status1 and matching receipt1 |
| Resume same Meta, start fresh Node | first-read `seed`, prior normal file `accepted-dirty` |
| Final Node/Meta stop | both0 with matching receipts; no listeners or FUSE mounts left |

All four lifecycle receipts bind PID, executable, config, start ticks, Linux boot ID and unique launch identity. Root additionally queries guest executable/controller/config/report hashes and final mounts/ports. Node SHA256 is `97d735243d741ef98d917f05a965c9e02703b3b05468ba42e7b1d7c5f2c38e31`; Meta SHA256 is `00cb99fe46f20024ab3b476978dd0e16d9e8a2c0535b95d5c8febc3aa863cf2d`. Controller SHA256 remains `b5dd0fa011475fb94658d3b1322ed8395c69aa1f85fbff5b5e1e4a307eec83a1`.

The actual run exercises error1, not watchdog124; the deterministic native regression covers124. Prior v50 stop20.124s has no signal/arm timestamp. Its extra time remains unexplained; differing procedures cannot support a20-to5-second performance claim.

## Collection integrity

The host initially used imported helper globals pointing to the old v50 runtime. Root rejected that report identity. [Attempt note](runtime-product/attempts/20260930T2330-host-collection-globals-error/README.md) records INCONCLUSIVE collection; the earlier v50 raw report remains in [its original bundle](../20261001-processctl/runtime-product/guest/report.json), not evidence for v51. The collector now binds all runtime globals, recollects the already completed v51 run without rerunning it, and matches [the explicit guest copy](runtime-product/guest-explicit-20260930T233254Z/report.json) SHA256 `c156f904d126646d5217569eaf4d592a0b31babbadd3ba6e9ca295b1931793f3`. The erroneous pulled payload was not retained separately in this bundle; the attempt note and immutable prior v50 report are the available record.

## Reproduction and limits

[Linux gate script](https://github.com/lelezi257/dms/blob/e925c5bcf0408851ebfa08a59df29953374da9e9/development/evidence/20261001-shutdown-signal/reproducers/afs-shutdown-signal-v51-gate.py) uses the research build-VM directory layout and prior input inventory. [Runtime orchestrator](https://github.com/lelezi257/dms/blob/e925c5bcf0408851ebfa08a59df29953374da9e9/development/evidence/20261001-shutdown-signal/reproducers/shutdown-signal-runtime-probe.py) imports the accompanying [helper](https://github.com/lelezi257/dms/blob/e925c5bcf0408851ebfa08a59df29953374da9e9/development/evidence/20261001-shutdown-signal/reproducers/processctl-runtime-probe.py). To use the recorded host layout, place both orchestrators under `experiments/afs-acceptance/` in the research root with `source/` beside `experiments/`; provide the documented qualified Linux artifacts and existing TLS paths. Probe preparation refuses an existing runtime. These are environment-specific development reproducers, not portable installer or release drivers.

The early startup window may arm the guard before service listeners are registered, resulting in bounded failed startup rather than graceful drain. Complete startup/signal races, cooperative resource cancellation, deployment/backend/R=N/RDMA faults, fairness baselines and final large-file/soak matrices remain unqualified. Formal69 cases are NOT_RUN; environment PREPARING; the overall goal remains active. TLS private keys and executable binaries are not included. `docs/handoff.md` is unchanged.

[Artifact manifest](artifacts.json) lists every artifact except this README and itself with bytes/SHA256; integrity is distinct from semantic qualification.
