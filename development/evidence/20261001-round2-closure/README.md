# Round 2: representative fault coverage and overall regression

Level: **stage gate PASS; representative ROUND2 scope PASS**. Current priorities
move to ROUND3 performance/resources and subsequently persistent backends.
Formal acceptance remains **69 NOT_RUN / ENV PREPARING**. This packet does not
qualify the complete release fault, backend or transport matrix.

## Whole-system evidence

| Scope | Evidence and limits |
| --- | --- |
| DFS admitted RDMA replica request exceeds caller deadline | [Actual GDB/verbs audit](build/dfs-deadline-v82-r3/cancel/audit.json): the business adapter posts one 4096-byte READ; caller receives DeadlineExceeded while the native worker is paused before consuming CQ completion. Client pooled source MR and server worker endpoint remain alive. The fixture explicitly closes the server registry lookup; this is not automatic production lookup removal. Resume persists the original exact chunk once, without gRPC replay, then releases the endpoint. Explicit client-pool retirement drains both sets of QP/MR/CQ/PD/context resources while the test process lives |
| Local test qualification | [Test binary and runner identity](build/dfs-deadline-v82-r3/cancel/identity.json), [27 checker checks](build/dfs-deadline-v82-r3/checker-regression.json), four actual required/Auto RXE adapter integrations. Same-VM plain gRPC and fixture-authorized placement; not an installed cross-VM authorization/fault matrix. Posted/unconsumed is not proof that physical DMA remains pending; the paused checkpoint does not exercise native CQ timeout |
| Batch source gate | [14 gate/feature exits](build/dfs-deadline-v82-final/full/), 421 library PASS / seven explicit environment ignores, 65 interface, four error, nine LocalAPI and five privileged FUSE tests; formatting, strict workspace/all-target/all-feature Clippy, five feature configurations and binary build. Exact unknown FileVersion ACK and repair claim/report gRPC tests execute again. [143-input comparison](build/dfs-deadline-v82-final/input-audit.json) has only `src/node/rpc/data.rs` cfg(test) changes. Public protocol, production behavior and persistent formats unchanged |
| Current installed whole system | Fresh isolated memory Meta and A/B required-RXE Nodes, existing frozen v80 product binaries/controller. [Workspace/Home REST](overall/a/evidence/workspace-r1.json), DFS write/fdatasync/fsync/close, two actual durable copies, repair Completed, physical digests, normal A/B restart and first cold read/EOF pass. [Before-restart consistency](overall/consistency-r1/report.json) and [after-restart consistency](overall/consistency-r2/report.json) each pass ten OwnerFs/DFS local/remote visibility, resize and close/reopen flows. Supported macOS controller orchestrates Linux workers; all file operations and probes use Linux guest ext4 |
| Actual transport and cleanup | [A counters](overall/a/evidence/metrics-r1.json), [B counters](overall/b/evidence/metrics-r1.json): positive RDMA payload and zero DFS gRPC payload. Counters from different process incarnations are not subtracted. Separate mounts and continuous memory Meta identities captured. New test processes stop normally and mounts disappear. Guest data free-space reserves remain at least 4 GiB; old files and experiments retained |
| Preserved cohort recovery | [Host power log](overall/host-sleep.log) records clamshell sleep 12:09–12:27 UTC. Old Nodes exited with expired-session errors during wake intervals; no proof of a defect under uninterrupted host execution. Original binaries restart A/B/C, [original Meta remains live](sleep-recovery/ctl/after.json), all six OwnerFs/DFS 4 MiB+17 reads have exact original hashes and EOF. New Node incarnations are explicitly recorded, not reported as continuous old PIDs |

Installed Node SHA `a3fe6573fc5f5a41c30b823855cfe2fdd1428950f878f1e29756e7bfc0d7e9d9`,
Meta `895f39fd660b7f7d9735eaaa4f3a082c3692a409d954c4aa8b5b0cc515a700ad`.
These are the v80 stripped artifacts; the fresh v82 debug build is not relabeled
as an installed binary. Test binary SHA
`1b6ce184b9fda887bd010786cf4b774f4606af70cca82748521f5ab08ba19378`.
The metadata policy is N=2/M=1; this proves actual asynchronous two-copy repair,
not a fresh three-copy synchronous deployment or memory Meta restart durability.

Original compile failures r1/r2 remain in [build](build/). An initial publication
audit assumed cleanup JSON had a `result` wrapper; the recorded export uses a
flat schema. The corrected audit reads that schema; no product fault or raw
runtime result was altered. First A/B exports stopped when preserved old PIDs
were found absent; partial `inputs` remain. Successful exports use `inputs-r2`
and `cleanup-r2.json` after explicit old-cohort recovery.

## Round exit and deferred scope

Round 2 covers the whole system through prior [Node interruption](../20261001-round2-node-recovery/README.md),
[network/ENOSPC](../20261001-round2-network-capacity/README.md),
[physical EIO and hard-error fixes](../20261001-owner-sync-eio/README.md),
[physical corruption and repair](../20261001-round2-corruption/README.md),
[Owner deadline](../20261001-owner-rdma-deadline/README.md), actual gRPC unknown
commit/repair replies, this DFS deadline and the final overall regression.
Known false-success defects are fixed in their measured scope. No observed
mainline, corruption, authorization, order or normal-resource defect is deferred.

Missing installed cross-VM ACK-loss, native CQ timeout, exceptional provider
teardown and complete fault/backend axes remain **unqualified**. Resource-lifetime
diagnostics continue in round 3; the complete fault matrix remains round 4.
Any newly observed unsafe consequence becomes an immediate blocker. Forced host
sleep is an environment observation, not a substitute for sustained liveness.
Fair durable MooseFS/3FS comparison prerequisites, capacity/copy/RPC costs and
then etcd/Redis parity/recovery form the next whole-system work. Full POSIX,
8 GiB and soak stay late; requirements are unchanged in [acceptance](../../../docs/acceptance.md).

[67 Linux semantic checks](audit.json) and [nine negative audit checks](negative.json)
pass. [Probes](probes/) preserve runner/checker and exact small-flow inputs.
Protected AGENTS and handoff hashes are unchanged. Publication hashes/links and
143 compiler inputs are independently checked in [verification](verification.json).
