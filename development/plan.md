**当前切片（2026-10-07）：** 固定6d51aeb/map66dbbe3e/157产品输入及release ELF未改。G2.21同步小补项实跑完成：DFS/local-file/gRPC/R2/OFF，A写64MiB、B/C各1预热5读，前后全SHA/EOF及4AFS真实wait0/3stdio rc0通过。ctl单调公共窗口中位45.149MiB/s，B/C纯C中位50.326/22.716；实际durable A+B、uniform去重为唯一4MiB，缓存未观察，不计三同步或3FS持平。旧父窗口和失败证据保持原样。 [完整证据](evidence/20261007-dfs-sync-read-small/README.md)。

**下一项：** DFS小删除G2.25先做独立合同/已有工具与比较依赖检查，再推进具备条件的功能/量化小项；真实环境阻塞停受影响项求助，不修环境打转。已留同步读数据不无输入重测或专项调优。G1历史8/8；G2 10限定完成/2普通性能FAIL/2bind进行中/13待验收。OFF试用可下载；bind默认OFF，锁/append/watch缺口、官方fuser API、Moose强持久及三同步3FS比较独立保留；大规模/长时/复杂可靠性/etcd/Redis后置。

[append偏移边界与暂缓理由](native-append-offset-boundary.md)：当前公开回复不返回实际追加位置；保留失败；经典锁也已按公开原语边界收口，不改第三方或反复重测。

**历史说明：** 下方旧检查点按原版本/范围保存；其旧下一动作由上方当前入口和验收主表覆盖。当前native偏移/经典锁/watch失败与普通性能失败原始记录均保留。

# Delivery task map

The active task map is [the three-stage acceptance checklist](trial-release-goals.md). This file gives execution order and dependencies; it does not replace the checklist or the full contract in [docs/acceptance.md](../docs/acceptance.md).

## Current checkpoint

G1/g1.5 is complete in its scoped trial sense: installable OwnerFs/DFS, memory demo, basic local/remote behavior, DFS basic behavior, local-file Meta restart recovery and colleague-facing lifecycle/self-check evidence. That result remains historical and bounded. It is not a current-candidate full POSIX, G2 performance or formal 69-case PASS.

G2 is active. The completed G2 items are bounded internal outputs: OwnerFs indexing/structural optimizations, DFS read-batch amplification reduction, and a limited small performance-tool qualification. Bind/native has real progress but remains in progress; the feature must be separately accepted and default OFF. Historical standards remain scoped to their recorded inputs; current OFF delivery and bounded remote deletion are complete, while performance parity and ON qualification remain open.

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
