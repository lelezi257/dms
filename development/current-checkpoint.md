**最新开发/交付入口（2026-10-07，决策）：main唯一入口。** 按用户授权，fix/native-orderly-recovery-20261007相对f09185e的全部有效修改（恢复、归属整改、测试、文档和证据）及本次已验证Node启动失败修复73842cd已无冲突fast-forward纳入本地main，本记录随main正常推送；远端实际提交/文件树以推送核对回执为准。其它分支逐项盘点无当前AFS有效遗漏；8个旧dirty工作树及1个untracked草稿原样保留并在源码树外归档、验证补丁可恢复。不创建PR/MR，不设单特性人工审批；必要Linux门禁继续，整体目标完成后统一项目review。合并不升级历史验收，G1历史8/8不重开。下一仍G2.12 accepted Node/workspace生命周期→G2.13性能；普通性能FAIL/复杂可靠性/fuser原版API阻塞后置或独立保留。[纳入、未纳入、版本和证据](evidence/20261007-main-convergence/README.md)。下方按版本保留的历史记录不覆盖本入口。

**新增限定通过（2026-10-07，事实）：** Node控制器启动失败现进入既有服务排空/显式关闭链，保留首错且不发ready。df56基底/157-map196a4177，六项Linux源码门禁、5个Node测试及真实拒绝启动28checks通过（Node实际wait1/Meta wait0）。首工具错误格式判据FAIL保留；仅修工具后R2，源码/ELF/环境未变。不是accepted ON、通用drain、POSIX/性能或新试用包；G1历史8/8与G2计数不变。[版本及原始证据](evidence/20261007-ownerfs-bind-node-startup/README.md)。

**新增限定通过（2026-10-07，事实）：** 165d基底/157-map6dd63d46，仅新增test-only真实FUSE用例；Linux fmt/测试编译/严格Clippy exit0，精确1PASS/585未选。实际物理Home ext4→FUSE一级native、双向fresh-open内容、EBUSY保留同一mount、子进程wait0后普通卸载/恢复原FUSE identity及正常outer umount/join通过；独立postcheck11项通过，无残留。不是Node宿主生命周期/full ON/POSIX/性能，新包未部署，G1历史8/8及G2计数/defaultOFF不变。

[当前单用例版本与原始证据](../development/evidence/20261007-ownerfs-bind-core-fuse/README.md)。

**当前整改收口（2026-10-07，事实）：** OwnerFs workspace bind mount核心已归属单个ownerfs/bind_mount.rs，runc保留独立适配层，配置原名兼容、默认OFF。6e基底/157-map fabab19a的新Linux受影响检查通过：25项独立测试（含4真实bind/3rootfs）、fmt/严格Clippy/release构建；独立静态审阅通过。新ELF尚未打包/部署，不继承3cc运行验收；G1历史8/8及G2计数不变。

[整改版本、命令与证据](evidence/20261007-ownerfs-bind-remediation/README.md)。

**当前小阶段收口（2026-10-07）：** source3cc10a2/157-map e15c的新Linux release源码检查及4KiB容器有序恢复PASS，58项驱动检查/独立postcheck通过，四wait0、八服务PID与两容器消失、旧记录/模板/本地Meta目录保留；首collector路径FAIL保留、只修工具复核未重跑产品。不是全ON、性能或通用OwnerFs bind验收。G1历史8/8、G2计数及defaultOFF不变。

**当前入口纠正（事实/决策）：** 既有6d B-Home远端读/写/删除及DFS同步一写两读已有实跑证据，G2.16已完成，本轮未重跑。G2.12新增无runc的核心+真实FUSE限定检查现已通过；下一继续独立Node准入/生命周期边界，再G2.13性能。普通宿主可见性、一般引用排空及混合路径语义仍未验收，不因改名或本次单用例升级完整ON。

[本轮实际版本与证据](evidence/20261007-native-orderly-recovery-runtime/README.md)。[独立整改计划](ownerfs-workspace-bind-remediation.md)。

**最终修复受影响检查（2026-10-07）：** 磁盘已由已归档历史副本移出释放37.64GiB；固定Rust8c6f1d50/157-map e15c，Linux release四门禁通过，普通10PASS/7忽略及root实际7PASS，test ELF SHA884ba8f9。仅此范围，不称新服务ELF/包/重启PASS。下一必要剩余源码检查、构建、新包及4KiB有序恢复；不重跑旧标准/性能，G1/G2计数和defaultOFF不变。

[本次版本、原始命令与完整结果](evidence/20261007-native-orderly-recovery-source/README.md)。

**VM归档移出完成（2026-10-07）：** 用户授权后只移除已完整校验的三历史目录，实际释放37.64GiB，磁盘100%→56%。主机9.49GiB归档保留、源端预移出清单及归档SHA复核一致；保护目录身份/旧ELF SHA不变，未扩盘/清当前缓存/动基线或工具链。磁盘阻塞解除，正在执行原计划最终修复的受影响Linux release检查；新包/实际重启尚未验收。

**构建VM历史归档（2026-10-07）：** 旧evidence/artifacts/logs已流式复制到主机，9.49GiB归档、11,852条目/10,661文件内容及基本元数据一致，源端前后无变化；VM原件保留，尚未释放空间，移除决定待用户答复。当前构建缓存/基线/工具链未动，最终修复验证仍阻塞。[归档回执](evidence/20261007-vm-archive/README.md)。

**Owner标准复用审计（2026-10-07）：** e925→已交付6d OFF普通操作未变，构造/statvfs等价变化及既有守卫支持历史限定复用；pjdf有e925 release，LTP6/短FSx仅dev。当前安装/套件只读身份匹配，未重跑标准、不增加PASS或计数。[审计与原proof索引](evidence/20261007-owner-standard-reuse/README.md)。afs-build满盘仍停分支等待用户决定，草稿验证/新包/实际重启尚未完成。

**当前状态（2026-10-07）：** 3FS基线资格按用户决定留专题，主线容器workspace。main f09185e、产品6d/157/map66/既有ELF包的通过范围保留；新rootfs私有副本/命令续号修复在独立分支fix/native-orderly-recovery-20261007，不视为新可用候选。G1历史8/8，G2为10限定完成/2性能FAIL/2bind进行中/13待验收，bind默认OFF。

**新增/阻塞：** 工具11个独立Linux guards通过；早期Rust10+4有输出但未冻结源码SHA，不计最终版本PASS。静态审阅发现并最小修正非root测试入口，原FAIL保留。afs-build85GiB根盘实际满：误用debug与root工具链下载、后续ENOSPC，已停止构建并求助；最终续号/源码门禁/新包/重启读回均未完成。[版本、失败与证据](evidence/20261007-native-orderly-recovery/README.md)。

**下一项：** 等环境处理决定后一次准入，复用release缓存、普通用户构建、仅真实test binary特权运行；新包后只做4KiB两阶段正常重启读回，不重跑标准/性能或3FS。独立文本/审查与证据收口不受该阻塞。

**历史说明：** 下方旧检查点按原版本/范围保存；其旧下一动作由上方当前入口和验收主表覆盖。当前native偏移/经典锁/watch失败与普通性能失败原始记录均保留。

# 2026-10-06 代码与目标检查点

**决策：** 本次按用户授权，将当前累积产品代码和相关文档一起版本化、发布GitHub。该检查点的Git提交同时标识代码、目标、状态、操作说明和交接；从本仓根执行 `git rev-parse HEAD` 获取当前检出身份。任务进度唯一来源是[三阶段验收表](trial-release-goals.md)。

## 当前范围

**事实：** 已交付g1.5试用包的G1为8/8完成。当前代码包含此后Owner索引/目录维护、DFS同批次完整校验共享、local-file恢复、写权限/未知结果处理、健康/容量语义、安装自检及native私有基础。当前源码与历史g1.5包不是同一输入，不用旧包的通过结果替代本检查点验证。

**旧e925/2b5d35c续跑证据（保持原版本）：** [当前E2E账本](evidence/20261006-e2e-current/README.md)记录Owner完整pjdfstest、Owner固定LTP、Owner/DFS短FSx通过；小读/写功能PASS但性能FAIL（0.5709/0.6297×ext4），小删除完成报告出口。DFS pjdfstest/LTP因statfs ENOSYS未进入断言，不计通过。G1保持8/8；G2逐行状态见[主表](trial-release-goals.md)，[双VM核心/中心恢复及DFS一写两读功能](evidence/20261006-e2e-current/crossvm-r1/README.md)已通过；正式比较/新性能包仍未完成，G3后置。旧发布回执原样保留；本轮文档/证据与R1整改一起版本化，产品154编译输入未改。

**决策：** OwnerFs优先；DFS首先一写多读。先小规模读/写/删除，再扩规模/并发/时间；按case自身依赖推进。Owner local吞吐≥90%ext4，remote与MooseFS持平，DFS在同FUSE/POSIX及三同步durable副本下与3FS持平；噪声容差测前固定。普通使用中的损坏、错误成功、权限绕过和核心恢复错误及时修复。

**同源release补充：** [独立结果](evidence/20261006-e2e-current/release-r1/README.md)：122秒构建，Owner完整pjdfstest及64MiB基础/单机中心有序恢复PASS；小读/写仍性能FAIL0.3477/0.5879，保留原dev失败。release DFS/双VM/LTP/FSx尚未新跑。最新用户性能优先容器workspace挂载访问，普通性能专项暂缓；[R1整改](repository-remediation.md)恢复/夹具验证已过，R2上游API缺口单列阻塞，不修改历史结论。

## 当前默认OFF安装回归

main25a8061/map8ef8b788的release ELF已生成两份逐字节一致的包。无编译器Linux/ext4 VM独立安装后，Owner/DFS各64MiB基础校验、目录fsync、中心Meta有序重启/全内容读回、托管退出0/两个exact mount移除通过。[证据和包摘要](evidence/20261007-installed-off/README.md)、[维护驱动及复现](installed-off-slice.md)。第一轮有限身份结果保留，复核加强安装文件inode/路径与mount ID后仅回归受影响小项。没有重跑标准全集或性能，不重开G1，不关闭G2.27及容器G2.12/13；用户已授权隔离VM安装官方runc，运行时准入已通过；实际OwnerFs容器功能仍待验收。

## 当前组合验证

[Linux验证入口](checkpoints/20261006-current/validate-linux.sh)及[输入绑定器](checkpoints/20261006-current/verify-inputs.py)随代码保存。只能在ARM64 Linux运行，Cargo输出与验证结果放源码树外；命令使用锁定依赖和offline模式，依赖需先按[构建说明](validation.md)准备。

```sh
export CARGO_TARGET_DIR=/path/on/linux-ext4/afs-target
bash development/checkpoints/20261006-current/validate-linux.sh /path/on/linux-ext4/new-checkpoint-results
```

当前执行的精确输入、命令、输出和返回值在[本次结果目录](checkpoints/20261006-current/results/README.md)。文档不计入编译输入摘要；验收驱动、安装脚本及本验证工具单独纳入同一输入清单。结果目录不计入工具输入，避免自引用。没有新跑正式性能、标准集或安装后的多节点恢复，不将源码门禁称为全部产品验收。

## historical-evidence

**事实（保留原版本范围）：** 以下小型结构化结果从研究归档按原字节复制，摘要一致，随GitHub可读取。它们不是本次候选的新测量；完整大ELF、磁盘数据及原始归档仍在研究区，不随Git clone携带。

| 结果 | 可携证明 | 可得结论与限制 |
| --- | --- | --- |
| g1.5试用/Owner B1核心恢复 | [271项审计](checkpoints/20261006-current/results/historical/g1.5-validation.json) | scope、源map、旧ELF、安装及实际恢复检查可查；不是当前输入，也不含完整raw归档 |
| DFS批次优化 | [48次paired结果](checkpoints/20261006-current/results/historical/dfs-batch-paired.json) | 同批次内容与计数/配对结果；热页缓存CPU诊断，非3FS性能 |
| 有限ext4工具 | [24次正向结果](checkpoints/20261006-current/results/historical/ext4-tool-positive.json) | 实际工具矩阵/内容/返回值；非AFS性能及完整evaluator |
| parent FD收益实验结案 | [完整失败报告](checkpoints/20261006-current/results/historical/parent-fd-paired-failure.json) | 48窗口/20对，四项耗时比分别约0.986217/0.982199/0.995844/0.983345，未达原≤0.95；保留FAIL，不反复无输入重跑 |

可携历史文件SHA256：

```text
24f354d7cbf18ac8c534c012f556f5b92ec0b1bd4d5691346b74b6770ca715ae  g1.5-validation.json
9c265bb36d4150dc31ea6c251add419c8eca2a68765f50731cf6b5c68d4cefbb  dfs-batch-paired.json
f29451f7513a54002c0f1529c691ac396209c034c50ff9760cc62d2b833367c6  ext4-tool-positive.json
fd2dbca678dcba28ec07665ddf7d9bcf3e98636af6842a58ca435cfb4ce02c5b  parent-fd-paired-failure.json
```

**事实（历史归档结论，完整raw未包含于本仓）：** Owner B2/B3/B4固定评估器与限定恢复通过；v37 OwnerFs/DFS pjdfstest各236文件/8819 TAP checks、v48远端DFS同全集，无意外失败/skip，28上游TODO登记；ext4标准参考（pjdfstest/FSx/冻结657 LTP命令）通过。它们均保留旧源码身份，不继承当前候选PASS。D20只关闭Owner local容量语义切片；native N2a/N2b1/N2c PF1只关闭身份/权限、服务端及私有引用基础。

研究归档名称供原工作区追溯：`evidence/afs-delivery/g2-owner-{index,structural,subtree}`、`g2-owner-local-d20-r1`、`g2-native-{n2a,n2b1,n2c-pf1}`、`g2-dfs-read-amplification`、`g2-ordinary-tool-qualification-r1`。这些是外部档案位置，不假设独立clone存在。

## trial-artifact

历史0.1.0-g1.5 Linux ARM64包由独立离线安装和中心local-file恢复验证；不是本次重新构建的性能release。本仓含[试用操作说明](../docs/guides/trial.md)与构建/打包脚本，旧包未随Git上传。原工作区产物在 `outputs/releases/afs-g1.5/`，跨电脑需单独传递并校验，不能只checkout旧base b259c44得到该dirty构建。

| 身份 | SHA256 |
| --- | --- |
| g1.5包 | `c767255d4a071fd33fcd5a38fbf9f27d4e939c8892c6201828ef4429b6b88829` |
| g1.5 afs-meta | `93c5528cd410d6f56f1ef87fa51f8da73ad4885e147b75e8a30d5ff87308bdcf` |
| g1.5 afs-node | `43813e8393af59509f1c27f9b16076dbb356f1bb6e481f9f8ca430837883696a` |
| g1.5源map（147输入） | `602a42843427e94c55435420f9460ee38846ca0582c4d209d86c50ececca83bf` |

## native-handoff

[Issue42](https://github.com/lelezi257/dms/issues/42)及[PR43](https://github.com/lelezi257/dms/pull/43)已纳入G2.12/13。既有交接读取head `80b0bca3d9d86a1357aa745bb65abfb567f5623f`，该交接对应草稿未合并分支，不把实验比值当主线生产资格；移植须检查与当前主线重叠的权限/写入/索引改动。

**事实/待验证：** 主线生产native入口仍禁用；已新增管理员实验ON配置及受管接线，尚非可用生产ON；[切片边界](native-workspace-slice.md)。将来要求显式开关、默认OFF，OFF FUSE独立回归，ON经过最终namespace/Root/epoch/Home核验、启动/停止、真正引用排空和重启对账。append/SEEK_CUR、经典kernel POSIX锁、watch/混合mmap、export detach后容器clone仍写等已知缺口必须在ON前闭合；不能用mutex/OFD/lazy detach代替证明。基础模块和测试存在不等于READY或物理排空ACK已完成。

## capacity

**决策：** 按case数据/副本/同时保留量与实测工件峰值准入，结果校验及正常关闭后复用自有空间。日志量先测，不能把完整矩阵100GiB准备条件施加到每个小case。A数据盘32→48GiB可作为独立环境任务，扩容前保存活动memory Meta/FDB状态并验证恢复，不能为此停止不相关服务。本次发布不扩盘、不运行大规模benchmark。

## 下一步

当前G2.04/07/08/11及Owner LTP分支按已记录范围复用；同源码release构建/Owner标准及单机恢复已过，小读写仍FAIL，原dev及release数据都保留，普通路径专项优化暂缓。独立R1仓库整改已完成并发布；R2官方fuser公开API缺口单列阻塞，不能冒称迁移完成。当前DFS本地R1 statfs/固定标准回归已通过，旧失败保留；性能优先容器workspace挂载G2.12必要功能出口→G2.13。bind默认OFF；大规模/复杂可靠性/etcd2GiB专题与Redis后置。

## Review boundary

本次静态审查覆盖配置、Meta/protocol、local-file、Owner私有native/索引、DFS批次/权限、Node健康及发布文件完整性。需确保所有被引用的新源码/驱动/脚本都入Git，并附实际当前验证回执。审查另记录DfsMeta解析权限trait默认退回open_write的未来adapter风险：生产GrpcDfsMeta及当前测试adapter均显式覆盖，未发现当前生产旁路；后续新增adapter需显式实现，见[REVIEW-01](issues.md)。静态审查不替代已列待验收的安装/标准/性能项。
