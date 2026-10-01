# OwnerFs native bind：架构 → 性能 → 生产集成验证计划

> 执行方式：在现有隔离分支内使用 superpowers:executing-plans。沿用 RFC、代码和原始实验；issue → 本地修改 → PR，不合入。此计划调整执行优先级，不宣告特性或阶段验收通过。

**Goal:** 先证明适用合同可实现，再实测同条件 ext4/native bind/MooseFS，最后补齐生产功能与可靠性。

**Architecture:** 保留 OwnerFs 创建 workspace、Home ext4 唯一 backing、原路径 native export 和远端 FUSE/P2P。现有管理器、授权、缓存及锁基础作为实验输入；不以继续修补单个实现缺陷代替架构决策。

**Tech stack:** 现有 Rust/fuser、Linux 6.8.0-142 Hyper-V VM、ext4、认证 P2P；WSL 仅构建。MooseFS 固定基线由现有 acceptance/baselines 提供。

**Spec:** [RFC 0001](../docs/rfcs/0001-ownerfs-native-bind-mount.md)、[用户合同](../docs/architecture/ownerfs-native-access.md)、[发布验收](../docs/acceptance.md)、issue [#42](https://github.com/lelezi257/dms/issues/42)。本计划优先于[原实现计划](native-bind-plan.md)中任务的执行顺序，不覆盖历史结果。

## 全局约束与当前检查点

- 用户于 2026-10-01 明确要求按本计划三个阶段执行；不得自行缩小需求或用测试数量表示进展。
- 已验证源码基线 `caec0fa`，主线基线 `78245771167643d5883491052e7cebcaba8c3be2`；隔离分支 `feat/ownerfs-native-bind`。
- 前两阶段只改阻碍验证或使测量失真的问题；每次修复必须标注它阻碍哪一个实验。其他缺陷进入有限后续清单。
- 同路径不意味着两个 VFS inode 相同；已进入 FUSE 的请求不会因 bind 自动转换成 native。旧文件 fd 不得按名称改绑。
- 不用 lazy detach 表示排空，不删除身份不明或其他任务的数据，不更改另一台机器的主线工作。
- 不改写历史失败为 PASS，不借用旧 Node 包或其他分支的网络结果作为当前候选验收。
- 不擅自升级内核、改成 OFD 锁或排除 mmap/watch；缺少平台/语义决策时记录并对齐。

### 证据索引

完整历史与输入身份见 [native-bind-evidence.md](native-bind-evidence.md) 和 RFC 附录。下列 run 是保存的原始结果，不是本计划重新执行后的全功能声明。

| 代号 | 原始证据与范围 |
| --- | --- |
| E1 | RFC 基线 `6ee3f177a43ee85cc6b79666502330095d445fdb`：真实 mkdir/同路径 bind、过渡与两个 Node 的 P2P、并发和反例；两个 Node 同一 VM，不是跨 VM 性能或当前候选资格化。 |
| E2 | `foundation-20261001T084141-1a9652f7`：17 个选定 VM 基础案例；mount、cache、Home 授权 teardown、ready 后子进程及本地 kernel flock。结果明示不覆盖实际 Node/network P2P、native POSIX/mmap、完整生命周期、性能。 |
| E3 | E2 二进制 SHA256 `8802ceaa703b5a089aca33306789608ec1af13c2a1160ae8088a9ce247842ea3`，原始归档 SHA256 `92f0736857b733c16049134f99bfad9d2861cf48baf9331bc31753f92506d7d5`。构建时 parent `0d66794` + 已保存 dirty patch，之后提交为 caec0fa；不能仅看 parent SHA 宣称源码相同。 |
| E4 | `owner-lock-20260930T203148-aeeed453` 与 `acceptance/probes/ownerfs_native_lock_owner.py`：native 同进程 POSIX 降锁成功；继承同一 fd 的子进程代理填写原 PID 或使用 OFD 均 EAGAIN。结论仅为简单代理设计被否定。 |
| E5 | `directory-20261001T032414-5552aea2` 等旧目录强合同诊断：native 移动后旧 FUSE getcwd/父关系不即时跟随。原断言/失败保留；用户后续明确接受这一边界。 |
| E6 | `directory-20261001T040518-8ed09be5`：普通缓存策略重开仍旧值反例，native-eligible 从构建时零 TTL/direct I/O 的正向结果；E2 另验当前路径重开、长度、替换旧 fd、稳定根新查找。 |
| E7 | E2 retained-reference matrix：独立 native cwd、dirfd、MAP_SHARED/PRIVATE VMA-only 子进程导致正常 umount EBUSY，精确停止该子进程后成功。它不是远端映射一致性或完整 Node fencing。 |
| E8 | `operation-lifetime-20261001T083318-1affca08` 及 caec0fa 源码：xattr 全调用 RootUse、目录 opening grant/readdir 校验。只能证明相应 admission 边界，不证明实际文件引用排空。 |
| E9 | `phase1-inventory-20261001T092403`：当前 A/B/ctl 只读清单，逐机保存 command、exit、日志 SHA；A VM headers 确认 bit36 已存在，资源/卷与进程状态如下。不是 mmap 或性能 PASS。 |
| E10 | `path-probe-20261001T093405-bf9a8b23`：当前生产 FUSE adapter、ready 后子进程、实际 strace请求。16轮绝对路径有192次根元数据请求，native cwd/dirfd为0；旧FUSE对照432。只关闭A1访问路径计数问题，不是性能/Node/P2P资格化。首次观察器失败与纠正后复验均保留。 |
| E11 | `transition-probe-20261001T094349-512d3e25`：真实FUSE、当前Home permit和kernel backend组合；临时仅去掉实验线程挂载能力导致真实EPERM，状态FuseOnly/无native claim；延迟、失败、恢复挂载、正常卸载及旧FUSE引用均验证同backing。不是生产Node READY/P2P或暂停中的回调验证。 |
| E12 | `network-probe-20261001T101117-f3ebb95a`：ctl/A/B三台实际VM、当前Meta/Node bootstrap和TLS/P2P；仅测试构建选择native-eligible，并用现有Home permit/manager挂载。原路径native、B关闭重开与旧FUSE/P2P文件对象均通过，正常detach/全部进程退出。ordinary拒绝对照与收证失败保留。不是生产配置/READY、锁/mmap/回收或性能资格化。 |
| E13 | `network-probe-20261001T102555-4c76fce1`：同一实际三VM链路、新构建且与当前Linux测试driver源码匹配的Node；补齐已打开warmed reader关闭重开、三路径unlink后fresh ENOENT、同名重建以及旧fd再次写入不污染新对象。原始/逐文件SHA、真实RPC/TCP与正常退出均核验。关闭A2；其余实验族不因此通过。 |
| E14 | `network-probe-20261001T112518-369f8d1e`：复用冻结的实际Node/Meta、三VM认证链路；native/另一个native/旧本地FUSE/远端FUSE的非阻塞flock双向互斥、共享锁、rename替换后新旧对象隔离及native final-close释放通过。只关闭A3非阻塞flock组合证据；POSIX owner、阻塞等待/远端close排空、其他并发仍未关闭。离线观察器误用OwnerFiles锁指标的失败与纠正均保留。 |

Windows 证据副本：`C:/workspace/code/dms/local/native-bind-vm/<run>/`；VM：`/mnt/afsdata/ownerfs-native-bind/<run>/`。源码输入/构建日志：`/home/lzc/workspace/dms/evidence/ownerfs-native-bind/20261001/`。证据文件必须按摘要校验后使用。

## 语义豁免授权审计 A0

审计依据为原聊天记录，而不是 RFC 中一句“已接受”。原助手消息 `msg_0fa3d67cc085e065016abdd57fa5a487d09ed20041532c1eda` 明列四条合同：重开校验、打开期间不保证实时/快照、旧文件对象身份、旧 FUSE cwd/dirfd 不即时追踪父关系；同时说锁、原子性、权限、回收仍需保证。

用户消息 `01a0f58c-7727-7571-92a3-f9c7f4517eba` 明确回答：“接受上面的约束，但要在我们的文档中明确指出约束。用实际的case指出来，方便用户了解。”并说明管理面创建 workspace 再拉起 Agent 的工作流。

| 合同或例外 | 审计结论 |
| --- | --- |
| close-to-open；已打开跨路径 reader 无实时刷新/快照承诺 | 已获明确同意；不能扩大到关闭重开仍旧值。 |
| 旧文件 fd 保留对象；旧 FUSE cwd/dirfd 的即时目录追踪例外 | 已获明确同意；后者不豁免同一 native 路径的普通目录语义。 |
| 普通 close 可见性与显式 fsync/fdatasync 持久性分开 | 原提议已说明，原生 ext4 对照也必须保持相同屏障；不得影响普通 FUSE-only/DFS 合同。 |
| 同进程 native 与旧 FUSE fd 的 POSIX 锁兼容性例外 | **没有找到同意。** 后续问题 `call_uS0k7SHbFuE5hAsJAP8bSmYg` 未得到回答；此次已重新提出架构冲突问题。当前仍是要求。 |
| mmap、watch/inotify 的一般豁免；依赖跨视图 inode 数字一致的应用 | **没有找到明确豁免。** RFC 记录观察/风险不是用户批准。分别说明能力、原理和所需设计，不以“首版不支持”自动关闭要求。 |
| 跨 workspace rename/link EXDEV | 既有 OwnerFs 已拒绝跨 Root，源码 `rename_across_roots_returns_exdev_io_error` 与两个操作分支可核对；需与 native 控制样本确认它是现有边界，而非新增例外。 |
| 管理面负责创建/删除/回收/切换 bind | 用户已明确选择此生命周期方向；不由此推导全局无管理进程也能即时撤销 native fd。根目录普通 rmdir/rename 与管理入口的区别必须向用户展示。 |
| 定制内核、内核模块或升级环境 | 未获平台选择；Linux 6.9 passthrough 的存在不是 POSIX owner 或一致性解法的证据。 |

## 阶段一：功能完备性的架构验证

分类可以在同一项并列：基础机制可行并不等于当前组合实现完备。

| 关键问题 | 当前分类及证据 | 尚缺的决定性证据 |
| --- | --- | --- |
| 同路径新访问与最终 Agent namespace | 已验证可行：E1/E2原路径落到同一ext4 source，ready后子进程inode/namespace匹配。E10确认绝对路径仍有FUSE祖先查询，相对路径操作为0，文件数据均走native。E12在实际Node/P2P链路中复验原路径/最终namespace与同一source。实现遗漏：生产 `src/node.rs` 仍构建ordinary OwnerFs，native worker未生产接入；实验入口只在Linux测试构建选native-eligible。 | 生产native READY发布有可行的身份核验原语，但仍需受管启动/拒绝失败的组合证明；祖先查询成本须在阶段二实测，若研究缓存替代必须保持权限/epoch/生命周期，不能只测相对路径。 |
| mkdir 与挂载过渡 | 已验证可行：E1 后回复挂载、E2 post-reply事件、E11实际延迟/EPERM失败/恢复/卸载及旧引用均使用同一backing；失败管理状态无native claim。架构事实：已分派FUSE请求不会自动改走ext4。 | 补暂停中的FUSE回调与实际Node/P2P组合；验证失败不能导致生产READY。E11管理原语状态不是实际Node ACK。 |
| 旧 cwd/dirfd/文件句柄 | 旧目录即时等价为架构限制/已接受边界：E5。文件对象保留与重开可行：E2/E6；E12/E13补齐当前认证跨VM组合，同长度覆盖/缩短/空文件、双向写回、rename替换、unlink/同名重建与已打开reader关闭重开；旧fd始终保留旧对象。 | A2所需对象/重开机制证据已齐。当前A/B从构造起采用native-eligible；ordinary/mixed客户端的缓存兼容与模式协商不能由此推断，归入A4。保留旧目录反例作边界展示，不扩大已有豁免。 |
| 本地/远端并发及锁 | 原基线 append/EXCL/独立区域并发可行：E1；本地 kernel flock 可行：E2；E14补齐当前认证跨VM非阻塞flock双向仲裁/对象替换。实现遗漏：native POSIX 仍软件模型。**当前简单 Home 代理不满足同 PID 锁合同：E4。** | 用户决定保留全部锁合同则必须证明能代表原 POSIX owner 的替代架构；不能用不同 PID 普通互斥通过替代同 PID 降锁/解锁/任意 fd close。阻塞等待/取消和远端final-close排空、当前候选append/EXCL/并发写仍需组合证据。 |
| mmap、缓存、元数据/watch | native-eligible客户端close-to-open普通I/O可行：E6/E13。原远端MAP_SHARED ENODEV、watch无native事件：E1；当前部分chmod/symlink/xattr已实现，历史ENOSYS不是当前事实。 | 当前ABI/capabilities下的MAP_SHARED/PRIVATE、msync/fsync/重新打开和映射脏数据证明；实际P2P权限/链接/xattr；ordinary/mixed客户端缓存及模式协商；watch需求与事件桥可行性。零TTL/direct I/O不自动解决映射，也不证明未选择该策略的客户端。 |
| 删除/回收/切换 | busy 与 source/target/epoch 防混淆可行：E2/E7。实现遗漏：完整受管 Agent/peer 排空未接入。架构限制：RootGrant 不能即时撤销已打开 native fd；挂载根 ordinary rmdir/rename 可先 EBUSY 而不进入 FUSE。 | 受管 Actor + 远端在途写；先拒绝 admission，再排空/停止，正常卸载，最后切换新 epoch/Home；延迟旧请求不得写新实例。必须证明实际引用消失，不用 RootUse=0 替代。 |
| 异常后重建与长期演进 | 现有 journal/helper crash 在**同一个存活 namespace**内可行：E2。尚未验证：Node/FUSE death、Agent namespace 消失、boot/session 更换与 Home 迁移的组合。 | 受控 daemon death/namespace-loss 的最小机制证明；旧 native fd 仍可写时不 ready、不复用；重观察权威和挂载身份，不把 journal 当当前内核事实。完整 retry/durable ACK 工程留阶段三。 |

### 有限实验清单及判定

每个实验族先复用已存在的证据关闭已知结论，仅执行缺失组合；每个新反例最多做一次受控复现用于排除实验错误。之后进入设计决策或后续清单，不无限追加相似测试。实现若改变关键假设才重做受影响项。

- [x] **A0 授权/证据审计**：固定基线、找出原用户同意、隔离第三阶段 WIP；锁范围选择尚待回答，不算合同全部关闭。
- [ ] **A1 同路径/过渡**：受控 mkdir→延迟 bind→ready→启动 Actor；失败回退；最终 namespace 身份与绝对/相对路径请求计数。关闭条件：同一 backing、过渡操作正确、不死锁/错误 ready；路径成本归因明确。
  - 访问路径计数已由E10关闭：真实绝对路径16轮有192次祖先元数据查询，cwd/dirfd相对操作为0；导航启动成本单列。A1整体保持未完成。
  - E11关闭实际挂载失败/延迟/恢复与旧引用的同backing组合；控制状态为FuseOnly而非ready。尚未暂停真实回调跨挂载，也未资格化实际Node/P2P READY，A1整体仍未完成。
- [x] **A2 对象/重新打开**：当前候选真实 A native、保留本地 FUSE、B 远端；内容/EOF、同长度修改、替换/unlink、稳定根查找。关闭条件：满足已同意 close-to-open/对象身份；旧目录边界按原始反例说明。
  - E12时关闭实际Node/认证跨VM链路缺口，并验证同长度覆盖、缩短、空文件、B写回、native替换与旧对象不改绑；当时仍缺unlink后fresh ENOENT/同名重建、已打开reader关闭后重开的组合，未关闭A2。
  - 后续E13补齐上述最后组合，A2关闭；限于从构造起native-eligible的实际A/B链路，普通/mixed策略的接入与兼容属于A4，未默认为通过。
- [ ] **A3 并发/锁**：复用 E4 拒绝简单代理；实际跨路径 append/EXCL、flock、POSIX owner/close/fork/dup 与 native/native 控制。关闭条件：所需锁机制有正向证明，或用户明确修改范围；不得默认豁免失败组合。
  - E14关闭当前认证跨VM非阻塞flock的共享/排他及对象替换问题；不重复扩大这个案例族。阻塞等待/取消、远端final-close以及其他并发仍保留，POSIX owner要求未获得豁免。
- [ ] **A4 映射/兼容性**：固定当前ABI与内核；MAP_SHARED/PRIVATE、msync/fsync、native写后fresh remote reader、P2P mode/symlink/xattr、ordinary/mixed客户端缓存及模式协商，以及watch/inode边界核对。关闭条件：适用要求有可行路线和关键机制证据；未获批准的例外不能关闭。
- [ ] **A5 受管回收/切换**：复用 busy matrix；增加真实 managed native Actor、remote fd/在途请求、同名新 epoch 与切换 Home 的最小控制。关闭条件：旧写入在新实例就绪前被 fenced；失败保持 draining，不删除/复用 backing。
- [ ] **A6 崩溃/namespace 边界**：已有同 namespace helper 恢复不重做；受控 Node/FUSE death、最终 Agent namespace loss 各一组。关闭条件：可正确辨认和拒绝未知旧状态、恢复路径可实现；完整自动恢复留阶段三。

**阶段一结束条件：** 所有适用要求均有明确合同、原理解释和关键机制证据；无未获授权的豁免，无会否定方案的未决架构冲突。可以遗留已经证明有实现路线的工程遗漏，必须进入阶段三清单。若某必需语义只能靠不被批准的平台/模型改变实现，阶段一不能 PASS，先与用户对齐；不开始性能调优来绕过这个问题。

### 当前 mmap 平台事实

Linux 6.8 已有 `FUSE_DIRECT_IO_ALLOW_MMAP`（flags2 bit36），当前 VM headers 也存在；[内核 mmap 处理](https://github.com/torvalds/linux/blob/v6.8/fs/fuse/file.c)会在 direct I/O 且未协商该能力时对 shared 映射返回 ENODEV。项目当前 fuser 默认 ABI7-33，vendor 未定义该能力；这首先是 ABI/协商和映射缓存设计的证据缺口，**不是已经证明必须升级内核，也不是启用一位即可证明一致性**。[内核 UAPI](https://github.com/torvalds/linux/blob/v6.8/include/uapi/linux/fuse.h)、[FUSE I/O 文档](https://docs.kernel.org/filesystems/fuse/fuse-io.html)。

## 阶段二：真实性能穿刺

### 最小运行链路 P0

```text
ctl VM：现有 Meta/etcd + 当前候选 Meta API/Root 权威
A/Home：当前候选 Node 的 native-eligible OwnerFs + TLS/P2P + ext4 backing
        FUSE mkdir → 现有 NativeMountManager 核验同路径 export → 再启动 Actor
B：当前候选 OwnerFs FUSE → 认证 P2P → A 同一个 Root/Home/backing
MooseFS：固定 master/chunkserver（A、goal=1）、A/B 客户端、相同 ext4 卷/网络配额
ext4：A 同卷独立实验目录，使用相同 Actor/数据/屏障
```

最低限度只加入实验专用启动/构造选择或驱动，沿用既有生产 RPC 和真实进程；不得用 in-process Meta fixture 或绕过 P2P 的文件函数当远端性能。生产 Node 的完整配置/worker/Agent-ready/cursor/ACK 不作为提前全面开发的理由。实验驱动也不能冒充已经发布的产品 native READY。

E12已跑通该链路中的当前Meta/Node/TLS/P2P与原路径export：Meta使用独立local-file存储，Node是debug lib-test产物，仅实验构造选择不同。它解决阶段一真实访问链路，不满足P0正式性能条件；阶段二还需冻结优化构建、同条件资源/数据/屏障，并跑通固定MooseFS及其B001合同。

只读盘点（2026-10-01）：A/B 均 Linux6.8.0-142、2 vCPU、约5925MiB RAM；A `/mnt/afsdata` 为 `/dev/sdb1` ext4，32GiB/约29GiB可用；ctl 有 env-rebuild etcd，A/B 未观察到运行中的 AFS/MooseFS 服务。`mfsmount/mfsmaster/mfschunkserver/fio` 未在 A 的 PATH 找到，B 未找到 mfsmount/fio；不能据此声称系统所有路径都不存在。原环境包/配置/TLS 仍在，不重建 VM。

MooseFS 复用 commit `ac106b2ec8661ff00def725d042cb67d3ca2184d` 的现有构建方案。已有基线 README 的 **B001 strong-durability 未匹配**仍成立：stock fsync 的物理持久性没有合格证据。先实测其应用 ACK/同步配置合同；不能 stock build 成功便判基线合格。可以公开可见性/读 lane 的诊断数据，但不得据此宣布 durable-write gate PASS。

P0 完成前不计正式性能：核对版本/SHA、PID+start tick、Root/epoch/Home、namespace/mount/卷、网络、资源、数据 seed、协议、副本和有效屏障；每方读写内容成功。MooseFS B→A 与 DMS B→A 固定同一 Home 位置。原生 ext4 没有同等独立远端接口：A ext4 是本地基线及 Home 上限，B→A 比较 DMS 与 MooseFS；不把额外 NFS/SSH 路径冒称“原生远端 ext4”。

### 冻结工作负载与门槛

沿用 acceptance PERF-01～04/07，新增同 workload 的 ext4/native 成对对照；不扩展本特性到 DFS/3FS/RDMA 全发布测试。

| 实验族 | 固定维度 |
| --- | --- |
| P1 元数据/小文件 | 10,000 个4KiB文件；create/stat/readdir/rename/unlink 分项及完整任务，分别并发1/8；绝对路径、ready 后 cwd/dirfd 相对路径各报结果。 |
| P2 顺序读写 | 总8GiB，1MiB请求，并发1/8；冷首次与预热后重复读取分开；写分别 close-only/fdatasync/fsync。并发不增加任务总字节。 |
| P3 随机读写 | 固定 seed，4KiB/64KiB；读的8GiB工作集与512MiB可驻留热 lane 分开；随机覆盖固定512MiB；并发1/8，写三种屏障。 |
| P4 远端及差距归因 | B→A 执行相同 P1～P3，用 DMS/MooseFS 两方；报告相对 A ext4 上限的差距、网络/CPU/Home I/O/FUSE 请求及尾延迟，不声称纯网络访问无需成本。 |

每一确定的 case 使用1次预热和5个有效配对轮次，交替运行顺序，公开全样本、配对中位数及不确定性；操作延迟的 p50/p95/p99 使用操作样本，不拿5个任务值伪装可靠的尾延迟。明显宿主争用/缓存前提失败导致环境无效，最多2次补跑，仍不足则 INCONCLUSIVE；不能一直跑到偶然通过。

- **达到 native ext4：** 对本地 native 的每个冻结工作负载、路径形式和相同终点，配对耗时比 `median(T_native/T_ext4) ≤ 1.0`；吞吐/CPU/尾延迟同时报告。无默认5%/10%宽限；波动足以改变结论则 INCONCLUSIVE，稳定差距则未达标。这个操作性门槛严格保留用户“达到 native”的目标，不自行允许退化。
- **MooseFS：** 既有 OwnerFs 本地读/写分别 `T_DMS/T_MFS ≤ 0.5`，远端分别 `≤ 0.8`；元数据固定报告，没有偷加一个已接受的 MooseFS 比例目标。
- **公平性：** 同硬件/CPU内存限制/ext4卷/虚拟磁盘层/网络/数据/缓存/并发/副本/成功终点；数据集按实验逐项重用，避免32GiB卷容纳三套8GiB外加MFS临时空间时失真。预留空间不足即不测。
- 冷读仅在没有其他业务的专用 VM 清 guest 缓存、重建 mount，并记录 DMS/MFS 自身缓存；Hyper-V/Windows宿主缓存不能完全控制的边界如实报告。8GiB大于guest RAM，重复读不能称全量热读；热 lane 用512MiB并记录驻留/底层读取。
- 任一内容/EOF/errno错误即该项功能失败；屏障无法匹配则对应项 BLOCKED，不用系数折算，不省略失败项。零 Home FUSE 数据请求只证明路径，不能替代性能数据；FUSE祖先元数据查询也应计入。

**阶段二结束条件：** 有完整可比的三方本地/两方远端原始数据、各门槛结论和差距解释，native 与既有 MooseFS 适用目标成立。存在稳定退化、未匹配 durability 或关键项 INCONCLUSIVE 时不能宣布通过；先有限归因并作设计/环境决策，不进入无界微优化或阶段三全量实现。

## 阶段三：暂缓工作清单与结束条件

前两阶段不以这些事项为主线；只有它们明确阻碍 A/P 实验时，才摘取最小范围修复。

1. 生产 native 配置/默认关闭、Node worker 与事件消费、完整 Agent readiness/start/namespace 协同。
2. 实际文件/目录/锁/映射引用排空，dirty sync 错误保留、重复 RELEASE、peer expiry 和可靠 retry。
3. durable command cursor/ACK/replay、全故障点恢复、boot/session/namespace 变更协调和 Home 切换工程。
4. 泛化权限/路径/对象审计、其他不阻碍实验的回归修补和重构；不扩展到普通 OwnerFs/DFS 无关模块。
5. 全量生产 POSIX/故障/部署回归、包装发布、最终 review 和不合入 PR。先前基础回归结果保持原范围。

已隔离 WIP：stash `1694e88261f70154beec1d3980928cc0249471f6`，完整副本 `native-stage3-deferred-20261001T090918`。包含句柄 drain/reaper/sync retry 候选及一个**仍失败**的 pinned-descriptor duplicate peer RELEASE 用例；不称为已验证提交，不在阶段一继续它。计划起始时Cargo缓存lib executable来自这份WIP；之后已被E12实验构建替换。必须按各run的源码快照/实际SHA区分，不能按同一个缓存文件名判定源码身份。

**阶段三结束条件：** 补齐接受合同的生产入口/实际排空/失败恢复，完成适用 Linux 回归和最终审查，更新状态及案例；push 隔离分支，创建并附加链接 issue #42 的 PR 供另一 AI review，不 merge/auto-merge。当前 goal 直到交付实际达到才 complete。

## 下一步与进度口径

优先关闭 A3 的 POSIX owner 架构冲突，同时完成 A1/A2/A4～A6 中不依赖该选择的缺失证据。每次更新仅报告：哪个关键问题关闭、依据哪个 run/设计决策、阶段是否满足退出条件。一次基础案例重跑或新增测试不等于阶段前进；默认不反复重跑已有 E2 全集。
