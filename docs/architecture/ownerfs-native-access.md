# OwnerFs native workspace：使用流程与一致性边界

本页定义已接受的 **OwnerFs native 模式目标合同**，不是当前版本已经实现或通过验收的声明。功能可用性见[实现状态](../status.md)；挂载设计见 [RFC 0001](../rfcs/0001-ownerfs-native-bind-mount.md)。普通 OwnerFs FUSE-only 与 DFS 保留原有合同。

## 推荐流程：先准备 workspace，再启动 Agent

典型使用者不需要自行执行 mount，也不需要更改 `/ownerfs/agent1` 路径：

1. 管理面创建 workspace，确定 RootId、epoch、Home/session 和唯一 backing directory。
2. 管理器将 Home 上的 ext4 backing 挂到 `/ownerfs/agent1` 原路径。
3. 在 **Agent 最终使用的 mount namespace** 中核验实际挂载、源身份、Root/epoch/Home 和挂载策略，确认 native ready。
4. 管理面再启动 Agent，向它暴露就绪的挂载；Agent 在此后解析路径、设置 cwd、打开文件和目录。

`mkdir` 成功、目录存在或等待固定时间，都不等于 native ready。管理进程中的挂载也不必然在 Agent namespace 中可见。不能把 bind 前打开的 FUSE dirfd/cwd 作为启动 Agent 的入口；如果传递目录 fd，应传递已验证的 native 引用。

挂载失败时保留数据并报告状态。管理面可以明确选择按普通 FUSE-only 合同启动，不能报告 native ready。对已运行的 workspace 不透明热切换；切换模式、删除、回收或 Home/epoch 切换先排空/停止受管使用者，再正常卸载并完成 fencing。

这个顺序避免本地 Agent 在创建与 bind 的过渡窗口获得旧 FUSE 引用。远端 Agent 仍使用 FUSE/P2P，因此仍需了解下述目录边界。

### 容器 Agent：只暴露已就绪的 workspace

这是核心使用场景：管理面仍通过 OwnerFs 根创建 `/ownerfs/agent1`，完成 native 准备后，容器运行时仅把这个 workspace 挂入 Agent 容器的 `/ownerfs/agent1`。容器内的父目录属于容器自身；不把宿主整个 OwnerFs 根、其他 workspace、backing 父目录或宿主 root/proc 暴露给 Agent。Agent 在 workspace 内可以正常读写，通过 `..` 或 symlink 不能访问宿主的父级数据。

管理面必须核验容器最终取得的 source inode、Root/epoch/Home、mount namespace 与挂载策略，不能仅把一个路径字符串传给 daemon 就认为 native 已就绪。Node 的私有 namespace 中已经挂好，不代表宿主 daemon 看得到；运行时误挂 FUSE、空目录或旧对象都不能进入 native ready。实验性 OCI 验证和当前性能能力见[实现状态](../status.md)，不是已完成 Docker/Podman 管理集成的承诺。

容器内父路径可使用其自身的 native rootfs，因此绝对访问这个 workspace 可以避开宿主 OwnerFs FUSE 祖先查询；它必须单独测量，不能用宿主路径或相对路径的成绩替代。目录隔离检查针对宿主数据的读取/枚举；Linux mountinfo 的 mount-root 字符串仍可能暴露源路径元数据，不能把数据隔离宣称为完全隐藏元数据或任意容器配置的安全保证。

删除、回收或切换时，管理面还必须处理最终容器 namespace 中的 bind 和 Agent 引用；只检查原 export 的 mountinfo 不足以证明容器已停止访问。不得在未知容器引用仍存活时删除或复用 backing。

## 文件合同：close-to-open

native 与 FUSE/P2P 访问同一 backing；它们是不同访问路径，不因位于同一个 mount namespace 就共享全部内核缓存或目录状态。

- writer 的先前写入和 close 均成功后，reader 从当前有效路径 **重新 open**，必须看见这些写入或更晚的已完成修改；没有并发覆盖时内容与长度精确一致。包括旧缓存被关闭后的再次打开。
- 已打开的跨路径 reader 不承诺实时观察其他端的变化，也不承诺 open 到 close 期间是不可变快照。
- 已打开的文件 fd 绑定原对象。rename、unlink 或同名替换不能让旧 fd 悄悄指向新对象。
- 此合同不提供并发写入的事务隔离。append/EXCL、锁互斥、权限、支持的 mmap/同步语义和资源回收仍按各自合同验证。
- native 普通 close 确认可见性，不能替代持久化屏障；需要重启/掉电恢复保证时检查文件 `fdatasync/fsync`，目录项还需父目录 `fsync`。普通 FUSE-only/DFS 的既有 close 完成策略不因此改变。

## 目录合同：重新解析当前路径

从稳定的 workspace 根重新查找/打开目录时，应按当前目录树解析。长期持有的 **旧 FUSE cwd/dirfd** 在另一访问路径移动或删除目录后，不承诺 `getcwd`、`..` 和 fd 路径立即追踪 native 目录树。

重新 open 一个文件并不一定刷新旧目录引用：`openat(old_dirfd, "../file", ...)` 可能先沿旧父关系选中了目录。需要当前目录树时，从 `/ownerfs/agent1` 重新解析当前路径，再取得新目录引用；不能仅在旧 cwd 下重新 open 相对路径。

这个边界适用于 bind 前获得的本地 FUSE 引用，也适用于长期保留的远端 FUSE 引用。bind 后取得的本地 native cwd/dirfd 使用 ext4 自身的目录语义；同一 native 路径中的普通 rename/unlink 不被豁免。

## 实际使用案例

### 当前未满足的要求：跨路径追加与文件偏移

这是[E15真实VM诊断](../../development/native-bind-evidence.md#e15-single-syscall-append-conflict)发现的缺口，**不是已批准的语义豁免**。即使管理面先挂好workspace再启动本地Agent，普通远端写入者仍可能遇到：

1. 文件先有4字节，远端以`O_APPEND`打开它。
2. 本地native再追加4字节。
3. 远端随后追加4字节，返回写入成功。
4. 最终内容正确、长度12，但远端查询当前文件偏移得到8，应为自身追加终点12。

另一个案例中，远端一次2MiB追加被FUSE拆成多个请求，本地native的小追加插入了这一次写入中；native/native对照的同一次写入保持连续。它不依赖本地继承bind前的FUSE引用，也不属于已打开reader的刷新边界。完整追加、偏移和锁合同仍是要求；当前版本不能声称全功能等价，路线变更或额外约束需要明确对齐并验证。

### Case 1：常规 Agent，创建就绪后启动

管理面准备 `/ownerfs/agent1` 并确认挂载身份，随后启动 Agent；Agent 才执行 `chdir("/ownerfs/agent1")` 和 open。

**合同：** 本地 Agent 取得 native 引用，工作区内操作进入 ext4。路径不变，Agent 不需要识别切换过程。验收必须证明就绪时序、Agent namespace 中的挂载身份以及操作确实绕过 Home FUSE。

### Case 2：本地写完，远端随后读取

本地 Agent 将 `result.txt` 写为 `NEW` 并成功 close；应用通知远端 Agent，远端随后重新 open `result.txt`。

**必须：** 远端读到 `NEW` 和对应长度。远端若在本地写入前已打开 reader，则不保证那个旧 reader 立即读到 `NEW`；关闭并从当前路径重新打开后，不能继续返回旧缓存。同步先后用应用通知建立，不能靠 sleep。

### Case 3：已打开文件遭到同名替换

reader 已打开 `result.txt` 的对象 A；writer 发布对象 B 并用 rename 替换该名字。

**必须：** reader 的旧 fd 仍对应 A，新 open 对应 B。不得为了“刷新”而按同名路径重开旧 fd，把它偷偷改绑到 B。A 的内容是否被另一个 writer 修改，另按已打开 reader 的一致性边界处理。

### Case 4：旧 FUSE cwd 遇到 native 目录移动

某进程在 bind 前进入 `/ownerfs/agent1/left/moving`；bind 后，本地 Agent 将该目录移动到 `right/moving`。旧进程直接调用 getcwd。

**允许的边界：** 旧 FUSE cwd 仍可能显示 `left/moving`；它的 `..` 也不保证立即指向 right。不能据此判断当前目录树或选择当前父目录。需要访问移动后的目录，应从 workspace 根解析 `/ownerfs/agent1/right/moving` 并取得新引用。

**推荐流程为何能避开：** Case 1 的本地 Agent 到 native ready 后才取得 cwd，因此不持有这个旧 FUSE cwd。长期使用远端 FUSE cwd 的 Agent 仍应遵守此边界。

### Case 5：旧 FUSE 引用遇到目录删除

旧 FUSE dirfd/cwd 指向某目录，另一端将它移动后删除。ext4 的旧目录 fd 可能仍能访问保留的父关系并返回 nlink0；旧 FUSE 引用可能返回 ENOENT，或暂时保留旧父关系/路径表示。

**不承诺：** 两条路径的旧目录引用此时完全等价。不能用旧 getcwd 或 `..` 判断该名字是否仍存在；从 workspace 根重新查找已删除名字应得到 ENOENT。此例不允许把旧文件 fd 改绑，也不允许跳过忙引用、fencing 和卸载检查来回收 backing。

### Case 6：删除整个 workspace 或切换 Home

Agent 仍持有 native fd、cwd 或 mmap；管理面请求回收 `/ownerfs/agent1`。

**必须：** 停止新 admission，排空/停止受管 Agent 并处理远端引用，完成 fencing 和正常卸载，再删除或切换。busy 时保留可诊断的 draining 状态。不能把 lazy detach 当作旧写入者已经消失，也不能用普通 `rm -rf` 代替 workspace 管理入口。

### Case 7：Home 授权失效，挂载仍有忙引用

管理器已把 workspace 挂好，进程还持有 native 文件 fd；此时 Home
RootGrant 被撤销。管理器必须拒绝新 native admission，并通过受管进程的
停止/排空完成 fencing。旧 native fd 的权限不会因 RootGrant 自动消失。

挂载生命周期仍需保留该 workspace 挂载点的正向 FUSE 身份和目录元数据，
直到正常卸载结束并释放最后一个管理凭证。这里保留的是挂载点元数据，
FUSE 文件读写、子项查找和 P2P 授权仍拒绝失效的 grant。不能通过对挂载点
返回负 LOOKUP 或使其 dentry 失效来模拟卸载：内核可能移除子挂载，但旧
native 引用仍然存在，无法据此认定已排空。

**必须：** 原挂载自身有忙引用时，正常卸载的 EBUSY 应保留物理挂载和管理
记录；引用关闭后正常卸载并核验完成。另一个 mount namespace 中的独立
容器 bind 是不同的挂载对象：E19 实测原 export 正常卸载成功后，容器仍能
读取、创建和写入同一个 ext4 source。因此必须登记并停止/卸载这些容器
挂载，核验最终 Agent 引用已消失，才能完成回收或切换；不能用原 export
的 umount 成功代替这一步。凭证释放后不再保留撤销根的元数据入口。
目录存在、RootGrant 失效、挂载点从 mountinfo 消失，都不是 backing
可删除或可复用的充分证据。实际 Node/Agent fencing 仍需独立验收。

### Case 8：应用依赖 shared mmap 或跨路径通知

本地容器 Agent 对工作区文件使用 shared mmap/msync，或远端 Agent 用 watch 等待本地 native 产物更新。

**必须核对：** native 文件访问和跨view close-to-open不自动证明旧FUSE/remote共享映射与事件投递可用。适用的映射写回、同步和watch需求应单独证明，不能作为旧cwd边界的附带豁免；管理面先挂载再启动Agent也不解决远端watch域的问题。native Agent 观察远端修改的事件同样需按实际链路验证。当前能力和正负向证据见[状态](../status.md)及[有界穿刺](../../development/native-bind-closeout.md)，本页不新增禁止这些API的永久使用限制。

## 验收与可用性

正向验收以管理面创建 → Agent namespace native ready → 启动 Agent 为主 lane，并覆盖 close-to-open、同名替换/旧文件 fd、从稳定根重新解析、挂载失败与受管回收。另保留提前持有 FUSE 引用的边界 lane，让用户能理解差异。

旧 FUSE cwd 的即时追踪与完全透明目录父关系不属于本 profile 的保证。历史强合同反例保留原始结果和断言，用于说明边界，不改称产品通过；产品正向用例仍需独立实现和验证。锁、权限、普通文件操作、支持的 mmap、显式同步、生命周期及性能验收不能因目录边界而跳过。
