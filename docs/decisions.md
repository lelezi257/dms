# AFS 架构决策

## 2026-09-28：DistributedFs 使用统一不可变 FileVersion/Chunk 数据模型

- **决策：** 通用分布式后端命名为 `DistributedFs`（`DFS`），与 1～4 节点小集群优化 `OwnerFs` 并列。DFS 的 inode 通过可变 `head_version` 指向不可变 `FileVersion`；FileVersion 再引用不可变 LayoutRoot/ExtentMap、Extent 和 ChunkObject。普通文件更新生成新 Chunk、布局和版本；镜像、Snapshot、Checkpoint 固定 FileVersion 后直接复用同一事实源进行 range read、缓存和多源 P2P。
- **接口：** 不把 BlobRecord、BlobManifest、Blob API 设为基础要求。`fsync` 强制形成完整、持久的 FileVersion，但不自动创建业务 Alias、Pin 或 RootManifest。R=1 与 R=N 只在 `ChunkStore::put` 以下分叉，上层消费相同的 ChunkReceipt。
- **共享接入：** 不新增 FUSE 中间层。当前 `src/node/fuse.rs` 是 OwnerFs 与 DistributedFs 共用的 FUSE module；两个后端分别建立独立 mount、FUSE connection、FuseSession、inode/handle table、notifier 和缓存策略。每个 Session 构造时绑定一个 Backend，不在同一 mount 内按虚拟根路由。
- **运行时边界：** `DfsWriteSession`、ChunkBuilder 和 ChunkStore 只属于 DistributedFs。OwnerFs 使用自己的本地文件句柄，只复用 FUSE/Backend 接口和公共连接工具。当前阶段不设计 OwnerFs 到 DFS 的 Snapshot 转换。
- **读取执行：** Accepted 数据模型不引入 `ReadSlice`、`ReadPlan` 或 `ChunkReadTask`；实现可以直接遍历 Extent。显式读取计划只有在后续调度设计证明必要时才作为 Node 私有类型引入。
- **依据：** 统一模型既保留 POSIX 多读多写，又让写后很少修改的主优化场景天然获得不可变身份、校验和多源读取条件，避免两套持久格式、GC 和修复逻辑。
- **限制：** 严格普通写的跨节点可见性、并发排序和失败返回仍由专题二确定；Accepted Design 不代表 DFS 数据路径已经实现。
- **权威文档：** [RFC-0002](rfcs/0002-file-version-chunk-model.md)与[专题一](architecture/01-file-version-chunk-model.md)。

## 2026-09-27：独立文件使用有界 FUSE 并发与按会话回收

- **决策：** 保持单 Home 普通文件与现有 P2P 协议；部分远端 FUSE 操作交给 8 个有界 worker，带 `fh` 的属性与 read/write/flush/fsync/release 共用同句柄 FIFO。本机只读操作与 LOOKUP 直接执行；OwnerFs 句柄 I/O 用每句柄锁，Home 对实际句柄核对根、peer 会话和授权。已确认 I/O 后远端 RELEASE 有界异步重试；Home 在 Meta 会话过期后清理 FD 和授权缓存。不引入 3FS 式 chunk 布局或新核心层。
- **依据：** 最终 Linux 三 VM 200×4 KiB/8 worker 完整 W2 两份 12 轮为 MooseFS 的 0.768/0.780，W1 0.482/0.451，功能 15/15。B 持有 FD 后 `SIGKILL`，A 实际回收 1 个句柄。LOOKUP 批量与 16 worker 同场回归，已撤销。见[修复与复验](reviews/2026-09-27-ownerfs-p2p-hardening.md)。
- **限制：** 固定并发场景通过不能外推到顺序或其他负载；顺序 W2 1.106/1.080，B 远端重读仍慢于 MooseFS。后台回收依赖 lease 和周期，不支持旧 FD 透明续用，也不替代 Meta 选主或 VM 掉电验证。

## 2026-09-27：MetaStore 统一可见状态与提交确认

- **决策：** Meta 的业务服务使用一份已提交权威状态；`src/meta/store.rs` 中的 Store 对命令排队、短窗合并，在私有副本上执行条件事务，后端确认完整状态和请求结果后才对外发布。Node 的 FUSE/P2P 已共用 OwnerFs 状态，本轮不为移动 RPC 适配器新增层。
- **依据：** 后端确认前不可见、提交失败封闭、重启重放与请求去重均有 Linux 定向测试；实现和故障边界见[提交边界](plans/2026-09-27-meta-store.md)。
- **后端：** `etcd` 是默认持久后端，`local-file` 是单机持久后端，`memory` 是易失后端。Redis 和 Meta 选主未纳入本次实现。
- **限制：** 全量状态提交的容量/延迟、多活动 Meta 和跨进程故障注入未验收；不能把新后端当前通过的功能测试写成 HA 或规模结论。
