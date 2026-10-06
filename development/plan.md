**当前切片（2026-10-07）：** 固定6d51aeb/map66dbbe3e/157输入，DFS-only/local-file Meta/gRPC/R2/native OFF完成64MiB一写两读：A fdatasync及目录fsync、B/C完整SHA与各1预热5读PASS；读中位数B26.138/C50.222MiB/s，父控制器含启动/预检/预热的768MiB总量诊断42.969MiB/s。两份Ready副本在A/C；均匀内容去重成各4MiB物理chunk，不称唯一64MiB语料或三同步/3FS正式性能。四个服务真实wait0、无新进程/mount、旧incarnation不变。首次收尾检查器误识别自身的FAIL原文保留，精确argv-token修正后通过，无产品重启/重测。 [证据](evidence/20261007-dfs-manyread-small/README.md)。

**下一项：** Owner远端小规模写按冻结屏障留核心数据；Moose强持久写基线/三同步3FS仍独立阻塞，普通性能优化、大规模/复杂可靠性及etcd/Redis后置。已有远端读/删除摸底与Moose客户端wait1 FAIL保留；bind默认OFF，锁/append偏移/watch缺口单列。G1历史8/8、G2 9限定完成/2普通性能FAIL/2bind进行中/14待验收不变；OFF试用可下载，不计完整G2.27。

**历史说明：** 下方旧检查点按原版本/范围保存；其旧下一动作由上方当前入口和验收主表覆盖。当前native偏移/经典锁/watch失败与普通性能失败原始记录均保留。

# Delivery task map

The active task map is [the three-stage acceptance checklist](trial-release-goals.md). This file gives execution order and dependencies; it does not replace the checklist or the full contract in [docs/acceptance.md](../docs/acceptance.md).

## Current checkpoint

G1/g1.5 is complete in its scoped trial sense: installable OwnerFs/DFS, memory demo, basic local/remote behavior, DFS basic behavior, local-file Meta restart recovery and colleague-facing lifecycle/self-check evidence. That result remains historical and bounded. It is not a current-candidate full POSIX, G2 performance or formal 69-case PASS.

G2 is active. The completed G2 items are bounded internal outputs: OwnerFs indexing/structural optimizations, DFS read-batch amplification reduction, and a limited small performance-tool qualification. Bind/native has real progress but remains in progress; the feature must be separately accepted and default OFF. Most current-candidate standard and performance items remain unrun.

G3 is deferred. Long soak, broad random/POSIX matrices, complex fault matrices, multi-Meta/HA, etcd 2 GiB topic and Redis are not the next blockers for small usable progress.

**High-priority independent maintenance:** the release slice has finished and stopped. R1 evidence cleanup is published. [R2 upstream gap report](repository-remediation.md) remains independently BLOCKED; continue independent E2E items. G1 and historical conclusions remain unchanged.

## Immediate sequence

Latest user override: pjdfstest targets functional completeness; after high-priority repository remediation, performance priority is the Issue42/PR43 container-mounted workspace path (G2.12 prerequisite qualification, then G2.13). Ordinary read/write baselines retain data and defer targeted optimization. The original sequence below remains a dependency/reference map, not an instruction to optimize every ordinary case first.

| Order | Checklist item | Exit |
| --- | --- | --- |
| 0 | G0.03 publish checkpoint | Code and related docs are coherent on GitHub, with verification notes and known gaps |
| 1 | G2.04 OwnerFs pjdfstest current candidate | OwnerFs complete applicable accounting or documented blocker |
| 2 | G2.06 Owner-relevant fixed LTP subset | Frozen list, ext4 reference boundary and OwnerFs result ledger |
| 3 | G2.07 Owner-relevant short FSx | Fixed seed/profile, no content or length mismatch |
| 4 | G2.08 affected Owner/basic + local-file recovery | Owner local/remote basics and Meta local-file restart on current candidate |
| 5 | G2.09 Owner local small read | Correct data and >=90% ext4 ordinary throughput under frozen case |
| 6 | G2.10 Owner local small write | Correct readback and >=90% ext4 ordinary throughput under matching barriers |
| 7 | G2.11 Owner local delete | Correct namespace and measured comparative report |
| 8 | G2.12/G2.13 bind function/performance | Explicit default-OFF switch, OFF regression, ON lifecycle and paired OFF/ON/ext4 numbers |
| 9 | G2.14-G2.16 Owner remote small cases | MooseFS parity for read/write, delete report |
| 10 | G2.05/G2.06/G2.07 DFS standard entry | DFS pjdfstest, DFS-relevant LTP and short FSx before DFS performance claims |
| 11 | G2.08 DFS affected basic check | DFS basics and any needed local-file recovery combination on current candidate |
| 12 | G2.21 DFS one-writer/many-readers | Highest-priority DFS performance item, with correctness and 3FS comparison |
| 13 | G2.22-G2.26 remaining DFS and large cases | Single read/write, multi-node, delete, 512 MiB/8 GiB records |
| 14 | G2.27 core performance delivery | Reproducible package, selected core cases, recovery check and state report |

This order is intentionally small-to-large. Completing a small item keeps its completed state; a later larger scale is a new item, not a reason to reopen the smaller result.

## Working rules

- Start with memory or local-file Meta when the case only needs filesystem behavior. Move to etcd/Redis only for their backend-specific items.
- Fix corruption, unsafe success, permission bypass, acknowledged-data loss and core recovery failure immediately.
- Defer broad coverage when it does not block the current item: 8 GiB, 8-hour soak, long FSx, full differential random, extensive fault axes and backend parity.
- Keep bind/native independent. OFF/FUSE must stay usable while ON is incomplete.
- Do not let a comparator problem stop unrelated functional progress. A blocked MooseFS/3FS lane blocks only the comparisons that require it.
- Record failed experiments once with enough evidence; do not repeat unchanged failed cases without a new cause or implementation change.

## Historical evidence boundary

Historical results remain useful for regression selection and risk assessment:

- v37/v48 pjdfstest full results prove those historical candidates only.
- g1.5 proves the trial package and local-file recovery scope only.
- Owner B1-B4 and DFS batch work prove bounded internal optimizations, not final performance ratios.
- PR43/bind experiments prove direction and partial numbers, not a shipped ON feature.
- etcd/Redis local evidence proves local slices only; final backend parity is still later.

Current execution reports must cite the exact current source/binary identities before using any result as current-candidate evidence.
