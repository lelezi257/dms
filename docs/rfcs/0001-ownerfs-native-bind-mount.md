# RFC 0001：OwnerFs workspace 同路径原生 bind mount

- 状态：待决策的方案；未实现产品功能，未改变已接受的架构或发布验收要求。
- 日期：2026-09-30。
- 穿刺基线：`6ee3f177a43ee85cc6b79666502330095d445fdb`。
- 关联：[OwnerFs 架构](../architecture/ownerfs.md)、[实现状态](../status.md)、[发布验收](../acceptance.md)。

## Problem

用户在 `/ownerfs` 下创建 workspace，例如 `mkdir /ownerfs/agent1`。`agent1` 就是 workspace，不再在其下面增加一个 workspace 层级。希望创建仍由 FUSE 接住，而之后本地文件访问使用 Home 上的普通 ext4 文件；远端仍通过自己的 FUSE 挂载，经 P2P 请求 Home。用户继续使用 `/ownerfs/agent1`，不需要改目录名、改工作目录或复制数据。

本方案决策所需的挂载时序、过渡读写、真实 OwnerFs/P2P、并发、缓存、常用文件操作和生命周期穿刺已完成。结果支持**同路径原生访问这一机制**，也证明**当前 OwnerFs 不能仅增加 bind mount 就获得完整的本地/远端语义等价**。缓存、锁、元数据和生命周期存在可复现的缺口；失败结果也是穿刺结论，不能计为功能通过。

推荐采用“创建后异步挂载、过渡请求访问同一 backing directory、从创建起使用共享缓存策略、由管理入口卸载”的方案。首版适合 Home 固定、Agent 生命周期可管理的工作区。若要求任意程序始终使用普通 `rmdir/rename` 管理 workspace 根目录，或者依靠每次 RootGrant 检查立即撤销原生句柄，则本方案无法满足，应保留 FUSE。

## Contract

### 1. 用户可见的步骤

1. 用户执行 `mkdir /ownerfs/agent1`，请求进入 OwnerFs 的 FUSE `mkdir`。
2. Home 创建普通本地目录，确定 `RootId`、root epoch、Home/session 和 backing directory。这个目录从一开始就是所有路径的唯一数据来源。
3. FUSE 返回 `mkdir` 成功。随后管理器把 **backing directory 挂到 `/ownerfs/agent1` 原路径上**。
4. 挂载前已经进入 FUSE 的请求，继续对同一 backing directory 正常执行。不能把 FUSE 目录作为另一份存储，之后再复制到 ext4。
5. 挂载后，从当前挂载命名空间重新解析 `/ownerfs/agent1/...` 的新访问进入 ext4。挂载前的文件句柄、目录句柄和 `cwd` 可以继续留在 FUSE，直到关闭或退出；不承诺这些旧引用自动变为原生引用。
6. 远端 `/ownerfs/agent1/...` 仍由远端 FUSE 经 P2P 请求 Home，Home 操作同一 backing directory。
7. 删除、回收、切换 workspace 时，由管理器先处理访问和 bind mount，再改变 workspace 的权威状态和存储。具体顺序见生命周期章节。

概念上的挂载方向如下：

```text
Home 普通文件目录：/data/ownerfs/<root-id>-e<epoch>
                                 │
                                 └─ bind 到 /ownerfs/agent1

本地新路径访问 ───────────────────────→ ext4
本地旧 FUSE 句柄 ─→ OwnerFs ───────────→ 同一 ext4 文件/目录
远端 FUSE ───────→ P2P → Home OwnerFs → 同一 ext4 文件/目录
```

不是把 FUSE 的 `/ownerfs/agent1` bind 到其他目录；那仍然访问 FUSE。路径保持不变，但 `st_dev/st_ino` 等身份、挂载边界和已有 watch/句柄并不完全透明。

### 2. `mkdir` 与挂载的时序

**不在未返回的 `mkdir` 回调里等待同路径 bind 成功。** 在本次 Linux 6.8 穿刺中，同线程执行 `mount --bind`、异步 helper 但等待 helper、直接 `mount(2)` syscall，都出现了等待 `mkdir` 完成的阻塞。取消或结束未完成的 FUSE 操作后才解除。直接 syscall 排除了仅由 `mount` 命令前置 `stat` 引发的问题，但不把这一结果泛化为所有内核和挂载 API 的不可行性证明。

采用异步挂载就必须允许过渡请求；单纯缩短挂载延迟不构成正确性保证。已经进入 FUSE 的 `CREATE/READ/WRITE`，在挂载之后回复也仍是 FUSE 操作。可以继续访问相同 ext4 backing file，不需要强行转换这次请求为原生操作。

可选的入口优化是暂缓 workspace 的 **LOOKUP**，直到挂载完成后再回复。玩具穿刺在 entry/attr TTL 为零、识别并放行 mount helper 请求的条件下，验证了 `mkdir` 后立即 `stat` 和立即 `open/create` 的新路径访问能进入原生目录。但已有 dentry、旧 dirfd/cwd、其他路径解析和 helper 身份处理都需要产品级验证，所以不作为正确性的前提。

不要统一暂停所有后续 FUSE 回调：helper 自己也可能需要访问目标路径。`GETATTR` 门控没有保证首次访问为原生；`CREATE` 门控在穿刺中等待至超时后，helper 才完成挂载。首版推荐正常执行过渡请求，不依赖门控。

### 3. 缓存：必须先解决再暴露原生路径

同一 backing file 是数据一致性的必要条件，但不能消除额外的 FUSE 页缓存。本次真实 OwnerFs 已复现：

- 在从未远端共享的 workspace 上，旧 FUSE fd 先读到 `BEFORE`；bind 后原生写入 `AFTERN`；旧 fd 仍读到 `BEFORE`。
- workspace 已发生远端访问后，简单的三路径覆盖读写可以相互看到；但保留更早打开的本地缓存 fd，在 rename/unlink 后原生继续写入，旧本地 FUSE fd 仍读旧值，远端 fd 能读到新值。

当前实现的 `PrivateFuseCache` 在首次 peer admission 时切换共享策略并失效缓存；这不能被视为旧缓存句柄持续与任意原生写入保持一致的证明。不能仅在 bind 前调用一次失效，然后保留旧缓存 fd。

建议为 **native-eligible workspace** 从创建开始启用共享访问策略：

- 本地和远端 FUSE 的普通文件 `open/create` 使用 direct I/O；entry/attr TTL 为零；不保留另一份数据页缓存。
- 过渡写和远端写直接操作 Home 的 ext4 backing file；原生路径使用 ext4 自身的页缓存。
- root 对应的 FUSE inode、目标 dentry 和 epoch 在 bind 生命周期内保持稳定；不因普通属性刷新重建 workspace 身份。
- 当前已有缓存句柄的 workspace 不直接热切换：先停止/排空使用者，处理映射与脏页，再在可控的挂载生命周期中转换。不能仅统计 fd 就宣称已排空所有 `mmap` 引用。

无缓存 passthrough 玩具穿刺通过了旧 FUSE fd 与新原生 fd 的双向读写、rename/unlink 后保留 fd 等用例。它验证了设计方向，没有替代真实 OwnerFs 的实现验收。真实 OwnerFs 的上述缓存失败必须在后续实现中修复并重新验证。

### 4. 文件语义与兼容性范围

| 范围 | 方案要求及边界 |
| --- | --- |
| 普通读写、追加、创建 | 同一底层对象；过渡 FUSE 请求和远端请求不能丢失或落入第二份目录。穿刺的独立区域并发写、并发追加和 `O_EXCL` 竞争通过；不据此声称所有重叠写入都具备事务语义。 |
| rename/unlink/替换 | 新路径访问对应当前对象；已打开 fd 保留原对象，不能把旧 fd 按路径重新打开。实际原子替换用例中，旧远端 fd 读 `OLD`，新 open 读 `NEW`。缓存 fd 的失败另见上节。 |
| inode 身份 | FUSE 与 ext4 的可见 `st_dev/st_ino` 不相同；按 root/epoch 和底层文件身份校验 P2P 句柄。不能把路径字符串当作完整身份，不能承诺依赖 inode 数字的程序完全无感。 |
| workspace 内 rename/link | 使用 ext4 原生语义；远端必须具备对应接口和身份检查。 |
| 跨 workspace rename/link | 独立 bind mount 之间，本次即使位于同一 ext4 设备也返回 `EXDEV`。明确维持 workspace 边界，不自动模拟跨根原子 rename。 |
| 文件锁 | 当前远端 `flock` 和 `fcntl`/`lockf` 锁能与本地原生独占锁同时获得，未形成互斥；两个原生进程对照返回 `EAGAIN`。需要 Home 内核同一 inode 上的统一锁机制与远端锁 owner、连接失效、句柄关闭的生命周期；单独增加一张用户态锁表不能拦截原生锁。POSIX 进程锁与 OFD/文件描述锁的映射也必须设计和验证。 |
| mmap | 原生共享映射写入后，普通远端读看到了更新；当前远端共享 `mmap` 返回 `ENODEV`。首版不能声称支持跨路径共享映射一致性，尤其不能保留旧 FUSE 缓存映射后直接 bind。 |
| chmod、符号链接、xattr | 原生路径可执行；当前远端 chmod/readlink/跟随符号链接返回 `ENOSYS`，远端 xattr 返回 `EOPNOTSUPP`。这是真实兼容性差异，不能以“性能不同、语义一样”概括。远端可读取原生创建的普通硬链接内容。 |
| inotify | 穿刺中原生 watch 收到创建/写入事件，远端 watch 建立成功但未收到该次原生变更事件。不承诺远端 watch 等价；如目标应用依赖它，必须补事件协议和重新验收。 |
| close 与持久化 | 当前 OwnerFs 普通 close 和原生普通 close 均不能等同于持久化确认。显式远端 `fsync` 在 Home 跟踪到了成功的 `fsync(2)`；本次两路径目录 `fsync` 返回成功。需要持久发布时仍按文件/目录同步合同执行，未进行掉电恢复验证。 |

对于锁、mmap、符号链接或元数据语义有要求的应用，不能静默启用本优化后宣称兼容。仅保留 FUSE 也不会自动补齐当前缺失的远端功能；这些属于后续实现和验收的前置条件。

### 5. 挂载管理器与状态机

建议引入独立的、权限收敛的 MountManager。OwnerFs 负责 workspace 权威状态与普通文件/P2P 操作；MountManager 负责原生出口的创建、校验和回收。只接受受信管理面按 `RootId + epoch + Home/session + namespace` 发出的请求，不能接受任意用户提供的源路径/目标路径去执行特权 mount。

每个 workspace 的管理记录至少包含：

```text
RootId、root epoch、Home Node/session
backing directory 身份和被固定的目录引用
目标路径、目标 FUSE inode 身份
mount namespace 身份、mount ID、挂载源身份、挂载标志
desired state、observed state、操作序号、最后一次错误
```

记录的 desired state 可持久化用于恢复；observed state、mount ID 和 namespace 状态必须重新观察，不能把上次运行的数值直接认作当前有效挂载。需要按 root/epoch 串行处理 mount、取消、删除和切换，避免迟到的 helper 挂载到同名新 workspace。

| 状态 | 行为 |
| --- | --- |
| `FUSE_READY` | backing 和权威 root 已准备；共享缓存策略已启用；允许普通 FUSE/P2P 请求。 |
| `MOUNTING` | 独立 helper 在正确 namespace 内执行 bind；FUSE 请求仍正常落到 backing。 |
| `NATIVE_ACTIVE` | 以 mountinfo、mount ID、源/目标身份确认成功；新路径访问走原生，旧 FUSE 引用仍有效。 |
| `FUSE_ONLY` | mount 失败或被禁用；保留完整 FUSE 数据访问；记录原因并按受控策略重试。 |
| `QUIESCING` | 停止新的生命周期变化与新 Agent admission；排空/停止现有使用者和远端请求，准备卸载。 |
| `DRAINING` | 存在仍未结束的旧 fd/cwd/mmap 或不能正常卸载的引用；不得标记为已回收或已切换。 |
| `DETACHED` | 管理的原生出口已解除且回收前提满足；后续按操作目的退回 FUSE、删除或切换。 |
| `RECOVERING` | 启动时对账实际挂载、root/epoch、namespace 和残留 endpoint；完成前不开放新的原生出口。 |

挂载幂等性必须由记录和实际身份保证：重复 `mount --bind` 会形成叠加挂载，本次叠两层后一次 umount 仍然是原生目录。不能把重复 mount 当作无副作用重试。

源目录和目标目录需要使用受控目录引用、严格的路径解析与身份校验，避免符号链接替换和同名 root 重建的竞态。具体 FD mount API 尚未穿刺，不能把它当作已验证的 `mkdir` 死锁解法。实现时必须针对选定 API 重新验证时序和 TOCTOU。

### 6. 卸载、删除、回收、切换

**workspace 根目录的普通删除/改名不能成为可靠的卸载触发点。** Linux 在 bind mount 根上返回 `EBUSY`，在机制穿刺中 FUSE `rmdir/rename` 回调计数均为零。`rm -rf` 还可能先清掉目录内的数据，最后删除根目录失败，不能靠这个流程回收 workspace。

建议由显式 workspace 管理入口统一触发创建后的挂载和删除/回收/切换前的卸载。普通子目录和文件操作仍使用文件系统接口。workspace 根目录保持 `/ownerfs/agent1` 不变，但根生命周期操作的管理合同需要单独接受。

**仅关闭原生优化、继续使用同一 workspace：** 暂停新的管理操作，正常卸载 bind；成功后回到 FUSE，共享策略继续保持。普通卸载忙时返回可诊断的 busy 状态；若允许 lazy detach，应显示仍有旧原生引用，不能宣称性能路径或权限已经统一回到 FUSE。

**删除/回收：** 标记 `QUIESCING` → 停止新的 Agent/peer admission → 排空或停止实际使用者 → 关闭远端 Home 句柄并完成相关权威 fencing → 解除全部受管 namespace 的 bind → 在无旧写入者的前提下删除权威记录及 backing。目录非空、引用未退出或权限不足时保留失败状态，不能只卸载目录入口就删除/复用存储。

**同名重建或 Home/epoch 切换：** 先完成旧实例 quiesce、fencing 和卸载；新实例使用新的 epoch 目录。重新挂到相同 `/ownerfs/agent1` 只影响新路径解析，不会重定向旧 fd/dirfd/cwd。穿刺中，新路径读 `NEWROOT`，旧 native dirfd 的 `openat` 仍读旧目录的 `AFTERN`。

本次正常 umount 在打开的原生句柄存在时失败；lazy umount 后旧 native fd 仍可写，远端旧 fd 看到了该写入。这说明 `MNT_DETACH` 是路径脱离，不能用作撤销访问或同步屏障。若无法停止/排空旧使用者，保持 `DRAINING`，禁止切换到会被旧写入污染的新实例。

### 7. 权限、命名空间与失败边界

- bind 后的原生访问由 Linux UID/GID、mode、ACL、挂载标志和进程隔离约束，不再逐次经过 OwnerFs RootGrant/peer 检查。RootGrant 失效不能凭空撤销已发给进程的原生 fd。
- 当前 mount 未启用 `allow_other/default_permissions`。本次降 UID 的直接路径访问被外层 FUSE 拒绝；但显式继承原生 dirfd 后可按 ext4 DAC 读取可读文件。该用例说明目录引用也是访问能力，不等同于证明任意其他 UID 可直接穿过 `/ownerfs`。
- 挂载管理器应显式制定 `nosuid/nodev/noexec/ro` 等部署策略，不能假设继承了外层 FUSE 挂载的全部限制。禁止向非受管进程暴露可绕过隔离的 backing 路径/句柄。
- 首版采用同一受管 Agent namespace；若运行器为 Agent 单独创建 namespace，必须在其 namespace 内挂载或建立明确的传播关系。本次 private namespace 内成功挂载，VM 初始 namespace 对该 workspace 路径得到 `ENOENT`。
- Home 固定时，该方式可用于可信、可停止的本地 Agent。要求强授权撤销、允许不受控进程保留 fd、或需要旧 Home 立即失去写能力的场景，应保持受权威检查约束的路径，或另行实现可验证的进程/存储 fencing。
- Node/FUSE daemon 退出不保证关闭原生出口。真实用例中旧原生 fd 在 `SIGKILL` 后仍可写；某些先前稳定的子挂载路径仍可访问，而经历切换的路径随后返回 `ENOTCONN`。只保证“daemon 退出不等于原生引用已撤销”，不承诺所有新路径在崩溃后都能访问。
- 恢复应由独立管理进程/运行器负责：读取实际 mountinfo，核对源、root epoch、namespace 和存活进程；处理受管残留 bind、死 FUSE 挂载与 Unix socket；再启动 Node 并按有效状态重建出口。本次直接重启先被残留 socket 拒绝，清理测试挂载和确认已死亡进程的 socket 后恢复了持久 workspace 文件。
- 对账只能操作本管理器拥有且身份匹配的对象；身份不符时进入错误状态，禁止对任意 mount 或正在使用的 socket 执行清理。

## Alternatives

| 方案 | 判断 |
| --- | --- |
| 全程 OwnerFs FUSE | 保留当前访问和生命周期入口；本地仍有 FUSE 成本。对不能接受新增生命周期合同的应用，保留此路径。 |
| `mkdir` 返回前直接 bind | 本次命令和直接 syscall 穿刺均阻塞；不采用。 |
| `mkdir` 返回后异步 bind，过渡请求正常执行 | 推荐；正确性依赖同一 backing、共享缓存策略和受管生命周期，不能只依赖挂载速度。 |
| 在 workspace LOOKUP 处门控 | 已证明特定条件下可改善首次新路径进入原生的概率/时序；作为后续优化，不能代替 fallback。 |
| 暂停所有 CREATE/READ/WRITE 等回调等 mount | 不能把已进入的请求改为原生，也可能让 mount 等待目标操作；不采用作为切换协议。 |
| 给用户一个不同的原生目录/符号链接 | 改变路径或链接语义，不满足同路径要求。 |
| FUSE passthrough | 可单独评估，绕过部分文件 I/O 路径但仍由 FUSE 管理命名空间；不是本次已验证的方案。需要核实目标内核/用户态支持并补安全与生命周期穿刺。 |

## Validation

### 1. 环境、证据和结果解释

WSL Ubuntu 24.04 构建；Hyper-V `dms-smoke` VM 验证：Ubuntu 24.04、x86_64、Linux `6.8.0-142-generic`、ext4。OwnerFs 使用真实 `afs-meta`、两个独立 `afs-node` 进程、不同 data/mount/端口、独立证书与 mTLS Node 身份、真实 gRPC/P2P，Meta 使用 local-file 后端。

两个 Node 在同一 VM、同一 mount namespace 中运行，协议与文件语义是实测的；不是独立物理节点、网络故障域或性能测试。全部挂载实验在 `unshare --mount --propagation private` 中运行，实验结束回收受管挂载与进程。

| 证据代号 | 本地留存内容 |
| --- | --- |
| M | `artifacts/runs/20260930-ownerfs-bind-probe/run-02/`：七种直接挂载时序、结果、FUSE 日志、阻塞时内核栈。 |
| G | 同目录 `run-03/`、`run-04/`：LOOKUP/GETATTR/CREATE 门控，分别用先 stat 与立即 open 的客户端。 |
| F | 同目录 `run-05-direct-io/`、`run-06-cached/`：共享 backing passthrough 与缓存负对照。 |
| R | 同目录 `real-ownerfs-results.json`，对应真实 OwnerFs `run-07` 的 26 个场景；完整 runner、结果及日志在 `ownerfs-real-evidence-final.tar.gz`。 |
| P | 同目录 `native-path-results.json`、`native-path-evidence.tar.gz`：真实 OwnerFs 原生访问的 FUSE syscall 对照；包含 runner 和原始 strace。 |

上述路径均相对于研究工作区 `/home/lzc/workspace/dms/`，不是 Git 仓库路径。原始过程产物与临时 probe 程序留在研究工作区，本 RFC 保存可阅读的结论、复现方法与验收合同；不发布证书私钥。R 的 `outcome=observed` 表示完成观察，不表示功能成功；错误 errno、缓存旧值和不互斥锁都是已观察的负结果。

### 2. 已执行的决策穿刺

| 穿刺 | 实测结果 | 设计结论 | 证据 |
| --- | --- | --- | --- |
| 同路径覆盖 ext4 | 新 fd 的 device 与 backing ext4 一致；路径名不变 | 基础原生出口可行 | M、R |
| 原生路径是否仍发 FUSE 请求 | 每组 40 次操作：绝对路径 open/read/write/close、保留 native fd 的 read/write、native dirfd 的 openat/read/close，均未跟踪到 Home `/dev/fuse` 请求；旧 direct-I/O FUSE fd 对照产生 40 READ + 40 WRITE | 这些访问确实绕过了 FUSE 请求路径；未量化耗时收益 | P |
| `mkdir` 未回复前 mount | inline/helper/direct syscall 阻塞，完成或取消目标请求后解除 | 不等待 mount 再回复 mkdir | M |
| mkdir 后异步 mount | 立即访问可先落到 FUSE；最终 bind 成功 | 必须支持过渡请求 | M |
| LOOKUP 门控 | 先 stat、立即 open 两种客户端进入原生 | 仅为条件成立时的优化 | G |
| GETATTR / CREATE 门控 | GETATTR 不保证首次原生；CREATE 等待超时后 helper 才完成 | 不作为统一入口切换协议 | G |
| 过渡创建和旧 fd | direct-I/O probe 双向读写，rename/unlink 后 fd 继续访问同一对象 | 同 backing fallback 可行 | F |
| 缓存负对照 | 缓存 probe 旧读未更新；真实 OwnerFs 旧 fd 返回 `BEFORE` | 缓存必须在 native 暴露前处理 | F、R |
| 真实远端 P2P | 远端读到 Home 数据，原生与远端写入互见 | 基本 P2P 数据路径可保留 | R |
| 两进程普通操作 | 200 次独立区域写及 200 组 create/rename/unlink，两路径最终字节符合预期 | 已覆盖有限并发用例 | R |
| 并发 append / EXCL | 200 条完整唯一记录；独占创建一方成功，另一方 `EEXIST` | 已覆盖相应基本原子性 | R |
| 原生 rename/unlink + 旧 fd | 远端旧 fd 保留对象；早期本地缓存 fd 出现旧值 | 保留对象语义可行，旧缓存未解决 | R |
| 原子替换 | 旧远端 fd 为 `OLD`，新 open 为 `NEW` | 身份不能只按路径重绑 | R |
| mode / chmod | 远端 stat 看见原生 mode；远端 chmod 为 `ENOSYS` | 元数据操作不等价 | R |
| 符号链接 / 硬链接 / xattr | 原生能创建；远端 symlink 访问 `ENOSYS`，xattr `EOPNOTSUPP`，普通硬链接内容可读 | 需要明确兼容性门槛 | R |
| 文件锁互斥 | 远端 flock/lockf 在原生独占锁期间仍成功；原生对照 `EAGAIN` | 当前锁不能跨路径协调 | R |
| mmap | 原生映射变更被远端普通读看到；远端 mmap `ENODEV` | 不承诺远端共享映射 | R |
| inotify | native 有事件，remote 此次无事件 | watch 不能承诺等价 | R |
| 文件/目录同步 | close 未触发 Home fsync；显式远端 fsync 跟踪到成功 syscall；两路径目录 fsync 成功 | close 不等于持久化屏障 | R |
| workspace 根 rmdir/rename | `EBUSY`；机制 probe 回调计数零 | 卸载必须有管理入口 | M、R |
| 跨 workspace rename/link | native→FUSE、两个独立 native bind 的相应操作为 `EXDEV` | 维持跨根边界 | R |
| bind 失败 | 不存在的源导致 mount 失败，随后 FUSE 创建/读取正常 | 可降级 FUSE_ONLY | R |
| 重复 bind | 两层叠加；卸载一次仍原生 | 需要真实幂等性 | R |
| 正常/lazy umount | 正常 busy；lazy 后旧 fd 可写，远端可见 | detach 不是 revoke | M、R |
| namespace | private namespace 可见，VM 初始 namespace 对实验 workspace 返回 `ENOENT` | helper 必须选对 namespace | R |
| 降 UID 与继承原生 dirfd | 直接路径被拒绝；继承原生 dirfd 可按 DAC 读文件 | 原生引用是能力边界 | R |
| 切换目录 | 新路径 `NEWROOT`；旧 native dirfd 仍 `AFTERN` | 切换不重定向旧引用 | R |
| daemon kill / restart | 旧 native fd 可写；新路径可受死 FUSE 影响；残留 socket 阻止直接重启，受控清理后恢复持久文件 | 外部管理与恢复对账必需 | R |

玩具缓存负对照没有失效协议且使用简化属性，它只证明“不处理缓存会读旧值”。真实 OwnerFs 又独立复现了旧值，因此方案不能将问题仅归因于玩具程序。玩具 FUSE 对不支持的创建可能故意返回 `EIO` 用于识别路径；此错误不是 DMS 的文件创建行为。

### 3. 复现与审计

OwnerFs 构建命令在 WSL/Linux 执行：

```sh
cd /home/lzc/workspace/dms/repos/dms
cargo build --locked -p afs --no-default-features --features ownerfs --bins
```

本次真实 runner 位于研究工作区 `infra/vm/ownerfs-bind-probe/real_ownerfs_probe.py`。它为每次运行创建独立目录和证书，启动 Meta/A/B、发现唯一 backing 文件、以 syscall/errno 和数据内容记录结果，最后清理自己的挂载与进程。VM 已部署目录为 `/home/lzc/ownerfs-real-probe/`；在已配置的独立测试 VM 内，可以使用新的 run 名复现：

```sh
sudo unshare --mount --propagation private \
  python3 /home/lzc/ownerfs-real-probe/probe.py run-new
```

脚本使用固定测试端口，需要它们处于空闲状态；只能在专用测试 VM 使用，不能在正在服务的部署上执行相同清理流程。本次脚本与原始日志不在源代码仓库，GitHub 阅读者可按以下步骤重建同样的核心实验：

1. 启动 mTLS Meta 和两个 OwnerFs Node，分别配置不同的 data_dir、uds_path、mount、监听端口、证书，并配置 `trusted_node_certs` 绑定确切 Node 身份。
2. 在 A mount 创建 workspace 和文件；保留一个 FUSE fd；B mount 读取，确认远端访问。
3. 从 A 的 local root catalog 确认 backing；在 A namespace 内 `mount --bind "$backing" "$mount_a/agent1"`，比较新 fd、旧 fd 与 backing 的 device。
4. 用 native/remote/old-FUSE fd 交叉覆盖读写；再测试原子替换、unlink 后保留 fd、追加和 EXCL。
5. 单独创建从未远端访问的 workspace，先缓存 FUSE 读取，再 bind 后原生覆盖，验证旧 fd 是否返回旧值。
6. 原生持有非阻塞独占 flock/lockf，让另一进程经 B 请求同类型锁；必须设置原生进程竞争作为负对照。
7. 测试原生 mmap 后的远端普通读和远端 mmap；测试 mode、链接、xattr、watch 与目录 fsync。
8. 测试 mounted-root rmdir/rename、重复 mount、busy umount、lazy 后旧 fd、namespace 可见性和切换后的旧 dirfd。
9. 在专用进程上跟踪 Home 的 fsync/fdatasync，比较普通 close 与显式远端 fsync；最后 kill Node，检查旧 fd、残留挂载、socket 和受控恢复。
10. 单独跟踪 Home 的 `/dev/fuse` `read(2)`，解析 FUSE request header 的 opcode；比较原生绝对路径、native fd、native dirfd 与挂载前保留的 direct-I/O FUSE fd。必须用旧 FUSE fd 的 READ/WRITE 作为正对照，不能把跟踪器没有抓到请求误判为绕过 FUSE。

审计 SHA-256：

```text
afs-node   ab68969f40915ab1dcfabde95d73285e2dd51dab08134c678623de69ae3c55c0
afs-meta   4da61bd68e2912e4ea58f784d2e24598b6d070d3ea9b5dec56e3fea7ad524654
runner     28b823f9ca996c42f87ed1b7e25832508ffd042e4b9af1d5d6f75c0f284e6379
evidence   56b8ec4fc4bb4cffd4a27c57711e9dbf95874bf3804181b2ce133cc07c2d2772
```

上述 runner/evidence 哈希对应最终 `run-07` 归档；归档未包含证书私钥或运行中的 socket。

P 使用同一真实集群配置，四组各执行 40 次操作；strace 成功附着 Node 的全部线程，以 `/dev/fuse` 路径过滤接收 syscall。前三组没有捕获请求，旧 direct-I/O FUSE fd 正对照捕获 opcode 15/16 各 40 次。证据支持已测试稳态访问绕过 FUSE，不代表任意路径解析、冷缓存、namespace 或故障状态均无 FUSE 请求，也不是性能基准。

P 归档 SHA-256：`02dfeec6bb302f5d223087feb0e1e3698a55865dee89fd81ca7fc47816342ac7`。

### 4. 后续实施顺序与准入条件

这些是产品实现后的验收要求，不是本次穿刺已经实现的功能：

1. **先接受适用合同。** 固定 Home、受管 Agent 生命周期；同路径新访问原生，旧引用可保持旧路径；workspace 根由管理入口删除/回收/切换；确定目标应用对锁、mmap、元数据、watch 的需求。
2. **先建立一致的 fallback。** native-eligible root 从创建开始使用共享策略；为已复现的两个真实缓存失败编写回归用例；覆盖即时访问与 mount 失败，禁止两个存储副本。
3. **再实现 MountManager。** 按 root/epoch 和 namespace 串行、幂等地管理出口；确认 mount ID 和源身份；验证目标 dentry 生命周期、路径替换竞态、迟到 helper、重复请求和崩溃中间状态。
4. **实现受管回收与 fencing。** 普通 busy 和 lazy draining 明确可见；停止/排空使用者后才能删除、复用名字或切换 Home/epoch；用保留 fd/dirfd/cwd/mmap 的测试证明旧引用不能污染新实例。
5. **满足应用兼容性。** 文件锁必须真的与原生内核锁互斥；若应用依赖 symlink、chmod、xattr 或 watch，过渡 FUSE 和远端路径都必须补齐，否则同一操作会因挂载时序而改变结果。需要跨路径 mmap 时另行设计，不能仅取消 direct I/O。
6. **实现后才量化收益与发布验证。** 在独立 Linux 节点和规定平台上覆盖负载、故障、权限、恢复与性能。比较 ext4 baseline、OwnerFs FUSE、native bind、remote FUSE/P2P 的完整工作负载，不能从 device 相同推出固定性能提升百分比。

本次没有执行掉电恢复、ARM64、跨 VM 网络故障、ACL 全矩阵、完整 POSIX/应用套件或性能基准。这些属于后续实现验证与现有发布门槛，未因本 RFC 降低。当前结论足以决定采用哪一种机制以及需要哪些合同变化，不构成全功能或发布验收通过。

### 5. 参考

- 当前实现：[FUSE 适配](../../src/node/fuse.rs)、[OwnerFs 与缓存](../../src/node/vfs/ownerfs.rs)、[root 权威与 epoch](../../src/node/vfs/ownerfs/root.rs)、[root catalog](../../src/node/vfs/ownerfs/catalog.rs)、[配置与身份](../../src/config.rs)。
- Linux：[mount(2)](https://man7.org/linux/man-pages/man2/mount.2.html)、[umount(2)](https://man7.org/linux/man-pages/man2/umount.2.html)、[mount_namespaces(7)](https://man7.org/linux/man-pages/man7/mount_namespaces.7.html)、[close(2)](https://man7.org/linux/man-pages/man2/close.2.html)。
- 内核路径实现参考：[Linux v6.8 namei.c](https://github.com/torvalds/linux/blob/v6.8/fs/namei.c)、[Linux v6.8 FUSE dir.c](https://github.com/torvalds/linux/blob/v6.8/fs/fuse/dir.c)。
- 缓存与替代方向：[FUSE I/O modes](https://docs.kernel.org/6.7/filesystems/fuse-io.html)、[FUSE passthrough 官方文档](https://docs.kernel.org/filesystems/fuse/fuse-passthrough.html)。
