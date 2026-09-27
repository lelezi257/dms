# OwnerFs 缓存目录项后的命名空间复验（2026-09-27）

**结论（事实）：** 在隔离 Linux A/B/C VM 上，最终 release 二进制通过 OwnerFs 三节点验收 **15/15**；本地 W1 两份独立六轮 p50 分别是 MooseFS 的 **0.358/0.349 倍**，继续满足 ≤0.80 门槛。代表性远端 W2 六轮总量为 MooseFS 的 **1.049 倍**，基本持平，但没有证明每个远端阶段持平或稳定领先。只改 OwnerFs，未启动 BlobFs/S6，未 push、merge 或 release。

## 为什么改

**事实：** 旧版 B 缓存了远端正目录项 1 秒。B 持有旧目录 FD 时，A 把目录改名并在原名新建一个目录；B 随后通过旧 FD 创建文件，实际写进了**新目录**。原始复现见[修复前结果](../../../../experiments/results/2026-09-26-afs-ownerfs-w2/optimization/stale-parent-before-fix.json)。它是错误目标写入，比单纯返回 `ESTALE` 严重。

**改动：** B 对子目录操作发送父目录的 `FileIdentity`；A 在命名空间操作的同一锁内校验真实父目录身份，身份不符拒绝。目录改名时更新已缓存后代路径；覆盖 rename 即使源未缓存也清理旧目标映射。首次远端访问的本地 FUSE 页缓存失效不再持有缓存互斥锁发送通知，同时用独立锁保证第二个远端请求等失效完成。这些是现有 OwnerFs/FUSE/Peer 边界内的修正，没有新增核心模块或改变 VFS/Meta/Storage trait。

**接口偏离，需重点评审：** `node_data.proto` 的 OwnerFiles Lookup/Create/Mkdir/Unlink/Rmdir/Rename 请求增加预期父目录身份字段，`RemoteFiles` 方法签名同步增加参数。原因是只传路径时 Home 无法区分“旧目录移走”与“原名新目录”，无法阻止错写。两端需同版升级；旧进程正在使用的目录 FD 在远端改名后可能得到 `ESTALE`，调用者应重新打开。当前不承诺远端旧目录 FD 在改名后继续访问移走的原目录。

## 最终证据

| 验证 | 结果 | 原始数据 |
| --- | --- | --- |
| W1，预建私有根、四后端同场、两份各六轮 | 0.358 / 0.349 × MooseFS；PASS | [W1 result](../../../../experiments/results/2026-09-27-afs-ownerfs-cache/w1-final/result.json) |
| W2，200 个 4 KiB 文件、交替顺序、六轮 | 1.049 × MooseFS；PASS（W2 无数值门槛） | [W2 result](../../../../experiments/results/2026-09-27-afs-ownerfs-cache/w2-final/result.json) |
| 三节点功能与恢复 | 15/15，0 失败 | [acceptance](../../../../experiments/results/2026-09-27-afs-ownerfs-cache/acceptance-final.json) |
| A/B 不同长度交替写、旧目录 FD + 改名重建 | 各 20/20；旧 FD 均 `ESTALE`，0 次写进新目录 | [namespace probe](../../../../experiments/results/2026-09-27-afs-ownerfs-cache/cache-namespace-final-review.json) |
| 同名立即重建后 B 重开 | 20/20，0 失败 | [reopen probe](../../../../experiments/results/2026-09-27-afs-ownerfs-cache/namespace-reopen-final-review.json) |

W2 分段 p50（OwnerFs/MooseFS，ms）：B 初读 **105.1/81.7**，B 重读 **61.0/38.0**，B 覆盖写 **154.8/143.7**，A 本机回读 **20.3/61.7**。总量近持平仍是本机优势抵消远端读劣势；剩余性能差距集中在 B 的读路径。与 v13 的 1.024 倍相比，本轮 1.049 倍包含不同时间的环境波动与新校验成本，不能只凭一次会话做归因。

W1 两个会话的 p50（ms）分别为 OwnerFs **240.4/228.7**、薄 FUSE **211.7/205.6**、Native FS **120.2/118.5**、MooseFS **670.8/655.6**。OwnerFs 已接近薄 FUSE 的量级，仍约为 Native FS 的两倍；Native 只作本机参考，不是 W1 数值门槛。

Linux `cargo fmt --all --check`、workspace 全特性测试（排除 vendored fuser 自身测试目标）、严格 Clippy 全目标全特性和 release bins 构建通过；新目标映射回归单测通过。真实 FUSE 与 P2P 已由上述 Linux 三 VM 用例覆盖。未验证完整通用 POSIX、VM 掉电、长稳、RDMA 文件内容路径，也未证明高并发目录改名与操作的所有时序。
