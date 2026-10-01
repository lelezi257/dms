# Round 1 healthy system integration

Level: **identified healthy-flow subset PASS and Linux source stage gate PASS**.
Round 1 is still open. Formal cases remain **69 NOT_RUN**, ENV **PREPARING**.

## Candidate and environment

The production Node and Meta are byte-identical to qualified v75/v76 binaries:
Node `d28d17faa8bb71eb8c0a28241e8556396e440b204cf0242d2af8133700f6a494`,
Meta `64d85fc3933637ccd7fdf4eafded5e5c8cb2354e73b4c8d1a27b2b8d43f4c7e7`.
The initial Linux package identifies source `2ce58eee4128643f42df3f34798c2b3210245151`
and SHA256 `ccd21dcbe1cd0a153a13f06e84e730fc1f44811e973069fe2caffd759a50dad3`.
The runtime controller is the corrected candidate
`01f38d32f34994f89bea3c75f57eecc506db6157309e74f3fe5425f64775601e`;
it was replaced explicitly after installation. This is **not proof that the original
archive contains the correction**. A fresh corrected-package install remains required.

ctl/A/B/C are the agreed ARM64 Linux guests: ctl 2 CPU/4 GiB, each Node 2 CPU/6 GiB,
kernel `6.8.0-142-generic`, guest ext4 state/data and real RXE `rxe0`.
Build was stopped during cross-VM runs. Each [preflight](a/preflight.json),
[install record](a/install.json), configuration and process-incarnation file is retained.
New isolated paths end in `round1-mainline-v77-rn`; old services/data were preserved.
Meta is central, memory-backed, and configured for **desired2/sync2**.
OwnerFs and DFS are independent mounts in each Node; mTLS binds individual Nodes.

## User-visible flow and independent checks

| Flow | Proof |
| --- | --- |
| Install, process readiness, workspace/Home REST | Four installer records; [Home A query](a/workspace.json) identifies routable Home endpoints and a serving lease |
| Owner local and remote | [A local write](a/owner-local-write.json), [B remote write](b/owner-remote-write.json), B/C reads and [physical Home hashes](a/owner-physical-sha256.txt) agree on 4 MiB +17 bytes, length and EOF; [local-only counters](a/metrics-owner-local.txt) show zero peer payload |
| DFS synchronous RN | [A write](a/dfs-rn-write.json), [actual Meta replica records](a/replicas-after-write.json) show two distinct Ready DurableReplica Nodes for each Chunk, exact persisted bytes/digest/epochs; B/C reads and physical Chunk hashes agree |
| Normal B restart | [restart](b/restart.log), before/after identities, [Owner read](b/owner-after-restart.json), [DFS read](b/dfs-after-restart.json), [replica health](b/replicas-after-restart.json) |
| Normal small-file semantics | OwnerFs/DFS A/B same-mount visibility, close-only commit, sparse growth, truncate/rewrite and cross-node fresh open; each named JSON checks bytes/length/error rather than only exit status |
| Auto RXE through production Node/FUSE | [A peer-read counters](a/metrics-after-peer-reads.txt), [B replica counters](b/metrics-after-cold-read.txt), native completion logs: actual RDMA payload, zero gRPC file payload |
| Explicit gRPC | Separate process incarnations/configs under `grpc/`: remote Owner write/read and DFS replica/read payloads use gRPC; initial and second-incarnation counters are separate |
| Required RDMA | `rdma-required/` configs, identities and remote Owner/DFS operations; 44-byte new payload uses actual verbs, gRPC file payload is zero |
| Normal shutdown | All Node phases and final Meta stop exit0; final statuses stopped and own mounts/listeners absent. Global verbs inventory can include preserved old services; it is not a zero-global-resources or peak/lifetime qualification |

## Failures retained and corrected scope

The first deployment only configured Node replication parameters. Meta still used
R1; [actual observation](r1-observed-policy.json) proves desired1/available1.
Its file flows remain R1 evidence, regardless of misleading initial probe filenames.
RN proof comes exclusively from the fresh centrally configured runtime above.

LAN-bound healthy services initially failed controller readiness because inherited
HTTP proxies intercepted the health request. [Original starts](ctl/start-original.log)
and [original failing regression](controller/original-failure.log) are retained.
The controller now bypasses proxies for local process health. [Linux script regression](controller/fixed-regression.log)
passes; actual Meta/three Node starts and normal restart pass with the correction.

The first truncate/rewrite probe selected a nonexistent fixture file; its ERROR
JSON is retained. Corrected probes select an existing file. The first C gRPC read
lacked the copied helper; the second incarnation's explicit read results qualify
that path. Missing-helper/fixture setup errors are not product failures or PASS.
The first source-gate snapshot omitted `client/`; its preparation failure is
retained separately. Identity audit also found missing example/error-code inputs
in the next snapshot: its passing commands are retained as incomplete-snapshot
results, not a full gate. After all143 declared inputs match the actual source,
the complete snapshot passes the final gate.

## Limits and next work

This is a small memory-backed functional subset, not a full round or acceptance
matrix. Fresh corrected-package installation, current cohort async repair completion
and round-end overall regression remain open. Faults, persistent Meta recovery,
complete replica/transport axes, strict POSIX accounting, performance, 8 GiB and soak
retain their assigned rounds in the [issue ledger](../../issues.md).
The [source gate](gate/full/gate/lib.log) verifies 414 library, 65 contract,
4 shared-error, 9 local API and 5 privileged FUSE tests, formatting, strict
Clippy, five feature configurations and binary builds. Explicit ignored tests
are not counted as run; the new Owner posted deadline has its separate
[actual RXE proof](../20261001-owner-rdma-deadline/README.md).
