# OwnerFs 阶段验收（2026-09-26）

## 结论与范围

**事实：OwnerFs 的本地优势场景与旧三节点功能目标通过；远端 W2 正确但性能退化，不能宣称恢复旧分支的远端性能。** 本轮只覆盖 `feat/agent-workloads-foundation` 的 OwnerFs/gRPC 数据路径，不覆盖 BlobFs、RDMA OwnerFiles、NFS、长稳或完整 POSIX。测试均在 Linux；没有 push、merge 或 release。

| 验收项 | 当前结果 | 证据 |
| --- | --- | --- |
| A/B/C 三 VM 功能 | 15/15 步通过，0 清理错误 | `experiments/results/2026-09-26-afs-ownerfs-acceptance/result.json`（工作区根目录下） |
| W1 本地小文件 | 两份独立会话各 6 轮，OwnerFs/MooseFS p50 = **0.6973 / 0.6795**；合同门槛每份 ≤0.80，通过 | `experiments/results/2026-09-26-afs-ownerfs-w1/result-v8-isolated.json` |
| W2 跨节点小文件 | 6 轮内容正确；总耗时 p50 OwnerFs **1803 ms**、MooseFS **324 ms**，比值 **5.56**；无 W2 数值门槛，但明显退化 | `experiments/results/2026-09-26-afs-ownerfs-w2/result-v8.json` |

W1 在同场还测得 p50：OwnerFs 451.9/439.9 ms、MooseFS 648.0/647.5 ms、薄 FUSE 196.5/197.2 ms、Native FS 122.4/118.7 ms。OwnerFs 已比 MooseFS 快约 30–32%，但离薄 FUSE 尚有约 2.2–2.3 倍差距。`result-v8-contended.json` 是与编译测试重叠的无效性能场，第二份比值 1.00；隔离负载后完整重跑并以 `result-v8-isolated.json` 为验收结果。所有 W1 数字均来自同一 v8 二进制和预建 workspace，不包含首次建根。

## 已跑通的功能链

**事实：** 三 VM 验收实际执行了首次根创建与 Meta REST 位置查询、A 写 B 重开、A 覆盖 B 重开、跨节点 rename/unlink/rmdir、并发建根唯一 Home、远端旧 FD 在 rename/unlink 与同名重建后仍指旧文件、不同长度交替写、Meta 进程重启、Home 进程重启后旧连接有界报错及重新打开恢复、跨根 rename 返回 `EXDEV`。Home 重启后 B 的旧授权按新代重新取得；运行中 FD 不承诺无感续接。

**事实：** 本地根由 Meta 持久预留/激活，Home 使用普通文件；B 从 Meta 取得位置与根授权，通过 mTLS P2P 到 A 的同一文件执行操作。B 加入不撤销 A 授权。Meta 重启期间 Node 在租约窗口内有界重试心跳。

## W2 退化定位

**事实：** 六轮 p50 分段（OwnerFs/MooseFS）：B 初读 558/79 ms，B 重读 547/38 ms，B 覆盖写 654/144 ms，A 回读 40/62 ms。每阶段为 200 个 4 KiB 文件，单线程顺序执行。B 单文件初读约 2.75 ms、重读约 2.72 ms、覆盖写约 3.25 ms；A 本地回读约 0.20 ms。

**推断（代码路径支持，尚无逐 RPC trace 计数）：** B 的一次 `open/read/close` 被 FUSE 拆为 lookup/open/read/release 等多个串行 P2P gRPC 命令；RootGrant 与 P2P Channel 已按根复用，Home 对已验证授权有缓存，因此“每次文件都重新建连/找 Meta”不是主要原因。B 的 FUSE 内容缓存为正确性保守关闭，MooseFS 在重读阶段可使用客户端缓存。主导成本是逐回调跨 VM 往返，尤其重读仍重复付费。后续若优化 W2，须先取得每文件 RPC/延迟 trace，再评估安全的缓存失效或复合操作；不能直接打开 B page cache 后把旧读当作收益。

## 接口与布局偏差复审

没有新增 VFS/OwnerFs/Storage/传输的核心层。相对最初骨架，已增加以下必要合同增量：`Vfs::with_ownerfs` 注入已注册的生产实例；Meta RootRecord 增 Pending/Active 与本机准备身份；Node 注册可发现的 P2P endpoint；mTLS 将 peer 身份绑定到授权；Storage 补普通文件属性/句柄操作；FUSE 的 `state.rs` 分离进程内 inode/FD 生命周期；OwnerFiles 扩展文件命令并在 `node/rpc/peer.rs` 实现客户端。它们分别用于防止空骨架挂载、支持崩溃恢复、真实跨节点路由、拒绝伪造身份、承载基本 POSIX、保留旧 FD、贯通 P2P。Meta 恢复后旧 B grant 通过记录 revision 原子替换，跨根错误稳定映射到 `EXDEV`。

## 未完成边界

- **待验证/风险：** `/ownerfs` 根目录列举目前只显示本 Node 已知的 workspace；另一 Node 新建的根在按名称 lookup 前可能不出现在 `readdir`。这不影响已测按路径访问，但尚不满足完整统一目录列举。
- **待验证/范围外：** symlink/hardlink、全部 chmod/chown/时间属性与 `RENAME_EXCHANGE`、根删除同名重建、VM 永久丢盘、掉电级持久性、完整通用 POSIX 和长稳未作为本轮通过项。
- **事实：** 本地根目录准备记录与文件会做同步，尚未对 `.ownerfs-roots` 父目录独立 `fsync` 的突然断电语义给出证据；本轮证明的是进程与 Meta 重启恢复。
- **事实：** OwnerFiles 当前内容通道使用 gRPC；共同 RDMA adapter 的诊断合同不等于 OwnerFiles RDMA 完成。

当前结果支持进入同事功能试用，但 W2 性能退化和以上边界必须随版本说明一起提供；不能称为完整 MooseFS 替代品。
