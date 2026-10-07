**最新开发/交付入口（2026-10-07，决策）：main唯一入口。** 按用户授权，fix/native-orderly-recovery-20261007相对f09185e的全部有效修改（恢复、归属整改、测试、文档和证据）及本次已验证Node启动失败修复73842cd已无冲突fast-forward纳入本地main，本记录随main正常推送；远端实际提交/文件树以推送核对回执为准。其它分支逐项盘点无当前AFS有效遗漏；8个旧dirty工作树及1个untracked草稿原样保留并在源码树外归档、验证补丁可恢复。不创建PR/MR，不设单特性人工审批；必要Linux门禁继续，整体目标完成后统一项目review。合并不升级历史验收，G1历史8/8不重开。下一仍G2.12 accepted Node/workspace生命周期→G2.13性能；普通性能FAIL/复杂可靠性/fuser原版API阻塞后置或独立保留。[纳入、未纳入、版本和证据](evidence/20261007-main-convergence/README.md)。下方按版本保留的历史记录不覆盖本入口。

**新增限定通过（2026-10-07，事实）：** Node控制器启动失败现进入既有服务排空/显式关闭链，保留首错且不发ready。df56基底/157-map196a4177，六项Linux源码门禁、5个Node测试及真实拒绝启动28checks通过（Node实际wait1/Meta wait0）。首工具错误格式判据FAIL保留；仅修工具后R2，源码/ELF/环境未变。不是accepted ON、通用drain、POSIX/性能或新试用包；G1历史8/8与G2计数不变。[版本及原始证据](evidence/20261007-ownerfs-bind-node-startup/README.md)。

**新增限定通过（2026-10-07，事实）：** 165d基底/157-map6dd63d46，仅新增test-only真实FUSE用例；Linux fmt/测试编译/严格Clippy exit0，精确1PASS/585未选。实际物理Home ext4→FUSE一级native、双向fresh-open内容、EBUSY保留同一mount、子进程wait0后普通卸载/恢复原FUSE identity及正常outer umount/join通过；独立postcheck11项通过，无残留。不是Node宿主生命周期/full ON/POSIX/性能，新包未部署，G1历史8/8及G2计数/defaultOFF不变。

[当前单用例版本与原始证据](../development/evidence/20261007-ownerfs-bind-core-fuse/README.md)。

**当前整改收口（2026-10-07，事实）：** OwnerFs workspace bind mount核心已归属单个ownerfs/bind_mount.rs，runc保留独立适配层，配置原名兼容、默认OFF。6e基底/157-map fabab19a的新Linux受影响检查通过：25项独立测试（含4真实bind/3rootfs）、fmt/严格Clippy/release构建；独立静态审阅通过。新ELF尚未打包/部署，不继承3cc运行验收；G1历史8/8及G2计数不变。

[整改版本、命令与证据](evidence/20261007-ownerfs-bind-remediation/README.md)。

**当前小阶段收口（2026-10-07）：** source3cc10a2/157-map e15c的新Linux release源码检查及4KiB容器有序恢复PASS，58项驱动检查/独立postcheck通过，四wait0、八服务PID与两容器消失、旧记录/模板/本地Meta目录保留；首collector路径FAIL保留、只修工具复核未重跑产品。不是全ON、性能或通用OwnerFs bind验收。G1历史8/8、G2计数及defaultOFF不变。

**当前入口纠正（事实/决策）：** 既有6d B-Home远端读/写/删除及DFS同步一写两读已有实跑证据，G2.16已完成，本轮未重跑。G2.12新增无runc的核心+真实FUSE限定检查现已通过；下一继续独立Node准入/生命周期边界，再G2.13性能。普通宿主可见性、一般引用排空及混合路径语义仍未验收，不因改名或本次单用例升级完整ON。

[本轮实际版本与证据](evidence/20261007-native-orderly-recovery-runtime/README.md)。[独立整改计划](ownerfs-workspace-bind-remediation.md)。

**当前状态（2026-10-07）：** 3FS基线资格按用户决定留专题，主线容器workspace。main f09185e、产品6d/157/map66/既有ELF包的通过范围保留；新rootfs私有副本/命令续号修复在独立分支fix/native-orderly-recovery-20261007，不视为新可用候选。G1历史8/8，G2为10限定完成/2性能FAIL/2bind进行中/13待验收，bind默认OFF。

**新增/阻塞：** 工具11个独立Linux guards通过；早期Rust10+4有输出但未冻结源码SHA，不计最终版本PASS。静态审阅发现并最小修正非root测试入口，原FAIL保留。afs-build85GiB根盘实际满：误用debug与root工具链下载、后续ENOSPC，已停止构建并求助；最终续号/源码门禁/新包/重启读回均未完成。[版本、失败与证据](evidence/20261007-native-orderly-recovery/README.md)。

**下一项：** 等环境处理决定后一次准入，复用release缓存、普通用户构建、仅真实test binary特权运行；新包后只做4KiB两阶段正常重启读回，不重跑标准/性能或3FS。独立文本/审查与证据收口不受该阻塞。

# AFS 三阶段目标与独立验收清单

2026-10-06，按本轮用户讨论对齐。本文是目标、优先级、独立验收项和出口的唯一主表，替代旧G2.2a/b/c整套准备优先的执行顺序。旧实验/失败/检查点保留原身份；[完整验收目录](../docs/acceptance.md)按本表分阶段执行。

## 总纲与状态口径

**最新用户优先级（2026-10-07）：** pjdfstest优先保证功能完备性。性能第一优先级是Issue42/PR43的容器内挂载workspace目录访问（G2.12必要功能/安全出口→G2.13性能），显式开关默认OFF。其它普通local/remote/DFS性能可先摸底留数据，未达标的专项优化暂缓，不无输入复测；目标阈值和原FAIL不改。[高优先级仓库整改](repository-remediation.md)R1已验证；R2原版fuser迁移因公开API缺口独立阻塞，其它功能/容器路径继续推进，不重开G1。

**决策：** 先交付简单稳定的试用版，再从小规模核心case提高性能，最后复杂可靠性。Meta：memory演示→local-file持久恢复→etcd→Redis；OwnerFs优先，DFS内部一写多读优先。正常使用的数据损坏、错误成功、越权或核心恢复缺陷立即修复。

**决策：** 小项预先确定范围、样本/资源预算、通过线与本轮止损点；完成后保持完成状态，新候选/新规模另列回归，不无输入重复已过项。卡点仅阻塞依赖它的case，不把全套工具、冷热证明、完整100GiB准备或native缺口作为所有工作的前置。失败收益实验可结案，不无限优化残差，不后验修改阈值。

**口径：** 完成=指定版本/范围有通过证据；进行中=有成果但出口未过；待验收=无本项合格结果；后置=阶段三。每项完成记录source/提交与ELF/包、命令、原始结果、判据和限制；每次汇报新增完成、失败、暂缓和下一项。历史失败结案不等于性能通过。

| 阶段 | 独立交付结果 | 当前可信状态 |
| --- | --- | --- |
| 阶段一 G1 | 同事独立安装OwnerFs/DFS；memory演示；中心local-file Meta可重启恢复 | **已完成，8/8；推荐g1.5**。新候选回归不重开G1；不是完整POSIX/69项认证 |
| 阶段二 G2 | 标准回归；OwnerFs核心性能；DFS一写多读；可开关bind功能/性能 | **进行中，27项：10项限定完成、2项存在性能失败、2项bind进行中、13项待验收**。Owner小读/写性能FAIL；DFS本地R1标准缺口已补齐，旧失败保留；系统对照/新性能包未完成 |
| 阶段三 G3 | 长时间、复杂并发/故障、扩展矩阵和最后的后端 | **后置，13项**；局部证据保留，不称整体验收 |

## 启动前收尾（不重开阶段一）

**当前6d候选补充（2026-10-07）：** [默认OFF独立安装/恢复新回执](evidence/20261007-installed-off-6d/README.md)，Owner/DFS各64MiB精确内容、中心有序恢复、正常清理PASS；35checks，不关闭G2.27全性能出口。同包复现已PASS，[限定试用清单](../docs/guides/trial-6d.md)已审阅，prerelease发布准备中；下一Owner远端小读/删独立摸底；必要ON缺口保留，不死磕相同诊断。

**历史候选补充（2026-10-07）：** main25a8061的默认OFF独立安装、Owner/DFS各64MiB基础与中心local-file有序恢复/正常关闭通过；[版本/证据](evidence/20261007-installed-off/README.md)。属于G2.08当前回归分支，27项计数不变；不是G2.27性能包出口、标准全集或容器ON资格。

| ID | 独立任务与出口 | 当前状态 |
| --- | --- | --- |
| G0.01 | 整理试用版/候选/保留与暂缓改动、实验失败及身份 | **完成**；current-checkpoint.md区分版本、失败与未交付项 |
| G0.02 | 固化本表与导航，完成状态有证据，新项有出口 | **本轮文档更新完成** |
| G0.03 | 收尾代码/文档同步GitHub：可审查提交、身份和验证结果 | **完成（本次代码/文档发布检查点）**；源/工具门禁18条通过，提交包含代码及相关文档，Git身份与结果见current-checkpoint.md |
| G0.04 | 按case实际容量准入；需要时保护旧服务状态后A盘32→48GiB，并验证ext4/原数据/host余量 | 待做；未扩盘，不阻塞可安全容纳的小项 |

## 阶段一 G1：可运行、可测试、中心local-file恢复

**事实：** 按已交付内测试用版验收，不追溯升级为全集POSIX、全部后端、8GiB或8小时验收。[g1.5包与说明](current-checkpoint.md#historical-evidence)、[271项Linux核对](current-checkpoint.md#historical-evidence)、[g1.5范围](current-checkpoint.md#historical-evidence)、[原G1跨节点结果](current-checkpoint.md#historical-evidence)、[g1.5同ELF恢复](current-checkpoint.md#historical-evidence)可复核。

| ID | 独立验收项 | 已通过出口与版本范围 | 状态 |
| --- | --- | --- | --- |
| G1.01 | Linux构建与基础源码门禁 | g1.5的147输入/14命令及271项核对；相关真实FUSE/feature/lint/build | **完成** |
| G1.02 | 可安装、可复现试用包 | g1.5离线无源码/编译器安装、包校验、不同umask字节一致 | **完成** |
| G1.03 | memory基础演示 | g1.5 memory R1、普通用户双mount操作及易失健康声明；新memory R2未跑且不在本出口 | **完成** |
| G1.04 | OwnerFs本地基本操作 | G1/g1.5：64MiB、覆盖、sync/close/reopen/EOF、基本命名空间/权限/锁/mmap | **完成** |
| G1.05 | OwnerFs远端基本操作 | G1跨节点及g1.5同ELF local-file Home/remote写入和精确读回/恢复 | **完成** |
| G1.06 | DFS本地/跨节点基本操作 | G1/g1.5 R1/R2核心文件与A/B读回，local-file R2物理内容/复制记录 | **完成** |
| G1.07 | 中心local-file Meta重启恢复 | g1.5同ELF Meta重启，确认路径/数据/结果保留，可继续写读 | **完成** |
| G1.08 | 同事自检与正常生命周期 | g1.5使用说明、普通用户自检、有序启停、重装/卸载保留状态 | **完成** |

**决策：** G1保持8/8完成。历史标准集证据单列下方；当前优化候选标准回归/核心恢复是G2.04–08，不以未运行的新回归否定已交付G1。G1跨节点读回不等于DFS并发一写多读性能。

## 阶段二 G2：独立标准回归与核心性能

**决策：** 标准集兜底、自定义case补充：pjdfstest完整适用项、LTP固定基础文件/权限/锁子集、FSx短固定种子分别登记；选择/版本/排除项测前固定，不因失败删适用项。专项补中心Meta恢复、跨节点可见性/多读者及安装生命周期。广LTP、长时FSx/差分序列后置。

**决策：** 同接口/数据/屏障/副本/缓存/资源比较。Owner local普通路径吞吐≥ext4的90%；Owner remote与MooseFS持平，中心比值1.0，撤销旧耗时≤0.8；DFS同FUSE/POSIX、三份同步durable条件与3FS持平；bind接近native ext4。测量噪声容差逐case开跑前固定，不按成绩调值、不追1%–2%残差。删除先要求正确性及操作数/秒、耗时/延迟对照报告，没有新增硬比例。

**决策：** 小规模默认64MiB，512MiB扩展与8GiB大case各自独立记录；必要时先1–8MiB明确标注诊断。先C1再多并发。DFS一写多读先一写节点+两个读取视图，再扩读者/节点；记录读者分布、总吞吐及逐读者表现。先校验后统计；无驻留实证仅标buffered/repeat，不宣称hot。完整36组/432样本蓝图和高级冷热/长时矩阵不作为所有小case前置。

| ID | 独立验收项 | 本项出口 | 当前状态 |
| --- | --- | --- | --- |
| G2.01 | OwnerFs内部优化 | 属性/identity、别名/目录索引等冻结切片的算法评价/正确性通过 | **完成（限定成果）**；[B1](current-checkpoint.md#historical-evidence)、[B2](current-checkpoint.md#historical-evidence)、[B1–B4索引](../docs/status.md)；非ext4达标 |
| G2.02 | DFS批次读放大优化 | 同批次共享完整校验，不掩盖错误，Linux/成对诊断/独审通过 | **完成（限定成果）**；[48次诊断/1766项审计](current-checkpoint.md#historical-evidence)；非3FS达标 |
| G2.03 | 小尺度性能工具资格 | ext4正向/错误注入、返回/内容/生命周期校验 | **完成（有限工具）**；[15tests/24正向/46负控制](current-checkpoint.md#historical-evidence)；非完整正式评估器 |
| G2.04 | 当前候选OwnerFs pjdfstest | 完整适用项、TAP/排除/版本/挂载身份可复核 | **完成（历史e925 Owner本地；6d OFF限定复用）**；[逐路径等价/受影响守卫审计](evidence/20261007-owner-standard-reuse/README.md)，非6d标准实跑；e925c5b dev及release各236文件/8819checks/28TODO，0skip/意外失败；[release](evidence/20261006-e2e-current/release-r1/README.md)；[proof](evidence/20261006-e2e-current/r2/owner-pjdfstest-r2/artifacts/std-01-pjdfstest/proof.json) |
| G2.05 | 当前候选DFS pjdfstest | 独立mount/state、完整适用项与逐项结果 | **完成（历史DFS本地R1；当前限定复用）**；0891cbfe/map6161e25b release：236文件/8819checks/28TODO，0skip/意外失败，正常关闭；[新proof](evidence/20261007-dfs-statfs/runtime-r2/std-01-pjdfstest-full/artifacts/std-01-pjdfstest/proof.json)。e925 ENOSYS/0TAP[原失败](evidence/20261006-e2e-current/r5/README.md)保留，非remote/RN标准资格；[6d差异审计/限定复用](evidence/20261007-standard-reuse/README.md)，非6d实跑 |
| G2.06 | 当前候选LTP基础子集 | 固定基础文件/权限/锁清单，完整结果账本 | **完成（历史固定6项，版本分列；DFS限定复用）**；Owner e925 dev固定6/6[限定复用审计](evidence/20261007-owner-standard-reuse/README.md)，未称6d/release实跑；DFS map6161e25b release固定6/6PASS，0TBROK/TCONF/FAIL/TIMEOUT；651库存未选；[DFS新proof](evidence/20261007-dfs-statfs/runtime-r2/std-02-ltp-smoke6/artifacts/std-02-ltp/proof.json)。旧6TBROK保留；不宣称新ELF Owner已实跑；[DFS限定复用](evidence/20261007-standard-reuse/README.md) |
| G2.07 | 当前候选FSx短测试 | 固定种子/时长/文件上限，无内容/长度mismatch | **历史通过（短范围，限定复用）**；[Owner6d OFF复用条件](evidence/20261007-owner-standard-reuse/README.md)；e925c5b Owner/DFS均seed1/请求1000、默认262144字节上限、无mismatch；[Owner](evidence/20261006-e2e-current/r2/owner-fsx-r1/artifacts/std-03-fsx/proof.json)/[DFS](evidence/20261006-e2e-current/r5/dfs-fsx-r1/artifacts/std-03-fsx/proof.json) |
| G2.08 | 当前候选基本组合/local-file恢复 | 受改动影响的Home/remote/DFS核心操作、中心重启、错误及正常关闭 | **完成（当前核心组合）**；e925c5b Owner双VM A写B读/B改A读、rename/delete及中心有序恢复；DFS R2一写两读/中心恢复，正常stop/卸载；[34项+关闭核验](evidence/20261006-e2e-current/crossvm-r1/README.md)。非Node崩溃矩阵，不重开G1 |
| G2.09 | OwnerFs本地小规模读 | 与ext4同条件、内容/EOF正确并达本地目标 | **性能FAIL**；e925c5b 64MiB/C1/5对，功能PASS，dev0.5709/release0.3477×ext4<0.90，摸底数据保留、专项优化暂缓；[证据](evidence/20261006-e2e-current/r4/README.md)，不刷成绩 |
| G2.10 | OwnerFs本地小规模写 | 相同持久屏障，计open/write/sync/close，读回正确并达标 | **性能FAIL**；e925c5b 64MiB/C1/5对，同fdatasync，功能PASS，dev0.6297/release0.5879×ext4<0.90，摸底数据保留、专项优化暂缓；[证据](evidence/20261006-e2e-current/r4/README.md) |
| G2.11 | OwnerFs本地删除 | 固定小文件集合，删除正确，操作性能对照报告 | **完成（限定小项）**；e925c5b 100×4KiB/C1/5对，正确且对照报告已留，无新增比例门槛；[证据](evidence/20261006-e2e-current/r4/README.md) |
| G2.12 | OwnerFs workspace bind mount功能验收（独立开关） | 显式可配置、默认OFF；OFF原FUSE回归；ON受管挂载/启动/停止、必要语义/权限、引用排空及重启对账；不安全配置拒绝 | **进行中**；[交接](current-checkpoint.md#historical-evidence)、[基础资格](current-checkpoint.md#historical-evidence)；[管理员实验接线](native-workspace-slice.md)，默认OFF；[实际受管单容器生命周期/清理PASS](evidence/20261007-managed-workspace/README.md)；[短语义](evidence/20261007-managed-semantics/README.md)：mmap字节/权限PASS，锁/append偏移/watch传播FAIL；[append r8独立数据](evidence/20261007-append-diagnostic/README.md)顺序/128记录PASS，偏移仍FAIL，本轮诊断收口；[经典锁原语/机制边界](native-classic-lock-boundary.md)收口、产品仍FAIL；[活动容器source注入拒绝](evidence/20261007-native-source-rejection/README.md)新有限PASS：真实identity/artifacts不变、20B合法exec及收尾；[活动控制额度耗尽](evidence/20261007-native-control-capacity/README.md)新有限PASS：63busy/64ledger、ENOSPC无副作用、Status及Stop收尾；[有序恢复窄修复](evidence/20261007-native-orderly-recovery/README.md)原ENOSPC及草稿身份保留；[3cc新包4KiB两阶段有序恢复](evidence/20261007-native-orderly-recovery-runtime/README.md)限定通过；独立高优先级[命名/模块归属整改](evidence/20261007-ownerfs-bind-remediation/README.md)已完成限定收口，通用宿主可见性/独立bind生命周期仍待功能验收；完整ON及生产开关未资格化 |
| G2.13 | OwnerFs workspace bind mount性能验收（独立开关） | 同候选OFF/ON/ext4配对；核心数据读写和元数据接近ext4，内容/语义正确 | **进行中**；[当前6d51aeb小配对诊断](evidence/20261007-container-perf/README.md)：OFF/ON各1预热+5轮，ON写/读耗时1.023/1.031×ext4、六元数据1.048–1.158×ext4；内容/清理通过，缓存/FUSE计数限制及锁/append/watch缺口保留，完整出口未过 |
| G2.14 | OwnerFs远端小规模读 | 同Home/缓存/接口MooseFS对照，内容正确并持平 | **正式待验收；小数据已留**；6d B-Home/C1/64MiB/1预热5配对，内容PASS，427.371/14854.399MiB/s、配对比0.028803；缓存未观察/旧负载限制，不计持平；Moose客户端wait1清理FAIL保留。[证据](evidence/20261007-owner-remote-small/README.md) |
| G2.15 | OwnerFs远端小规模写 | 同持久屏障/数据量，跨节点读回正确并持平 | **当前6d小功能/清理PASS、数据已留，正式待验收**；A远端/B-Home，64MiB/1预热5配对，12内容校验+B6fresh全量读、6正常退出PASS；234.630/430.767MiB/s，配对比0.528615。缓存未观察/波动、Moose强持久ACK基线独立BLOCKED，不计持平。[当前证据](evidence/20261007-owner-remote-write-small/README.md) |
| G2.16 | OwnerFs远端删除 | 跨挂载可见性正确，操作性能对照报告 | **完成（当前限定小项）**；6d/100×4KiB/1预热5配对，12sample PASS、B600路径ENOENT、6实际wait0；Owner1049.602/Moose1542.929ops/s、配对比0.680266，无硬比例。旧wait1 FAIL保留。[完整证据](evidence/20261007-owner-remote-delete-small/README.md) |
| G2.17 | OwnerFs本地大规模读 | 8GiB顺序核心case，内容正确并达本地目标 | **待验收** |
| G2.18 | OwnerFs本地大规模写 | 8GiB同持久屏障，读回正确并达本地目标 | **待验收** |
| G2.19 | OwnerFs远端大规模读 | 8GiB同条件MooseFS对照，正确并持平 | **待验收** |
| G2.20 | OwnerFs远端大规模写 | 8GiB同持久语义MooseFS对照，正确并持平 | **待验收** |
| G2.21 | DFS小规模一写多读 | 一写确认、多读者相同数据，逐读者/总吞吐与3FS对照 | **当前6d R2同步小功能/数据PASS，正式性能待验收；DFS最高优先级**；64MiB一写/B+C各1预热5读，前后全SHA/EOF、4AFS wait0/3stdio rc0；同一ctl公共窗口中位45.149、B/C纯C50.326/22.716MiB/s。实际A+B durable、uniform去重/缓存未观察，不计三同步/3FS。[同步证据](evidence/20261007-dfs-sync-read-small/README.md)；[旧含启动/预检/预热父窗口](evidence/20261007-dfs-manyread-small/README.md)保持原范围，非改善对比；[历史e925功能/核心恢复](evidence/20261006-e2e-current/crossvm-r1/README.md)保留原版本 |
| G2.22 | DFS单节点小规模读 | 同副本/接口/缓存与3FS对照，正确并持平 | **待验收** |
| G2.23 | DFS单节点小规模写 | 三份同步durable，屏障/读回正确并持平 | **待验收** |
| G2.24 | DFS多节点读写 | 固定文件/并发，读写分别验收，不用平均数掩盖失败 | **待验收** |
| G2.25 | DFS删除 | 固定文件集合，删除正确，操作性能对照报告 | **当前功能/摸底完成，正式对照待验收**；6d候选29.861/30.682ops/s，[原候选证据](evidence/20261007-dfs-delete-small/README.md)；patched3FS R2一次615.210/509.565、B/C各600ENOENT通过；A超预算4,067,328B/FDB-15使原资源/关闭资格FAIL，18owned已消失，不调判据/不重测。[完整新数据及失败](evidence/20261007-threefs-delete-small/README.md)。非3FS持平/物理回收/复杂故障 |
| G2.26 | DFS大规模一写多读 | 512MiB/8GiB分别留结果；读者正确、总/单读者与3FS对照 | **待验收**；各规模独立验收 |
| G2.27 | 核心性能版本交付 | 同候选已选核心case/必要组合回归、可复现包、独立安装/恢复及完整状态报告 | **待验收**；[6d OFF安装/恢复](evidence/20261007-installed-off-6d/README.md)和[同包复现/试用清单](evidence/20261007-off-trial-handoff/README.md)分项已过，固定prerelease已发布并核对；完整选定性能/组合出口仍待验收，native保持OFF，不称G2全表完成 |

**决策：** native已知append、实际kernel锁、混合mmap/watch、最终namespace/Root/epoch及排空缺口属于G2.12启用前条件，不能延期后冒充通过。OFF版本和普通FUSE性能独立推进；开关不掩盖ON错误，当前不声称已有可用生产开关。

**当前独立出口（2026-10-07）：** G2.16限定完成。G2.25两边小删除功能/量化已留数据，3FS A预算与FDB退出判据FAIL原样保留，不关闭正式项。按用户选择，3FS基线资格留专题、普通性能调优后置；主线回容器workspace必要语义。OFF限定试用下载/安装恢复/复现已过，G1 8/8及G2计数不变。

**前序顺序（范围保留，当前动作以上述出口为准）：** 当前已启动release切片已测完并正常停止，普通读写未达标数据保留、专项优化暂缓；R1仓库整改验证完成并发布，R2原版fuser公开API缺口独立阻塞。DFS本地R1受影响标准回归已完成；其它标准项按功能范围复用或补齐，不为了性能反复跑标准集。性能优先G2.12容器workspace挂载的必要功能/安全出口→G2.13性能。其它核心性能按独立摸底项保留数据，不无限优化；大规模/复杂可靠性/后端仍后置。

## 阶段三 G3：复杂可靠性及最后的后端

| ID | 独立验收项 | 出口 | 当前状态 |
| --- | --- | --- | --- |
| G3.01 | 8小时长时间运行 | 持续负载正确、资源有界，无卡死/错误成功 | **后置**；未整体验收 |
| G3.02 | 广LTP/POSIX矩阵 | 更广syscalls/权限/锁及适用项完整账本 | **后置**；保留历史单项/标准结果 |
| G3.03 | FSx长时/差分随机 | 原长时固定种子FSx、10×10000差分及缩减反例 | **后置**；ext4参考有证据，最终候选未完整验收 |
| G3.04 | 节点异常退出/恢复 | Home/remote/DFS分别检查确认水位及恢复 | **后置**；有代表性切片 |
| G3.05 | 网络/未知提交故障 | 断网/超时/ACK丢失/重连不错误成功/重复提交/损坏 | **后置**；有D18等局部闭合证据 |
| G3.06 | 磁盘故障/容量/损坏修复 | ENOSPC/EIO/坏副本/修复及错误可见逐case通过 | **后置**；有EIO/修复切片，D20仅local闭合 |
| G3.07 | 复杂并发/native扩展 | append/rename/撤权/mmap/多Agent的复杂组合与长时 | **后置**；不代替G2.12必要安全条件 |
| G3.08 | RDMA异常/资源生命周期 | CQ超时、迟到完成、引用/设备拆除不越权/损坏/泄漏 | **后置**；有posted deadline等限定证明 |
| G3.09 | 复杂性能矩阵 | 扩展随机/并发、专用cold/真实hot等完整原矩阵 | **后置**；不将无驻留证据repeat升级hot |
| G3.10 | 多Meta/HA | 权威切换、恢复及一致性 | **后置** |
| G3.11 | 更广部署与最终组合 | 剩余安装/升级/内核/拓扑等适用矩阵及完整账本 | **后置**；正式69 NOT_RUN/ENV PREPARING，不是G1进度分母 |
| G3.12 | etcd后端 | 功能/重启/故障/资源，D17专题暂2GiB | **后置，倒数第二**；已有局部恢复/失败 |
| G3.13 | Redis后端 | 持久配置、功能/恢复/故障/资源一致性 | **最后/TODO**；已有实现/局部证据 |

## 已完成历史证明和结案（不重复计入当前候选PASS）

| 成果 | 已完成内容 | 边界/证据 |
| --- | --- | --- |
| pjdfstest全集 | v37 OwnerFs/DFS各236文件/8819 TAP checks；v48远端DFS同全集；无意外失败/skip、28上游TODO登记 | [旧版本状态](../docs/status.md)；不是g1.5/当前候选全集 |
| ext4标准参考 | pjdfstest/FSx、预冻结657命令LTP及差分参考已有通过 | [参考状态](../docs/status.md)；不是AFS标准全集通过 |
| D20本地容量语义 | statvfs/stat/df、权限/错误、Linux门禁/独审 | [本地切片](current-checkpoint.md#historical-evidence)；不是整个D20/测试盘容量已足 |
| native基础 | N2a身份/权限、N2b1服务端、N2c PF1私有引用分别资格化 | [N2a](current-checkpoint.md#historical-evidence)、[N2b1](current-checkpoint.md#historical-evidence)、[PF1](current-checkpoint.md#historical-evidence)；非生产ON/READY/全排空 |
| parent FD收益实验 | 48窗口/20对、正常关闭和失败审查结案 | [结果](current-checkpoint.md#historical-evidence)：未达原≥5%改善，非优化PASS，无新输入不重跑 |

## 当前边界

**事实/决策：** G1完成，完整G2仍ACTIVE；本次将当前代码、三阶段表和相关文档共同上库；新Linux源码/工具验证见current-checkpoint.md，G2标准/性能/安装后恢复仍按独立项验收。后续编译/运行/验证仅ARM64 Linux；本次按用户明确授权同步刷新跨电脑handoff。旧结果/失败/包/检查点不改写。

**决策：** 容量按case实际守卫；基线/候选顺序运行、结果校验/关闭后复用自有空间；同时保留两份8GiB才至少20GiB加实测开销。日志先量化，A扩盘保护活动memory Meta/FDB状态并停机；薄置备不是host物理空闲。见[容量分级](current-checkpoint.md#capacity)、[NEXT](current-checkpoint.md)、[CURRENT](current-checkpoint.md)。
