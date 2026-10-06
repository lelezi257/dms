**当前发布事实（2026-10-07）：** [默认OFF试用版本](https://github.com/lelezi257/dms/releases/tag/afs-trial-6d51aeb)已发布并远端核对：三资产SHA/大小一致，tag精确指向产品6d51aeb，prerelease，正式Latest仍v0.1.0。同包复现/限定试用交付小项完成；复用35checks安装恢复，不计完整G2.27/性能或ON。下一Owner远端64MiB读/100×4KiB删除新夹具准入与摸底，保留全部旧FAIL及R2阻塞；G1历史8/8、G2计数不变。

**当前交付小项（2026-10-07）：** 同Rust6d51aeb/map66dbbe3e/157输入，已有OFF包在Linux umask077下重打与已测原包完整字节一致（ee25d589，14,728,435B），复现PASS；复用35项安装/恢复PASS。[复现命令与Git输入](evidence/20261007-off-trial-handoff/README.md)、[版本化试用清单](../docs/guides/trial-6d.md)已审阅，固定prerelease发布准备中，不关闭G2.27全性能/ON出口。远端只读准入区分A旧helper保留4GiB不足与新case未冻结合同；B新夹具待准入，不降低64MiB或清理旧环境。[实际库存](evidence/20261007-owner-remote-admission/README.md)。下一独立项为Owner远端小读/删新夹具和数据，普通优化暂缓；G1/G2计数不变。

**当前新增（2026-10-07）：** 相同Rust6d51aeb/map66dbbe3e/157输入，append-only r8补齐独立结果：顺序52B双路径内容、并发128唯一完整记录PASS；第三SEEK_CUR24/38及62个并发偏移不匹配FAIL，原生对照全部PASS，正常清理/独立postcheck PASS。[原始命令、版本和证明](evidence/20261007-append-diagnostic/README.md)。原r6/锁/watch失败与性能摸底保留，不升级ON/G2.12/13；默认OFF，G1历史8/8和G2计数不变。本项诊断收口，偏移一致性待修；下一独立项为当前OFF试用包安装/核心恢复回执，不继续无输入复测本缺口或标准/性能。

## 前序检查点（原版本/范围；下一动作由上述当前入口覆盖）

**当前容器性能诊断（2026-10-07）：** 相同Rust6d51aeb/map66dbbe3e/157输入，真实容器内OFF/ON与同卷ext4完成C1、64MiB同步写/读、1000×4KiB六项元数据，各1预热+5配对。内容/正常清理PASS；ON耗时中位数写1.023、读1.031、元数据1.048–1.158×ext4。仅诊断留数，缓存未观察/FUSE计数NOT_OBSERVED，锁/append/watch缺口未闭合，不升级G2.12/13或生产ON；G1/G2计数不变。[原始数据/身份/命令](evidence/20261007-container-perf/README.md)。普通性能及未变标准不重复。下一独立小项append/SEEK_CUR功能缺口。

**当前容器小项（2026-10-07）：** Rust main6d51aeb/map66dbbe3e不变，实际受管单容器基础生命周期、64MiB及正常清理已通过；本轮短混合语义新增4KiB mmap双向数据及权限/错误PASS，锁冲突、append/SEEK_CUR和跨路径watch传播FAIL，[逐项原始证据](evidence/20261007-managed-semantics/README.md)。未执行的混合并发append不计通过；旧锁阻塞判据单列补强。完整G2.12/生产ON/G2.13仍未通过，默认OFF，G1历史8/8和G2计数不变。下一项容器workspace小规模性能仅作诊断留数，不用数字掩盖语义缺口；未变标准及普通性能不重复。下方旧记录保留原版本。

**新增环境出口（2026-10-07）：** 用户授权后在隔离afs-g2-micro安装固定官方runc v1.5.2，SHA/资产digest/GPG及真实非root容器创建/执行/正常停止删除PASS。[证据](evidence/20261007-runc-runtime/README.md)。仅运行时准入；产品map8ef8b788/ELF未变，G2.12/13未闭合，G1和G2计数不变。下一项实际OwnerFs受管容器，使用干净受信rootfs，不重跑未变标准/普通性能。

**最新源码切片（2026-10-07）：** 默认OFF的受管单容器workspace实验接线，157编译输入map8ef8b788；Linux562库/4实际native挂载/8实际FUSE/接口、严格lint/build及helperTERM通过，复用未变的r3结果。[证据及失败记录](evidence/20261007-native-workspace/README.md)。[官方runc安装/运行时准入已通过](evidence/20261007-runc-runtime/README.md)，容器OwnerFs ON功能/性能仍未资格化，不继承旧标准PASS。

**最新源码增量（2026-10-07）：** 基于2b5d35c的四文件DFS statfs修复，154编译输入map6161e25b；Linux库553/实际FUSE8/feature/lint/build及新ELF DFS本地R1 pjdfstest236/8819、固定LTP6/6、正常关闭通过。[独立回执](evidence/20261007-dfs-statfs/README.md)。未继承新ELF Owner标准、多节点恢复或性能资格。

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
