# AFS 当前状态

## 2026-09-28：DFS UML、双 mount 与核心模块边界已固化

- **Accepted Design：** OwnerFs 与 DistributedFs 使用两个独立 mount 和 FuseSession；共享的是 `src/node/fuse.rs` 的实现代码与 `Backend` 接口，不共享 FUSE connection、会话 inode/handle table、notifier、缓存策略或数据模型。
- **Accepted Design：** Meta 分为 `NamespaceService`、`VersionService`、`PlacementService`、`CopyCatalog`、`LifecycleService` 和 `MetaStore`；Node DFS 分为 `distributedfs`、`write`、`chunk`、`replication`、`cache/spill`，并复用公共 `fuse` 与 `peer`。
- **Accepted Design：** `DfsWriteSession` 是 DFS 专属运行时类型；OwnerFs 使用自己的本地句柄。当前不设计 OwnerFs 到 DFS 的 Snapshot 转换，也不把 `ReadSlice/ReadPlan/ChunkReadTask` 纳入已接受数据模型。
- **实现边界：** 当前源码仍是单 mount、多 namespace VFS 骨架，并保留 `blobfs` 历史名称；目标结构尚未实现。权威 UML 与模块关系见[专题一](architecture/01-file-version-chunk-model.md)和[架构总览](architecture/overview.md)。

状态：Implemented Capability Index
更新时间：2026-09-28
详细实验与阶段记录：[status.md](status.md)

## 能力矩阵

| 能力 | 状态 | 当前事实 | 主要缺口 |
| --- | --- | --- | --- |
| `afs-meta` / `afs-node` 进程 | Experimental | CLI/TOML、REST、gRPC、观测和退出已接入 | 生产部署、滚动升级和多 Meta 选主 |
| MetaStore | Experimental | etcd、local-file、memory 共用 Store 提交入口 | 全量状态规模、多活动 Meta、跨进程故障注入 |
| FUSE/VFS | Experimental | Linux 真 FUSE 和 OwnerFs 路径已接通 | 完整 POSIX 兼容矩阵 |
| OwnerFs | Experimental | 本机普通文件、P2P 回 Home、根授权和句柄回收已接通 | 常用属性、根删除、全局列举、掉电与长稳 |
| UDS + SHM SDK 基础 | Experimental | 本地 API、memfd 和 FD passing 已接线 | 正式文件批量异步 API |
| RDMA transport | Experimental | RXE 握手、READ/WRITE 和诊断链通过 | 文件内容路径和硬件吞吐 |
| DistributedFs | Planned | 产品合同和数据模型已接受；VFS 骨架位置存在 | FileVersion、ExtentMap、ChunkStore、副本与数据路径 |
| FileVersion 数据模型 | Accepted Design | RFC-0002 定义统一不可变版本模型 | 实现与故障验证 |
| 多写可见性 | Research | 专题二已定义研究边界 | 排序、CAS、跨节点可见与错误合同 |
| 固定版本多源 P2P | Accepted Design | 固定 FileVersion 后按 Chunk 从合格来源读取 | tracker、选源、限流和产品 E2E |
| 对象存储 spill | Research | 独立兼容对象存储实验存在 | 外部提交、逐出、recall 和灾难恢复 |

## 已验证边界

### OwnerFs

- 一个 WorkspaceRoot 具有稳定 Home 和授权身份。
- 本地操作在 Home 的普通文件上执行。
- 远端 Node 通过 P2P 操作同一份 Home 文件。
- close-to-open、旧文件描述符身份、同名重建和跨根 `EXDEV` 已进入三节点功能验收。
- Meta/Home 重启后通过重新打开恢复；运行中的旧 FD 不承诺无感续接。

证据入口：[OwnerFs P2P 并发优化](plans/2026-09-27-ownerfs-p2p-concurrency.md)、[根恢复审视](reviews/ownerfs-v16-root-recovery.md)。

### DistributedFs 设计

- inode 的可变字段是 `head_version`；提交后的 FileVersion、布局和 Chunk 不原地修改。
- 小覆盖写以新 Chunk 和 Extent overlay 表达，后台 compaction 处理碎片。
- R=1 和 R=N 在 `ChunkStore::put` 以下分叉，文件层不感知副本协议。
- `fsync` 必须完成数据 flush 和 FileVersion 提交，但不等于业务 Snapshot、Alias 或 Pin。
- 固定版本多源读取先固定 `FileVersionId`，防止不同来源返回不同版本的同一范围。

设计入口：[专题一](architecture/01-file-version-chunk-model.md)、[RFC-0002](rfcs/0002-file-version-chunk-model.md)。

## 尚未实现

- DFS 通用数据路径和分布式 POSIX；
- `StagedChunk → ChunkObject`、ExtentMap、FileVersion 提交；
- 可变文件多副本协议和跨节点写入可见性；
- 固定版本的多源 P2P、Pin、Alias、RootManifest 和 GC；
- 自动 spill、逐出和 recall；
- 文件内容 Native Async SDK；
- MicroVM 块设备正式适配；
- Meta HA 和生产长稳。

## 代码入口

| 领域 | 入口 |
| --- | --- |
| 进程 | `src/bin/afs-meta.rs`、`src/bin/afs-node.rs` |
| Meta | `src/meta.rs`、`src/meta/store.rs`、`src/meta/owner_roots.rs` |
| Node | `src/node.rs`、`src/node/fuse.rs`、`src/node/vfs.rs` |
| OwnerFs | `src/node/vfs/ownerfs.rs`、`src/node/vfs/ownerfs/` |
| DistributedFs 骨架 | `src/node/vfs/blobfs.rs`（历史占位名，待实现时重命名） |
| P2P | `src/node/rpc/peer.rs`、`src/node/rpc/data/owner.rs` |
| SDK | `client/src/`、`src/node/api/local.rs` |
| Transport | `common/transport/` |
| Protocol | `common/protocol/` |
