**当前出口（2026-10-08，G2.14）：** bind ON远端READ输出所有权单次64MiB/C1试改，吞吐366.844→360.850MiB/s（-1.63%），独立pread p95 5.630→5.339ms（-5.16%），未过测前双保留线；限定功能PASS/测量COMPLETE/优化REJECTED，四产品文件恢复main原160输入，补丁/tests/ELF及负面数据可恢复。不重跑此方向。仅合入独立时延探针边界修正及对应维护工具/测试；历史含oracle时延不重标。DFS容量项按用户选择暂缓，G1历史8/8关闭、原G2仍12/0/15，b80 ON交付及既有READ/DFS读改善保持原身份，正式Moose/3FS仍待验。下一定位远端请求/服务处理主要成本，不重复复制微调。[版本、测量和恢复证据](../development/evidence/20261008-owner-remote-read-payload/README.md)。

以下日期记录保留原版本与结论。

**2026-10-08 当前新增（事实/决策）：** 新候选e6f基底/158输入mapd053a333的host bind ON生产RootCommand接收、精确Home拒绝及正常关闭限定PASS：59 Rust+5 Python针对性测试、58运行检查、46独立保存数据核验及Linux实际恢复通过。真实Store两提交/生产RPC日志匹配，Node自然wait1、Meta及bootstrap正常wait0，原4KiB/身份/权限不变；测试发行器不入普通包。Strict all-target/Owner-only Clippy被未修改文件既有lint阻塞，失败保留；受影响Clippy带既有warnings通过。Host ON控制错误fail-closed，OFF策略未改；不称生产issuer/durable ACK/full bind或性能。G1历史8/8/G2总数/defaultOFF不变。下一新候选default-OFF包复现/安装和OwnerFs+DFS local-file核心恢复，标准/八bind性能按身份复用；复杂可靠性和点优化后置。[版本、边界、原始证据](../development/evidence/20261008-workspace-bind-root-command/README.md)。以下保留原时点记录。

**2026-10-08 当前事实：** f03/7bfc的三同步副本正常Meta重启读回已补齐：三Node/挂载身份不变，前后48物理份及A/B/C全64MiB SHA/EOF，五actualwait0、12保护进程和预算通过。副本观察器15 Linux针对性测试、506保存数据检查及605raw/60guest实际恢复通过；R1原FAIL保留，51→54历史回执按三当前serving节点计数，无产品/vendor/环境变更。仅正常进程恢复，不称崩溃可靠性或3FS性能。G1历史8/8/G2总数/defaultOFF不变。 [当前版本、证据和下一项](../development/evidence/20261008-dfs-r3-recovery-qualified/README.md)。 以下保留原时点。

**2026-10-08 当前新增（事实/决策）：** 固定f03/7bfc复用停止的local-file/R3夹具，仅Meta正常重启，三Node/挂载/UDS身份不变、重启前A/B/C全64MiB SHA/EOF与48物理份通过，五actualwait0/12保护进程和预算闭合。原观察器重启后把同三节点跨epoch六历史回执计成六副本而FAIL，后续物理检查/读回NOT_RUN；post-closure实体及代码支持观察器判据不匹配，不称产品六物理副本或恢复PASS。Linux6独立guards/137保存数据检查、600raw/51guest及工具/失败脚本实际恢复通过；所有首失败保留，无产品修补/重跑/环境变更。G1历史8/8、G2总数/defaultOFF不变。下一独立小项：副本观察器跨epoch唯一serving-node/设备/catalog-floor/失效authority针对性覆盖，再补受影响R3正常恢复读回；f03 R1和旧标准直接复用，7e6 R3保持历史，点优化/3FS资格/复杂可靠性后置。 [版本、FAIL及证据](../development/evidence/20261008-dfs-r3-meta-recovery/README.md)。 以下保留原时点。

# AFS 交付验收规范

**现行范围（2026-10-08）：** [三阶段独立验收主表](../development/trial-release-goals.md)决定当前交付范围、顺序与性能目标。G1内测试用出口已完成；新候选标准回归和核心性能独立列G2；复杂/长时/全矩阵与etcd/Redis列G3。下文完整用例目录不是每个小case或G1的统一前置。普通OwnerFs本地和远端核心读写统一按[普通OwnerFs性能准则](../development/ownerfs-performance-criteria.md)验收：吞吐>=同条件MooseFS的1.2倍，操作时延<=同条件MooseFS的0.8倍，两项分别测量且同时满足；正确性/权限/持久语义不豁免。

本规范定义 OwnerFs 和 DistributedFs（DFS）的交付目标、固定实验环境和验收用例。开发、回归和发布使用同一份合同；实现完成度见 [实现状态](status.md)。接口或类型存在、测试被跳过、环境无法启动，都不等于验收通过。

## 1. 交付目标

### 1.1 架构与产品目标

AFS 是近计算文件系统。应用通过普通 Linux 文件接口访问本机 `afs-node` 的 FUSE mount；OwnerFs 和 DFS 是 Node 内的两个后端，使用独立 mount。Node 访问集群内的 `afs-meta` 管理命名空间、写权限、版本和副本位置，文件字节通过本地磁盘和 Node 间 P2P 搬运，不经 Meta。

| 范围 | 交付要求 |
| --- | --- |
| OwnerFs | 1～4 节点 workspace，Home 本地普通文件，远端经 P2P 访问 Home；保留本地亲和性 |
| DFS | 通用多读多写文件语义；immutable Chunk、可变文件布局、版本提交、稀疏文件、可配置副本和故障修复 |
| Meta | G1以memory演示及中心local-file重启恢复交付；etcd、Redis可替换持久后端列G3最后，Redis不是缓存。后端切换不改变文件、提交和错误语义 |
| 数据路径 | 小数据 inline、大数据通过受控数据面；gRPC 和真实 RDMA 文件读写路径均交付 |
| 部署 | Linux 安装包，一条命令安装及启动进程；支持状态检查、停止、重启、卸载；不要求 systemd 或 Kubernetes |
| 本期排除 | DFS SDK、VerifiedCache/Seed 缓存扩散、外部 Spill、OwnerFs 转 DFS Snapshot、容器或 MicroVM 专用适配器 |

DFS 必须能从多个持久副本读取并在源失败时换源；这不依赖本期排除的缓存扩散功能。Replica count 是初始化配置，验收覆盖 1、2、3、4，不支持在线改策略。OwnerFs 不因远端访问而增加持久副本。

第一阶段部署单个 `afs-meta` 进程，交付持久恢复和进程重启；重启前确认旧进程已停止。部署工具负责本次部署的 PID/端口检查和重复启动防护，不承担跨主机 Meta 选主或防双活协议。第一阶段不支持两个独立 Meta 实例同时操作同一 filesystem 后端。

etcd可使用三成员持久后端；其选主不等于AFS Meta服务选主。etcd及Redis单实例持久恢复按G3.12/13独立验收，不作为G1/G2交付门禁。Meta选主、切主、跨实例fencing和Redis自动主从故障转移见[后续TODO](#10-后续能力-todo)。故障时可以拒绝服务，不能以成功响应隐藏状态丢失。

### 1.2 可验收目标

| 类别 | 判据 |
| --- | --- |
| 功能 | 所有适用的必测用例通过，数据、长度、属性、返回值及 errno 与合同一致 |
| POSIX | pjdfstest 全集、固定的 LTP 文件系统子集、FSx 和差分随机测试通过；排除项在执行前审定，不接受运行失败后删除测试 |
| OwnerFs 性能 | 普通本地、远端读写吞吐分别>=同条件MooseFS的1.2倍，操作时延分别<=同条件MooseFS的0.8倍；两项独立验收且噪声容差、判定分位数每case测前固定 |
| DFS 性能 | 同条件、同接口、同副本及持久化要求的读、写任务耗时分别 ≤ 3FS；不以不同任务的平均成绩抵消不达标项 |
| RDMA | 文件读写实际经过 RDMA，并通过故障、资源回收、gRPC 回退和禁用回退测试；记录性能，无硬性速度目标 |
| 可靠性 | 成功完成持久化屏障的数据不因保证范围内的故障丢失；不返回混合版本、坏数据或错误成功；未知提交可恢复 |
| 运维 | 每个故障能通过状态、日志和指标定位到组件、请求及文件；进程重启后可验证恢复状态 |
| 安装 | 全新 VM 无源码、无 Rust 编译器时一条命令部署，完成实际 mount 读写；第二次执行不会删除数据或启动重复进程 |

同数据量同时记录吞吐与操作时延，明确比例方向；普通OwnerFs吞吐默认用配对中位数判定，操作时延默认用测前声明的p95判定；p50/p95/p99、CPU和资源使用同时记录。具体计时边界见第5节和[普通OwnerFs性能准则](../development/ownerfs-performance-criteria.md)。删除先验正确性并报告对照操作数/秒及延迟，不新增硬比例。bind功能及性能分别为G2.12/13，要求显式开关且默认OFF；普通配置默认OFF；当前交付场景必须提供明确ON配置、启用步骤、支持范围/限制和真实跨节点运行证据。G2.12必要功能/ON交付→远端协同和性能→DFS一写多读→普通本地FUSE，详见主表有限出口。

## 2. 文件语义与判定方式

### 2.1 默认一致性

采用 [JuiceFS 默认一致性](https://juicefs.com/docs/community/guide/cache/)作为用户合同：

1. **同一 mount：** 一个进程成功 `write` 后，该 mount 上其他句柄，包括已经打开的只读句柄，能够读取相应修改；长度和相关属性反映本地已接受状态。
2. **不同 mount：** writer 成功 `close` 后，后来 `open` 的 reader 看见这些写入或更晚的已提交修改，即 close-to-open。无并发覆盖时内容必须精确相等。
3. **已打开的远端 reader：** 不承诺 writer 未完成屏障时立即刷新，也不承诺整个 open 生命周期是不可变快照。不能把固定版本的内部读取计划当成普通 open 的 snapshot API。
4. **普通 write：** 可以先在 owner dirty state 可见，不承诺落盘，不要求逐次 write 生成 FileVersion。
5. **成功 close：** 通过可返回错误的 FUSE `flush` 路径排空该句柄先前写入、满足持久化策略并提交恢复所需状态；DFS 随后新 open 可见。OwnerFs 对应 Home 文件及必要 Meta 状态的完成确认。`release` 仅清理资源。
6. **显式屏障：** `fdatasync` 提交数据、Chunk、ExtentMap、LayoutRoot、FileVersion、length/head 等恢复必要状态；`fsync` 另同步完整 inode 属性。父目录项仍需 `fsync(dir)`。

`close` 失败或超时没有成功发布的保证。应用应检查 `write`、`fsync/fdatasync` 和 `close` 返回值；close 错误后不能盲目重试旧 fd，因为 fd 可能已经释放。需要确认内容时重新打开并校验，或使用应用自己的操作身份做恢复。`fsync` 不创建业务 Snapshot 或专门 Blob 发布动作。

同一 inode 的 write/resize/sync/close-flush 由 owner 串行协调。Meta 结果未知时保留 OperationId、版本、布局、receipts 和精确请求，阻塞该 inode 后续修改；应用超时不清除内部 pending。其他 inode 和合法固定版本读取不应被全局阻塞。

### 2.2 POSIX 覆盖集合

先用 Linux guest ext4 跑相同 harness，作为返回值、errno、属性和数据的参考。覆盖：目录与路径解析、create/open flags、read/write/pread/pwrite、append、truncate、seek/EOF/hole、rename、unlink-open、hardlink/symlink、权限/uid/gid/umask、stat/时间、xattr、目录枚举、flock/fcntl 锁、mmap 与同步屏障。

FUSE 支持的普通文件 POSIX 行为是必测。Linux 专有扩展（例如特定 fallocate 模式、OFD locks、特殊文件或特定 ioctl）逐项登记为支持或明确不支持；不支持必须返回正确 errno。不得据此排除普通 rename、锁、mmap、权限或 close-to-open。不能将一个测试集通过描述为对所有 POSIX 条款的形式化证明。

## 3. 固定验收环境

### 3.1 实验规格

**2026-10-07当前环境变更：** B/C数据盘已原地32→42GiB，A/ctl按用户决定保持原容量/运行状态；afs-build未扩容。磁盘映射、恢复证明、50GiB宿主余量及64MiB/512MiB/8GiB预算见[容量管理](../development/vm-capacity.md)。旧证据保留原环境，不追溯修改。

主环境使用本机原生 **ARM64** Linux VM，不用 x86 模拟成绩与原生 ARM 成绩混比。VM 资源是本验收规范选定的实验参数，不是产品最低硬件承诺。

| VM | 规格 | 持久卷 | 用途 |
| --- | --- | --- | --- |
| `afs-accept-ctl` | 2 vCPU / 4 GiB RAM | 系统 24 GiB + 状态盘 8 GiB | Meta、etcd 或 Redis、结果收集；四节点功能 case 时临时运行第 4 个 Node |
| `afs-accept-a` | 2 vCPU / 6 GiB RAM | 系统 24 GiB + 数据盘 32 GiB | Node A、OwnerFs Home、客户端、DFS 副本 |
| `afs-accept-b` | 2 vCPU / 6 GiB RAM | 系统 24 GiB + 数据盘 42 GiB | Node B、远端客户端、DFS 副本 |
| `afs-accept-c` | 2 vCPU / 6 GiB RAM | 系统 24 GiB + 数据盘 42 GiB | Node C、DFS 副本、换源/修复 |

总运行配额 8 vCPU、22 GiB RAM。宿主为 ARM64、10 核、32 GiB RAM。系统使用 Ubuntu 24.04 LTS；环境准备时锁定同一基础镜像 SHA256、具体发行修订和 Linux 6.8 内核包。四台保持一致。单 VM case 只使用 A；测试四节点副本时为 ctl 附加独立 16 GiB 数据卷，结束后卸载清理，ctl 不参加性能数据节点排名。

系统盘、Meta状态盘、数据盘使用guest ext4、virtio块设备；实际数据不能放macOS共享目录、virtiofs、tmpfs或宿主目录。薄置备必须记录实际占用。完整大规模矩阵准备要求宿主可用≥100GiB、运行保留≥50GiB、每台数据卷保留≥4GiB（专用ENOSPC除外）。独立小case按实际数据/副本/工件峰值及保留空间准入；不以完整矩阵容量阻塞可安全运行的小项。基线/候选顺序运行，校验关闭后复用自有空间；需要扩盘先保存活动服务状态，不按日志规模猜测容量。

Guest 能使用 root、`/dev/fuse`、网络故障注入和 `rdma_rxe`。开启 swap 会污染性能判定，性能运行使用关闭 swap 的固定配置；构建在单独的 build VM 中完成，性能测量时不运行构建。`/dev/kvm` 不作为本次文件系统验收前提。

### 3.2 网络和 RDMA

四台 VM 放在一个互通的专用实验网络，固定地址、MTU 1500。准备时从无冲突地址段选出具体 CIDR，写入锁文件；必须证明双向 TCP、UDP/RoCE、TLS 服务和故障注入均工作。普通 Lima NAT 地址相同或只能经宿主端口转发，不算已具备 Node P2P/RDMA 网络。

RDMA 使用 Linux **Soft-RoCE/RXE**。记录 `rdma link`、`ibv_devinfo`、GID、网卡、MTU、内核模块和 verbs provider。安装 rdma-core 或存在 `rdma_rxe.ko` 不等于 RDMA 可用，必须完成独立跨 VM verbs 传输，再验证 AFS 文件数据路径。

RXE 验证协议和资源生命周期，不证明硬件零拷贝、RoCE NIC 性能或硬件掉电行为。RDMA 不可用时，显式 RDMA-required 用例必须失败；普通 TCP fallback 用例可以成功，但不能替代 RDMA gate。

### 3.3 后端和拓扑矩阵

| 维度 | 必测组合 |
| --- | --- |
| 文件后端 | OwnerFs、DFS；同时启动两个独立 mount，防止 inode/handle/缓存策略串用 |
| Meta 持久化 | etcd、Redis；相同文件合同与全部产品功能/可靠性 case 分别执行 |
| etcd | 功能基础 lane 使用 ctl 单成员；后端节点失效 case 使用 A/B/C 三成员，其额外资源成本单列 |
| Redis | ctl 单实例、专用数据库/键前缀、AOF；`appendonly yes`、`appendfsync always`、`no-appendfsync-on-rewrite no`、`maxmemory-policy noeviction`；禁止 TTL 淘汰 Meta 状态 |
| DFS 副本 | 初始化时 R=1、R=2、R=3；R=4 用四节点功能拓扑。主性能 lane 为 R=3 同步副本，另记录 R=1 本地路径 |
| 异步副本 | `desired=3, sync_required=1`，成功只代表当时一个持久副本；不足副本须有持久 repair task/状态 |
| 数据面 | 强制 gRPC、强制 RDMA、自动选择及显式回退；小数据和大数据都覆盖 |

Redis 的 `always` 在回复前同步 AOF；`everysec` 可能丢失最近的写入，不能作为强持久化基线。[Redis 持久化说明](https://redis.io/docs/latest/operate/oss_and_stack/management/persistence/)。单 Meta 在后端确认前不得发布候选状态；不要求后端提供原生多记录事务，但必须满足原子持久状态、幂等重放和 inode owner lease/epoch 校验。Meta 实例之间的 fencing 属于后续高可用设计。

### 3.4 冻结清单与准备 gate

环境准备产出 `acceptance.lock.json` 和 `cases.json`，放在研究区的实验目录，不把动态结果写回产品文档。锁文件必须包含：

- host/VM 架构、CPU/RAM、镜像摘要、内核、磁盘及缓存策略、IP/MTU、时间同步、cgroup 配额、FUSE 参数、RXE 信息。
- AFS SHA、Rust toolchain/Cargo.lock、二进制及安装包摘要；etcd、Redis、MooseFS、3FS、FoundationDB、fio、测试集和 runner 的精确版本/commit/制品摘要。
- mount 参数、Meta 及副本策略、durability lane、测试集合/排除项、随机种子、数据集摘要、超时和结果 schema。
- 每个上游测试 ID 与 AFS case 的映射；包括发现的 TODO、TCONF、SKIP，避免通过 TAP 标题掩盖未覆盖项。

`ENV-01`是完整最终矩阵gate：四VM身份与配额、ext4/TLS/P2P/后端重启/跨VM verbs、上游ext4参考、MooseFS及3FS实际mount读写均通过。独立case只要求其实际依赖先通过；MooseFS/3FS或RDMA未资格化仅阻塞对应比较/数据面，不阻塞独立ext4工具、普通FUSE回归及小规模诊断。未满足的依赖记BLOCKED，诊断不升级正式PASS。

已有 `dms-dev` 等 VM 不是自动合格的验收环境。准备环境不会默认停掉无关服务；开始性能 lane 前必须确保无其他运行 VM 或进程争用实验资源，记录宿主负载。四个 VM 的磁盘仍共享一块宿主 SSD，不能把它们当成独立物理故障域。

## 4. 功能用例

下表保留完整目录的固定case ID。所有读写验证比较内容、EOF/length和errno，并保存调用顺序与后端证据。完整矩阵覆盖两文件后端×etcd/Redis；G1/G2按主表选择memory或local-file及必要拓扑，不要求预先跑完该矩阵。标准集完整适用pjdfstest、固定LTP基础子集、短FSx分别是G2出口；下列更广LTP、900秒×3 seeds及差分10×10000属于G3扩展。

### 4.1 标准测试集

[JuiceFS 的 POSIX 测试说明](https://juicefs.com/docs/community/posix_compatibility/)是测试选型依据；AFS 使用其上游测试思路，不复制它的全部排除项或历史通过数量。

| ID / 名称 | 执行和判据 |
| --- | --- |
| `STD-01 pjdfstest` | 使用 [sanwan/pjdfstest d25636a](https://github.com/sanwan/pjdfstest/tree/d25636a227606f8960e5179741d8f4ad7030ef41)；root 与非 root 配合运行完整 tests，检查全部 TAP 子测试、TODO 和 skip。每一个适用失败都是 gate 失败 |
| `STD-02 LTP-filesystem` | LTP 20260529，锁定 kirk；冻结 fs、fs_bind、fs_perms_simple、fcntl-locktests 及文件相关 syscalls 集合。删除内核无关或环境不适用项时逐项说明，AFS 功能缺失不能当环境排除 |
| `STD-03 FSx` | 使用 [secfs.test edf5eb4](https://github.com/billziss-gh/secfs.test/tree/edf5eb4a108bfb41073f765aef0cdd32bb3ee1ed) 的 FSx，覆盖读写/截断和 mmap；每个固定种子至少 900 秒 × 3 seeds。任何数据或长度 mismatch 失败，失败保留最小操作轨迹 |
| `STD-04 differential-random` | 参考 JuiceFS fsrand/Hypothesis，Linux ext4 与 AFS 对相同操作序列比较；10 固定 seeds，每 seed 10,000 次操作，保留 Hypothesis database 和缩减序列 |
| `STD-05 suite-accounting` | 总发现数 = PASS + FAIL + 审定排除 + 未完成；不得失踪。与 ext4 参考结果不同的 errno 或 skipped 项逐一解释 |

上游参考固定在 JuiceFS commit `adcca1cc61bb4d668a945d64b2e176b44ac8e5b5` 的 [CI workflows](https://github.com/juicedata/juicefs/tree/adcca1cc61bb4d668a945d64b2e176b44ac8e5b5/.github/workflows)。上表是 AFS 自己固定的 suite SHA，不声称 JuiceFS 已固定其所有依赖。runner/依赖摘要在环境准备时写入锁文件。Fsracer 不是本规范强制 gate，也不宣称 JuiceFS 已启用它。

### 4.2 用户可见语义

| ID / 名称 | 场景、步骤和目的 |
| --- | --- |
| `FUN-01 basic-io` | `/xxx.txt` create → 分次 write → pread → close → reopen，覆盖 0、1、4 KiB、64 KiB、1 MiB、跨 Chunk 边界和 8 GiB；证明流式文件不需要预知最终长度 |
| `FUN-02 same-mount-visible` | 同 mount 的只读 fd 先 open，另一个进程 write 覆盖、append 和 resize；不 close 不 fsync 即从原 fd 读到已接受状态。分别测本地 owner/Home，以及 B mount 两个句柄经 A owner/Home 处理的远端场景；必须走真实 FUSE 页缓存，不能只测 VFS trait |
| `FUN-03 close-to-open` | A write 后仅 close，不调用 fsync；记录 close 成功，再通知 B open/read/stat，必须获得新内容和长度。用应用屏障建立先后关系，不能靠 sleep 猜时机 |
| `FUN-04 sync-to-reopen` | A 分别 fdatasync/fsync 后 B 重新 open；检查数据、length/head 和属性保证；保持旧远端 fd 的结果只按远端可见合同判定，不要求永久旧版本 |
| `FUN-05 dup-close` | dup/fork 出多个 fd，关闭其中一个、继续用其他 fd 写，最后关闭；每个成功 close 的先前写入可见，不误清其他句柄状态、不重复提交无变化版本 |
| `FUN-06 sync-flags` | O_SYNC/O_DSYNC 写后直接故障注入，检查相应屏障已完成；用户态 write 成功与 dirty 接受状态区分清楚 |
| `FUN-07 sparse-resize` | 远处 pwrite、shrink→grow、truncate/ftruncate、跨 Chunk 边界；hole 读零、EOF 正确、被截断数据不复活；大洞不实际生成等量零数据 |
| `FUN-08 namespace` | mkdir/rmdir/readdir、rename 替换、hardlink/symlink、open-unlink、路径错误和跨 mount 操作；原子 rename 不出现半状态，跨文件系统 EXDEV |
| `FUN-09 permissions` | 2 个普通 UID/GID、root、umask、chmod/chown、xattr、时间属性；拒绝错误权限，远端不得绕过本地权限检查 |
| `FUN-10 locking-mmap` | 同 mount/跨 mount flock、fcntl 锁及进程退出释放；MAP_SHARED/MAP_PRIVATE、msync、映射写后 fsync/close。测试锁所有权，避免把共享锁错误绑定到所有 peer |
| `FUN-11 concurrent-file` | 8 进程在相同 inode 做互不重叠 pwrite、并发 append、resize 与 sync；按 owner 顺序检查完整记录和已确认前缀。重叠普通写不额外承诺整文件事务 |
| `FUN-12 concurrent-inodes` | 一个 inode 的未知提交被阻塞，其他 inode 继续读写；证明 owner 串行队列不是全局队列 |
| `FUN-13 mount-isolation` | 同 Node 两个独立 OwnerFs/DFS mount 使用相同路径、inode 数字和 fd 操作；身份、权限、错误和数据互不串用 |

### 4.3 后端与分布式数据路径

| ID / 名称 | 场景、步骤和目的 |
| --- | --- |
| `DIST-01 owner-local-remote` | 1/2/3/4 Node 拓扑，Home A；分别本地和 B/C 访问同 workspace；内容和权限相同，远端流量只指向授权 Home，不冒充 DFS 副本 |
| `DIST-02 dfs-remote-owner` | B 写 owner 位于 A 的 inode，resize、sync、close；正确路由和 fencing。不同 mount 的普通只读不因 B 已 open 就被强制读 owner dirty overlay |
| `DIST-03 replicas` | R=1/2/3/4 初始化策略，同一文件分块写；检查独立 Node 上的 durable receipts 和 Meta catalog，不以 ACK 数量代替不同 Node；R=1 本地路径为 0 Peer 数据 RPC |
| `DIST-04 multi-source` | R=3 文件，从 A/B/C 不同副本取不同 Chunk/range，同时有 writer 提交新版本；每次内部读取计划的版本/布局/Chunk 身份一致，不混合不同布局 |
| `DIST-05 source-retry` | 某个 source 中途断连、返回短数据、坏 digest 或缺失 op header；换到合格副本，不将部分 scratch 数据返回应用 |
| `DIST-06 backend-parity` | etcd 与 Redis 各建独立 filesystem，执行同一全量功能/故障集合；重启后版本、路径、幂等结果一致。不把复制 Redis key 称为跨后端在线迁移 |
| `DIST-07 rpc-contract` | 抓取计数/trace：有效 owner 的普通 write 不逐块访问 Meta；R1 有变化屏障批量提交；RN 数据传输在 Node 间；无变化屏障不生成空版本；lease 刷新、重试单独计数 |
| `DIST-08 async-repair` | 1 个同步副本、3 个目标副本；后台复制失败后 task 与不足副本指标保留，恢复源/网络后补足；全部有效源丢失则明确 I/O 错误，不能把失败延迟到毫无提示的某次读 |

## 5. 性能用例与基线

### 5.1 公平比较

MooseFS、3FS 和 AFS 顺序使用同一 VM/卷/网络/数据集/客户端资源。固定版本和全部配置；禁止候选用 RAM、对端用磁盘，或候选单副本、对端多副本。对端的 Meta/存储基础设施消耗计入资源清单，不隐藏 3FS 的 FoundationDB。

普通OwnerFs 本地和远端读写均与同条件MooseFS比较，固定Home/数据位置以确保远端case不变成本地命中；吞吐和操作时延分别验收，必须同时达标。OwnerFs workspace bind mount是单独G2.13，继续与native ext4做OFF/ON/ext4配对。DFS 主 lane 使用 3 个同步 durable copies，与 3FS 有效三副本 chain 对比。若某种副本或持久化合同不能匹配，该项 BLOCKED，不能使用折算系数充当通过。

读写都经 FUSE/POSIX；不拿 3FS USRBIO 数字与本期不含 SDK 的 AFS 混比。3FS 和 AFS RDMA 使用同一 RXE 网络；另外的 gRPC 结果单独报告。RXE 数据只能说明这个 VM lane，不能外推为物理 RDMA 集群的“持平 3FS”。

写任务包含 `open/create → writes → barrier → close`。分别测 close-only、fdatasync 和 fsync 三 lane，要求每方计时结束时已经达到相同 durable/visible 状态；若对端 close 本身不保证落盘，要添加相应同步，并在结果注明。读任务先写好并完成屏障，计时 `open → reads → close`，含远端首次定位和传输，不只计 memcpy。

### 5.2 工作负载

下表是大规模/扩展矩阵规格。G2先独立验收64MiB核心单节点读、写、删除及多节点读、写；512MiB/8GiB各自登记，不等待整张表。DFS先一写确认后多读者，记录逐读者正确性、吞吐及总吞吐。先C1再扩并发；没有驻留证明只称buffered/repeat。每case开跑前固定版本、种子、接口、数据、并发、缓存、屏障、适用副本语义、规模、计时边界、计时器、样本数、分位数算法、噪声容差、容量、停止预算和判定分位数，不按结果变更。

| ID | 固定工作负载 | 门槛 |
| --- | --- | --- |
| `PERF-01 owner-local-read` | A/Home 冷读、预热后重复读；连续 8 GiB、随机 4 KiB/64 KiB；另测 512 MiB 页缓存可驻留的热读 lane；并发 1/8 | 吞吐>=1.2×同条件MooseFS，操作时延<=0.8×MooseFS；同接口/缓存，case测前固定噪声容差和判定分位数 |
| `PERF-02 owner-local-write` | A/Home 连续 8 GiB，1 MiB 写；4 KiB/64 KiB 固定 512 MiB 随机覆盖；并发 1/8，三种屏障 lane | 吞吐>=1.2×同条件MooseFS，操作时延<=0.8×MooseFS，同持久屏障 |
| `PERF-03 owner-remote-read` | B 读 Home A，同 PERF-01；明确冷首次/重复读取，不启用 DFS 缓存 | 吞吐>=1.2×同条件MooseFS，操作时延<=0.8×MooseFS；固定Home/缓存条件 |
| `PERF-04 owner-remote-write` | B 写 Home A，同 PERF-02 | 吞吐>=1.2×同条件MooseFS，操作时延<=0.8×MooseFS，同持久屏障 |
| `PERF-05 dfs-read` | R=3；连续 8 GiB、固定 512 MiB 随机读，4 KiB/64 KiB/1 MiB，并发 1/8；冷、重复读及 512 MiB 页缓存热读分开 | 每项 T_AFS/T_3FS ≤ 1.0 |
| `PERF-06 dfs-write` | R=3；连续 8 GiB、固定 512 MiB 覆盖，4 KiB/64 KiB/1 MiB，并发 1/8；三种屏障 lane | ≤ 1.0 |
| `PERF-07 metadata` | 10,000 个 4 KiB 文件 create/stat/readdir/rename/unlink，并发 1/8，OwnerFs/DFS 分开 | 固定报告，不冒充已约定的读写比例指标 |
| `PERF-08 rdma-profile` | RDMA/gRPC 同数据集，记录 CPU、吞吐、尾延迟、注册/拷贝次数；检查大 payload 未走 gRPC body | 功能硬门禁，无速度门槛 |

随机任务固定 seed，8 GiB 顺序任务可采用 N 个等大小文件保证总字节相等；并发是任务并发而非每个 worker 再增加 8 GiB。所有任务校验内容，失败运行不得进入性能统计。

每项至少 1 次预热、5 个有效配对运行，交替候选/基线顺序。普通OwnerFs吞吐默认以配对中位数判定；操作时延必须来自测前固定的文件事务或逐I/O调用样本数组，p50/p95/p99全部公开，默认以p95作为时延判定分位数，除非case计划在运行前另行固定。既有C工具的聚合wall-time/任务汇总只能作为吞吐诊断，不能当作系统调用或逐操作时延。DFS继续以同条件任务耗时比判定。任一功能错误失败；明显宿主争用/热降频先标记环境无效并重跑，不挑选最快轮。门槛没有默认 10% 容差；证据不足或波动足以改变结论则 INCONCLUSIVE。

冷读在没有其他业务的专用 VM 中清理 guest 页缓存、重建 mount 并证明无产品数据缓存；宿主 APFS 缓存无法完全控制的边界明确记录。8 GiB 超过单台 guest RAM，预热后重复读不能称为全量热缓存；真正热读使用 512 MiB 数据集，记录页缓存驻留及缺页/底层读取量。有限资源下的小集群结果不替代大规模扩展性指标。

## 6. 可靠性用例

必测故障阶段：dirty 接受后、Chunk 写入中、finalize 后、replica ACK 前后、Meta 持久化前后、Meta ACK 丢失、close/fsync 返回前后。每个用例先记录应用已经获知的成功水位，再注入故障；没有水位记录不能证明“不丢已确认数据”。

| ID / 名称 | 操作与判据 |
| --- | --- |
| `REL-01 dirty-crash` | 普通 write 后 kill owner，未屏障数据允许丢失；已持久版本必须可恢复。曾读到 dirty 不代表持久，失败/丢失不得伪造为提交成功 |
| `REL-02 barrier-restart` | 成功 fsync/fdatasync/close 后 kill -9、重启 Node/Meta，另测 guest 硬复位；在配置故障预算内数据与恢复必要状态不丢 |
| `REL-03 finalize-crash` | 每个 staging/验证/publish/catalog 切点注入失败与重启；没有半个 Ready Chunk，没有引用未完成数据的 FileVersion，orphan 有可观测回收结果 |
| `REL-04 lost-meta-ack` | Meta 已持久化但 ACK 被丢弃；重复原 OperationId/精确请求只产生一次 head 变更；同 inode 后续修改被阻塞，其他 inode 正常 |
| `REL-05 meta-restart` | 单 Meta kill/restart，确认旧进程已退出再启动新进程；etcd 单成员与三成员失效、Redis AOF 重启及 rewrite 中断；确认旧 head、幂等结果和 inode 身份；后端不可持久化时不得返回成功 |
| `REL-06 stale-owner` | partition/重启后旧 lease、Node/Device epoch、重放旧 ACK；旧 owner 不可写当前 inode，不可把旧设备证据计入副本数 |
| `REL-07 replica-loss` | R=3 成功后隔离/永久移除一个数据卷，剩余副本仍可读并报告 degraded；加入替换空卷及新 DeviceEpoch，或启用 ctl 第四 Node，再 repair 补足。修复预算从合格目标可用时开始，不能靠同 Node 双副本凑数；R=1 唯一副本永久丢失只能明确不可恢复 |
| `REL-08 capacity-io` | 专用限额卷 ENOSPC、只读设备、注入 EIO、后端 OOM/拒写；write/屏障/close 返回明确错误，不发布损坏布局，不侵占宿主保留空间 |
| `REL-09 corrupt-copy` | 篡改一个 Chunk 文件，读时检测并剔除坏源；RN 换源修复，所有源损坏返回 EIO；hole 不能掩盖丢失 Chunk |
| `REL-10 network` | 连接断开、丢包、延迟、单向 partition、TLS 失败；超时受控，不死锁，恢复后不重复修改或混淆成功失败 |
| `REL-11 handle-cleanup` | 应用崩溃、dup/fork/重复 flush、mount 退出；不泄漏 fd、锁、lease 和注册内存，不把 release 错误当应用已收到的 close 错误 |
| `REL-12 owner-home-loss` | Home 暂时退出/恢复后文件可用；Home 磁盘永久丢失时明确不可恢复和受影响 workspace，不承诺 DFS 型副本修复 |
| `REL-13 namespace-durable` | 文件 fsync 与 fsync(dir) 分开注入崩溃；测试带目录屏障的 create/rename/unlink 恢复，普通文件屏障不能冒充目录持久化 |
| `REL-14 soak` | 8 客户端混合操作持续 8 小时，每 15 分钟可重现故障/恢复；全量摘要正确，无持续 fd/任务/内存增长，没有未解释的 stuck pending |

`REL-15 duplicate-active-meta`保留为后续HA预留ID，不纳入当前69项manifest。`REL-06`防止旧Node修改当前文件，不要求Meta选主；G1中心local-file核心恢复已有证明，复杂故障组合列G3，不把普通使用中的错误成功/损坏/权限绕过延期。

故障 case 的 harness 单操作 deadline 为 30 秒；恢复网络或重启服务后，60 秒内进入可服务或明确失败状态；小数据集副本修复 120 秒内完成。可靠性切点默认使用 64 MiB 文件，不以 8 GiB 性能任务套用这项修复预算；性能任务 watchdog 为 1,800 秒。它们是此 VM 验收的超时预算，不是所有容量的生产 SLA。超时必须保存状态/日志，不能无限等待使 case 假通过。

guest 硬复位和虚拟卷移除只模拟相应故障。SSD 控制器掉电、宿主丢盘、机房断电和三个虚拟副本同时失去宿主，不在单机 VM 实验证明范围内。

## 7. RDMA、可维可测与部署

### 7.1 RDMA 集成用例

| ID | 场景与证据 |
| --- | --- |
| `RDMA-01 verbs-preflight` | 跨 VM RXE 读写/发送探针、完整摘要、设备/GID 记录；是环境 gate，不是产品交付证明 |
| `RDMA-02 file-io` | OwnerFs 远端和 DFS RN/peer-read 的实际大文件读写；trace 关联 file→Chunk 或 Home range→RDMA completion，verbs 字节计数变化，gRPC payload 只含控制或 inline 小数据 |
| `RDMA-03 lifetime` | 成功、取消、超时、peer 掉线、进程退出时 MR/QP/CQ/buffer 回收；不能复用仍在 DMA 的内存，重复运行无增长 |
| `RDMA-04 fallback` | RDMA 初始化/链路故障：自动模式可降级 gRPC 并记录原因；required 模式明确失败，不能悄悄改用 TCP |
| `RDMA-05 integrity` | 边界长度、scatter/range、多 op 完成乱序、坏 descriptor/epoch；鉴权、完整性和 ReplicaAck 保证与 gRPC 等价 |

### 7.2 可维可测用例

| ID | 要求 |
| --- | --- |
| `OPS-01 health` | readiness 区分 Meta 持久化可用、Node 注册、设备可写、mount 可用、RDMA 可用；进程 alive 不等于文件服务 ready |
| `OPS-02 metrics` | 暴露操作成功/错误/耗时、Meta/Peer RPC、dirty bytes、pending commit、durable copies、repair backlog、坏源、磁盘空间、RDMA/fallback 和资源使用；inode/path 不作为无限增长 label |
| `OPS-03 trace` | 从一次 `/xxx.txt` write/sync/close 关联到 Node、OperationId、Meta commit、Chunk/replica ACK；OwnerFs 关联到 Home 操作。日志能回答“卡在哪一步”且不泄漏 token/凭据 |
| `OPS-04 diagnostics` | 一条诊断命令导出版本/配置摘要、进程、端口、mount、设备、后端、近期错误及指标；脱敏，有超时，失败仍输出可读结果 |
| `OPS-05 backpressure` | 8 客户端超量请求时内存、连接、任务和 buffer 有界；返回受控错误/等待，不 OOM。资源上限写入 lock；soak 结束返回同一 idle 基线，无线性增长 |
| `OPS-06 reproducible-runner` | 可按 case ID/类别/backend/transport 运行；输出 JSON + JUnit、原始日志、种子、SHA/环境身份，结果只有 PASS/FAIL/BLOCKED/INCONCLUSIVE/预审排除 |
| `OPS-07 workspace-location-affinity` | OwnerFs 在 A 创建 workspace，经管理 REST `GET /v1/roots/{root_id}` 验证 Home=A、root epoch 和归属 revision；按查询结果在 A 访问时无 Peer 数据 RPC，在 B 访问时转发到 A。Meta/Home 重启后归属和 session 正确；Home 不可服务时保留归属但不可把它报告为健康，未知 root/后端不可用返回明确错误。etcd/Redis 都执行；管理查询不承担实际调度器实现，也不提供自动 Home 迁移 |

位置查询供调度器或管理工具选择计算节点，文件 I/O 仍通过本机 Node。workspace 归属与 Home 在线状态是两个不同字段或查询结果，不能用固定的 `active` 表示 Home 健康。DFS 的 inode owner 与 Chunk 副本分布不等同于 OwnerFs 整个 workspace 的单 Home。

### 7.3 一键安装与进程部署用例

| ID | 场景与判据 |
| --- | --- |
| `DEP-01 clean-install` | 全新 ARM64 Ubuntu VM，仅安装脚本前置条件（root、下载/校验工具、FUSE）；无源码和编译工具，一条命令获取校验过的 release 包、安装、配置、启动、mount 并完成读写/close/reopen |
| `DEP-02 cluster-start` | 一份显式拓扑配置部署 ctl/A/B/C，etcd/Redis 各执行；逐组件 readiness 后宣布成功。不接受只启动进程而 mount 不可写 |
| `DEP-03 idempotent` | 相同配置重复安装/启动，不覆盖已有 Meta/数据卷、不启动重复 PID、不重复挂载；配置冲突明确拒绝 |
| `DEP-04 lifecycle` | status/stop/start/restart；正常 stop 排空已接受工作并报告失败，强杀按可靠性合同恢复；PID 复用不能误杀其他进程 |
| `DEP-05 failure-cleanup` | 后端地址错误、端口冲突、权限不足、FUSE/RDMA 缺失、下载摘要错误；非零返回、解释错误、清理本次临时资源，保留已有数据 |
| `DEP-06 offline-package` | 已取得包后不依赖 cargo/git/在线构建，依赖清单及版本明确；etcd/Redis 可指向用户提供的实例，不强迫覆盖现有服务 |
| `DEP-07 uninstall` | 停进程、卸载 mount、移除本次程序；默认保留数据/Meta 配置，删除持久数据需独立显式选项 |
| `DEP-08 restart-package` | 同版本重新部署保留数据；不兼容格式拒绝打开，不隐式迁移或静默重建 filesystem；跨版本升级不默认成为首期保证 |

## 8. 验收执行与证据

完整扩展矩阵的准备包括环境、参考测试集、对照基线、功能及故障/RDMA/部署/性能和组合回归。当前执行按主表走独立分支：Owner标准/必要恢复→小规模local→remote；进入DFS先标准/核心恢复及一写多读。每项只等待自身依赖；合格对照缺失时可作标注清楚的诊断，不宣称性能达标。基线冻结后改变该case的配置、版本或合同，重测受影响的case。

每个结果包含case ID、参数矩阵、锁文件摘要、开始/结束时间、应用返回值和成功水位、数据校验、命令、日志/指标/trace、故障及恢复记录。性能额外保存每轮原值与比较公式。未执行记NOT_RUN，实际依赖阻塞记BLOCKED；均不是PASS。

各阶段发布gate以[独立验收主表](../development/trial-release-goals.md)为准。G1为已交付试用范围；G2候选须完成选定核心case、受影响标准/恢复回归和独立安装；完整69项/8小时/RDMA异常等留作G3逐项验收，不作为所有小项前置。审定排除、BLOCKED及INCONCLUSIVE单列，不总结成全通过。native未资格化保持OFF并可独立交付普通版本，G2.12/13仍待完成。不得降低已选case的正确性、持久语义、副本或吞掉错误。

## 9. 环境准备需要完成的冻结项

本规范定义完整目录的拓扑、资源、数据量、case和判据。进入相应完整验收项前须形成下列实际制品；独立小项只冻结实际依赖，不能把文档定义当成已有环境：

1. 同版 ARM64 Linux 镜像、专用 VM 网络与 RXE 工作证据；etcd/Redis/工具版本及安装包摘要。
2. 测试脚本、完整用例 manifest、逐项 LTP/扩展 POSIX 适用性表、ext4 参考结果。
3. 固定版本的 MooseFS 和 3FS build/mount 结果及同条件完整性能原始基线。3FS ARM64 源码参考 [22fca045](https://github.com/deepseek-ai/3FS/tree/22fca04564c7cc230fd8b9523b8b92864e1dad47)；源码支持不代表已在这组 VM 上通过 RXE/运行预检。
4. 可复用验收 runner 与 Skill；自动归档身份、命令、结果、失败最小轨迹和基线。

依赖版本与排除项在准备阶段记录精确值后冻结。若 3FS 在本机资源/RXE 下不能完成公平基线，该性能 gate 保持 BLOCKED，单独评审环境调整，不改成“接近公开硬件数字”或用其他文件系统替代。

## 10. 后续能力 TODO

下列能力属于后续目标，分别设计范围、接口和验收标准；当前优先级见G3主表。列入TODO不自动批准实现方案，也不要求G1/G2预先搭建框架。

### 10.1 已明确的后续能力

| ID | 能力 | 需要单独解决的问题 |
| --- | --- | --- |
| `TODO-01` | Meta 高可用 | Meta 实例选主、leader 任期、切主、持久化写入 fencing、旧实例暂停/隔离后恢复、未知提交与幂等结果在切主后的恢复。分别设计 etcd/Redis 后端条件下的协议；届时启用预留 `REL-15` |
| `TODO-02` | DFS 高性能 SDK | 侵入式文件 API、身份/授权、buffer 与完成语义；与 FUSE 使用同一文件及副本合同；不扩展为 OwnerFs SDK |
| `TODO-03` | VerifiedCache 与 Seed 扩散 | 完整 Chunk 校验、种子发现、缓存淘汰、容量控制、与持久副本的角色边界；扩展大规模镜像/快照多源读取，不能把缓存计为 durable replica |
| `TODO-04` | 外部 Spill | 写穿/迁出、外部副本验证与 Meta 提交、召回、容量和本地删除条件；明确外部存储能否替代本地持久副本 |

### 10.2 后续需评估的扩展

| ID | 候选方向 | 评审重点 |
| --- | --- | --- |
| `TODO-05` | Redis 后端高可用 | 主从切换的已确认数据保证、后端角色变化与 Meta leader 协议的关系；不能把 Redis 自动切主直接等同于 AFS 端到端高可用 |
| `TODO-06` | 升级、格式迁移与后端迁移 | 跨版本兼容、回滚、CopyLocation 等存量格式、etcd/Redis 迁移时的持久化与幂等记录；第一阶段仅保证同版本重部署 |
| `TODO-07` | 物理集群与规模验证 | 独立物理故障域、真实 RDMA NIC、硬件掉电和大规模并发；重新制定性能/容灾指标，不能从单宿主 RXE 成绩外推 |

systemd/Kubernetes 集成、OwnerFs 转 DFS Snapshot 和专用容器/MicroVM 适配器不因本表存在而成为后续必做承诺，需要独立需求确认。
