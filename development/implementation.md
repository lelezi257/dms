# Delivery implementation rules

The current execution source of truth is [the three-stage acceptance checklist](trial-release-goals.md). The full release contract remains [docs/acceptance.md](../docs/acceptance.md), but the order is now iterative: ship the simple runnable version, qualify small standard and performance cases, then expand to complex reliability and backend matrices.

Architecture documents describe the accepted target design. Implementation progress and evidence belong in status, handoff and development records. Do not weaken public semantics, durability claims, permissions, close-to-open behavior or error propagation to make a case pass.

## Current priorities

1. Preserve the completed G1/g1.5 colleague trial: installable OwnerFs/DFS, memory demo and central local-file Meta restart recovery. New candidate regression is G2 work; it does not reopen the historical G1 exit.
2. Stabilize the current source snapshot and documentation so code, goals, known limits and evidence boundaries agree on GitHub.
3. Run Owner-first standard fallback and necessary recovery checks on the current candidate: OwnerFs pjdfstest, Owner-relevant fixed LTP subset, short FSx, and the affected Owner/basic local-file recovery combination. Run DFS standard entry before DFS performance claims.
4. Performance first: implement and qualify the Issue42/PR43 OwnerFs workspace bind mount path (explicit experimental switch, default OFF), then paired core ON/OFF/ext4 measurements for that bind path. Ordinary OwnerFs local/remote read/write cases may baseline and preserve data; defer targeted tuning. Their active target is now [throughput >=1.2x same-condition MooseFS and operation latency <=0.8x MooseFS](ownerfs-performance-criteria.md), judged independently.
5. Enter DFS performance with one-writer/many-readers first, then single read/write, multi-node read/write, delete and larger sizes. DFS comparisons use matched POSIX/FUSE and three synchronous durable copies against 3FS.
6. Keep bind/native as two independent G2 tasks: functional qualification and performance qualification. The switch must be explicit and default OFF. Public ON production use is not qualified until lifecycle, permissions, namespace/root ownership, mmap/watch/lock/append/seek and drain requirements pass.
7. Keep etcd and Redis near the end. etcd memory/resource work may use the user-authorized 2 GiB topic lane. Redis is last/TODO unless an earlier correctness defect makes it urgent.

## Architecture boundaries

OwnerFs and DFS remain separate mounts with separate backend state machines. OwnerFs is the small workspace path with a Home node and remote forwarding. DFS is the general distributed filesystem path with immutable chunks, file versions, placement and repair. They may share FUSE and transport infrastructure, but they must not share inode/handle/cache authority in ways that blur semantics.

Meta owns namespace, inode records, versions, layout roots, placement, leases and idempotent commit results. File bytes do not pass through Meta. R=1 and R=N split below `ChunkStore`, not in layout or public file semantics. gRPC and RDMA carry the same identity, authorization, completion and error contracts.

Do not pre-build Meta HA, DFS SDK, VerifiedCache/Seed, external spill or broad migration machinery for the current phase. Add narrow interfaces only when current cases need them.

## Slice discipline

Before each slice, name the exact checklist item, current candidate identity, data size, backend/meta/transport axes, pass line, stop point and verification commands. A slice can be small; it must still say what it proves and what it does not prove.

Immediately fix silent corruption, unsafe success, permission bypass, acknowledged-data loss, broken commit ordering and normal-use resource exhaustion. Defer large matrix breadth, long soak, broad random cases, etcd/Redis parity and native ON polish when the issue does not block the current smaller item.

Use [validation.md](validation.md) to size verification. Reuse historical evidence only with its source/runtime identity and scope. Historical pjdfstest, g1.5 and optimization evidence are useful context, not current-candidate PASS.

## Publication and handoff

The user's 2026-10-07 standing authorization makes main the sole daily development and delivery entry. Commit each completed independent item under Lore and normally push origin/main after the necessary affected checks. No feature PR/MR or per-feature human review/approval gate is required; review the overall project after the overall goal is complete. Preserve incomplete work and its evidence, and do not rewrite history or force push. See the [main convergence inventory](evidence/20261007-main-convergence/README.md).

Refresh [docs/handoff.md](../docs/handoff.md) only when the user explicitly asks for its refresh. Routine convergence/publication updates current-checkpoint.md and the evidence index; the protected historical handoff remains unchanged in this slice.

Every commit follows the workspace Lore protocol. Commit messages must record the intent, verification, known gaps and scope risk. Do not claim G2 completion, full POSIX qualification, native ON readiness, formal 69-case release, 8 GiB qualification or etcd/Redis parity unless those exact gates have fresh evidence.
