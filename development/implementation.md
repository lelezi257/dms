# Delivery implementation rules

The current execution source of truth is [the three-stage acceptance checklist](trial-release-goals.md). The full release contract remains [docs/acceptance.md](../docs/acceptance.md), but the order is now iterative: ship the simple runnable version, qualify small standard and performance cases, then expand to complex reliability and backend matrices.

Architecture documents describe the accepted target design. Implementation progress and evidence belong in status, handoff and development records. Do not weaken public semantics, durability claims, permissions, close-to-open behavior or error propagation to make a case pass.

## Current priorities

The [three-stage goal table](trial-release-goals.md) is authoritative. The finite current-scenario G2.12 function and explicit ON trial delivery is closed with [version-scoped runtime, install and publication proof](evidence/20261008-workspace-bind-on-trial/README.md); this is not unrestricted production bind support or a new performance verdict.

1. Preserve historical G1/g1.5 8/8. Reuse unaffected version-scoped standard, local-file recovery and eight bind >=0.90 ext4 core performance results; current regression belongs to G2, not reopened G1.
2. G2.14–16: required Home workspace bind ON and remote FUSE cooperation/performance. Read/write require >=1.2x same-condition MooseFS throughput **and** <=0.8x independently measured operation latency. Delete requires correctness and comparison reporting. Optimize one actual product issue per round with frozen conditions, correct bytes/errors/freshness and before/after data; do not keep expanding measurement qualification or repeat packaging for every change.
3. G2.21 DFS one-writer/many-readers first. The one missing per-read measurement is closed; qualified three-synchronous-durable-copy 3FS parity remains pending. Other DFS core cases follow independently.
4. G2.09–11 ordinary local FUSE optimization follows remote and DFS; keep the same MooseFS dual targets.
5. Complex mixed append/offset/locks/watch, production command issuer/durable ACK, expanded topologies, live Meta-only availability/crash reliability, large/long matrices, multi-Meta, etcd and Redis remain later scopes. Original failures and explicit unsupported scope remain; any defect needed by the current supported flow must still be fixed. etcd resource work may use the user-authorized 2GiB topic; Redis is last/TODO.

OwnerFs workspace bind mount functionality and performance remain independent existing G2 tasks. Ordinary distribution config defaults OFF; the identified current trial provides explicit host ON steps. Never enable stale remote caching, weaken authority/permissions/errors/persistence or call a FUSE-self bind a bypass. No full POSIX or deferred combinations are inferred from finite ON success.

**当前出口（2026-10-08，G2.15）：** 远端写一次839→d14对照189.875→199.831MiB/s（+5.24%）、独立pwrite p95 7.050→6.951ms（-1.40%），达到测前保留线，Owner-only TCP公开API入站256KiB帧改动保留；p99+5.85%及轮次波动完整留数，不称稳定广泛提升。功能限定PASS／测量COMPLETE／正式Moose1.2/.8仍PENDING。 复用原64MiB同inode，B/C512MiB、ctl256MiB预算贯穿准入/运行/退出，采样峰值B264,790,016B、盘余21,269,839,872B，容量PASS；五actualwait0/原mount及保护进程不变。160输入与已过四测试/构建候选完全一致，Linux身份/fmt及537保存证据核验、1690raw+27guest实际恢复PASS；严格Clippy既有失败和19,493B/5ERRO/60WARN保留。无第三方/环境修补/新包；旧FAIL_BUDGET不改。G1历史8/8关闭、原G2仍12/0/15，DFS写按用户暂缓。下一Owner远端核心的一项剩余产品成本，复用有效基线、不重跑本项或扩资格矩阵。 [本轮版本与证据](evidence/20261008-owner-remote-write-server-pair/README.md)。

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
