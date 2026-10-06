**当前新增通过（2026-10-07）：** 同Rust6d51aeb/map66dbbe3e/157输入，已有6d默认OFF包在无编译器ARM64 Linux afs-g1-clean独立安装；Owner/DFS各64MiB完整内容/EOF、中心local-file Meta有序恢复、执行文件inode/路径及挂载身份、正常退出/卸载均PASS，35checks/独立postcheck通过。[版本、命令与原始证据](evidence/20261007-installed-off-6d/README.md)。未改产品或维护驱动，复用旧7个工具guards；未重跑标准/性能，不重开G1，不关闭G2.27全性能/ON出口。当前可交付限定OFF候选；下一独立项为同包复现及试用交付清单，append/锁/watch缺口保留OFF，停止无输入复测。

**当前新增（2026-10-07）：** 相同Rust6d51aeb/map66dbbe3e/157输入，append-only r8补齐独立结果：顺序52B双路径内容、并发128唯一完整记录PASS；第三SEEK_CUR24/38及62个并发偏移不匹配FAIL，原生对照全部PASS，正常清理/独立postcheck PASS。[原始命令、版本和证明](evidence/20261007-append-diagnostic/README.md)。原r6/锁/watch失败与性能摸底保留，不升级ON/G2.12/13；默认OFF，G1历史8/8和G2计数不变。本项诊断收口，偏移一致性待修；下一独立项为当前OFF试用包安装/核心恢复回执，不继续无输入复测本缺口或标准/性能。

## 前序检查点（原版本/范围；下一动作由上述当前入口覆盖）

**当前容器小项（2026-10-07）：** Rust main6d51aeb/map66dbbe3e不变，实际受管单容器基础生命周期、64MiB及正常清理已通过；本轮短混合语义新增4KiB mmap双向数据及权限/错误PASS，锁冲突、append/SEEK_CUR和跨路径watch传播FAIL，[逐项原始证据](evidence/20261007-managed-semantics/README.md)。未执行的混合并发append不计通过；旧锁阻塞判据单列补强。完整G2.12/生产ON/G2.13仍未通过，默认OFF，G1历史8/8和G2计数不变。下一项容器workspace小规模性能仅作诊断留数，不用数字掩盖语义缺口；未变标准及普通性能不重复。下方旧记录保留原版本。

**Latest functional output (2026-10-07):** local R1 DFS statfs and affected fixed standards now pass on map6161e25b release, with normal cleanup; [evidence](evidence/20261007-dfs-statfs/README.md). Do not repeat unchanged suites. [Managed container workspace source adapter](native-workspace-slice.md) now exists as a default-OFF partial experiment; [official runc installation/runtime-only admission passed](evidence/20261007-runc-runtime/README.md) after human authorization; actual managed OwnerFs lifecycle remains pending. Source gates do not qualify container lifecycle/performance. Ordinary performance tuning stays deferred.

# Delivery task map

**Current installed OFF branch (2026-10-07):** main25a8061 reusable release binaries now have fresh compiler-free installation, Owner/DFS64MiB integrity, orderly central local-file recovery and managed cleanup proof. [Evidence](evidence/20261007-installed-off/README.md). No standard/benchmark rerun, G1 reopening or G2.27/container exit claimed. Actual container lane still waits on the missing-runc environment question.

The active task map is [the three-stage acceptance checklist](trial-release-goals.md). This file gives execution order and dependencies; it does not replace the checklist or the full contract in [docs/acceptance.md](../docs/acceptance.md).

## Current checkpoint

G1/g1.5 is complete in its scoped trial sense: installable OwnerFs/DFS, memory demo, basic local/remote behavior, DFS basic behavior, local-file Meta restart recovery and colleague-facing lifecycle/self-check evidence. That result remains historical and bounded. It is not a current-candidate full POSIX, G2 performance or formal 69-case PASS.

G2 is active. The completed G2 items are bounded internal outputs: OwnerFs indexing/structural optimizations, DFS read-batch amplification reduction, and a limited small performance-tool qualification. Bind/native has real progress but remains in progress; the feature must be separately accepted and default OFF. Most current-candidate standard and performance items remain unrun.

G3 is deferred. Long soak, broad random/POSIX matrices, complex fault matrices, multi-Meta/HA, etcd 2 GiB topic and Redis are not the next blockers for small usable progress.

**High-priority independent maintenance:** the release slice has finished and stopped. Publish [R1 evidence cleanup / R2 upstream gap report](repository-remediation.md), then continue independent E2E items; R2 stays BLOCKED pending the lock support decision. G1 and historical conclusions remain unchanged.

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
