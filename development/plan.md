**最新开发/交付入口（2026-10-07，决策）：main唯一入口。** 按用户授权，fix/native-orderly-recovery-20261007相对f09185e的全部有效修改（恢复、归属整改、测试、文档和证据）及本次已验证Node启动失败修复73842cd已无冲突fast-forward纳入本地main，本记录随main正常推送；远端实际提交/文件树以推送核对回执为准。其它分支逐项盘点无当前AFS有效遗漏；8个旧dirty工作树及1个untracked草稿原样保留并在源码树外归档、验证补丁可恢复。不创建PR/MR，不设单特性人工审批；必要Linux门禁继续，整体目标完成后统一项目review。合并不升级历史验收，G1历史8/8不重开。下一仍G2.12 accepted Node/workspace生命周期→G2.13性能；普通性能FAIL/复杂可靠性/fuser原版API阻塞后置或独立保留。[纳入、未纳入、版本和证据](evidence/20261007-main-convergence/README.md)。下方按版本保留的历史记录不覆盖本入口。

**当前状态（2026-10-07）：** 3FS基线资格按用户决定留专题，主线容器workspace。main f09185e、产品6d/157/map66/既有ELF包的通过范围保留；新rootfs私有副本/命令续号修复在独立分支fix/native-orderly-recovery-20261007，不视为新可用候选。G1历史8/8，G2为10限定完成/2性能FAIL/2bind进行中/13待验收，bind默认OFF。

**新增/阻塞：** 工具11个独立Linux guards通过；早期Rust10+4有输出但未冻结源码SHA，不计最终版本PASS。静态审阅发现并最小修正非root测试入口，原FAIL保留。afs-build85GiB根盘实际满：误用debug与root工具链下载、后续ENOSPC，已停止构建并求助；最终续号/源码门禁/新包/重启读回均未完成。[版本、失败与证据](evidence/20261007-native-orderly-recovery/README.md)。

**下一项：** 等环境处理决定后一次准入，复用release缓存、普通用户构建、仅真实test binary特权运行；新包后只做4KiB两阶段正常重启读回，不重跑标准/性能或3FS。独立文本/审查与证据收口不受该阻塞。

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
