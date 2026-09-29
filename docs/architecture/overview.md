# AFS 架构总览

状态：Accepted Design
实现状态：Partial
产品合同：[产品定位](../product-positioning.md) · [架构原则](../../PRINCIPLES.md)

详细设计入口：[架构设计专题](design-topics.md)。专题一的数据模型已经 Accepted；其余专题仍处于 Research，不代表对应能力已经实现。

## 系统结构

OwnerFs 和 DistributedFs 使用同一份 FUSE 模块代码与 `Backend` 接口，但分别建立独立 mount、FUSE connection、`FuseSession`、inode table 和 handle table。一个 `afs-node` 进程可以同时承载两个 mount。

```mermaid
flowchart LR
    AppO[Workspace Application] --> MountO[/mnt/ownerfs]
    AppD[Distributed Application] --> MountD[/mnt/dfs]
    AppSDK[High-performance Application] --> SDK[Native Async SDK]
    VM[MicroVM] --> Block[Block Adapter]

    subgraph NodeA[Compute Node A / afs-node]
        FuseCode[Shared FUSE module<br/>src/node/fuse.rs]
        OwnerSession[OwnerFs FuseSession]
        DfsSession[DFS FuseSession]
        Owner[OwnerFs]
        DFS[DistributedFs]
        DfsWrite[DfsWriteSession / ChunkBuilder]
        LocalStore[ChunkStore]
        LocalDisk[(Local SSD / HDD)]

        FuseCode -.instantiates.-> OwnerSession
        FuseCode -.instantiates.-> DfsSession
        OwnerSession --> Owner
        DfsSession --> DFS
        SDK --> DFS
        Block --> DFS
        DFS --> DfsWrite --> LocalStore --> LocalDisk
    end

    subgraph MetaCluster[AFS Meta Cluster]
        Meta[MetaService]
        MetaStore[(MetaStore)]
        Namespace[NamespaceService<br/>Dentry / InodeRecord]
        Version[VersionService<br/>FileVersion / LayoutRoot]
        Placement[PlacementService<br/>Policy / ReplicaGroup]
        Copies[CopyCatalog]
        Meta --> MetaStore
        Meta --> Namespace
        Meta --> Version
        Meta --> Placement
        Meta --> Copies
    end

    subgraph StoragePeers[Storage Nodes]
        StoreB[ChunkStore B]
        StoreC[ChunkStore C]
        Pool[PeerConnectionPool]
        StoreB <--> Pool
        Pool <--> StoreC
    end

    Object[(Optional OBS / S3)]

    MountO --> OwnerSession
    MountD --> DfsSession
    Owner -->|root auth / home| Meta
    DFS -->|resolve / commit version| Meta
    Meta -->|policy + epoch| DFS
    LocalStore -->|replicate / range read| StoreB
    StoreB --> StoreC
    LocalStore --> Object
    StoreB --> Object
```

Compute Node 与 Storage Node 可以同机或同进程部署，但逻辑边界保持独立。

## FUSE 与 mount 边界

`FUSE module` 直接对应当前代码中的 `src/node/fuse.rs`，不增加 `FuseFrontend`。它实现内核 FUSE 协议、挂载会话、FUSE inode/handle 映射、请求调度、errno 和 reply。

共用的是代码和接口：

```text
fuse module
Backend trait
request / reply conversion
error mapping
```

运行时不共用：

```text
/mnt/ownerfs
  -> OwnerFs FuseSession
  -> OwnerFs inode/handle table
  -> OwnerFs cache and invalidation policy
  -> OwnerFs Backend

/mnt/dfs
  -> DFS FuseSession
  -> DFS inode/handle table
  -> DFS cache and invalidation policy
  -> DistributedFs Backend
```

每个 `FuseSession` 在创建时绑定一个确定的 Backend，不在同一 mount 内根据 inode 或虚拟根选择 OwnerFs/DFS。Native SDK 与 Block Adapter 直接进入 DistributedFs，不构造 FUSE 请求。

## Meta 核心模块

| 模块 | 核心类型 | 职责 |
| --- | --- | --- |
| `NamespaceService` | `Dentry`、`InodeRecord` | 路径、目录项、inode 属性和当前 `head_version_id` |
| `WriteLeaseService` | `WriteLease` | 活跃 inode owner、lease epoch、续约与 fencing |
| `VersionService` | `FileVersion`、`LayoutRoot`、`ExtentMapNode`、`Extent` | 不可变文件版本、布局查找和 Head CAS |
| `PlacementService` | `DurabilityPolicy`、`ReplicaGroup`、`PlacementRecord` | 选择副本组、持久性策略和 PlacementEpoch |
| `CopyCatalog` | `CopyRecord` | Durable Replica、Verified Cache 和 External Copy 的动态位置目录 |
| `LifecycleService` | `Alias`、`PinRecord`、`RootManifest` | 固定版本、保留与多文件一致视图 |
| `MetaStore` | 条件事务与修订 | 提交后权威状态、幂等结果和恢复 |

Meta 不转发文件内容，不参与每个 FUSE WRITE，也不参与每个已解析 Chunk 的读取。

## Node 核心模块

| 模块 | 核心类型 | 职责 |
| --- | --- | --- |
| `fuse` | `FuseSession`、FUSE inode/handle 映射 | 两个 mount 复用的内核协议适配代码 |
| `ownerfs` | `OwnerFsHandle` | Home 本地普通文件与远端回 Home；不使用 DFS Chunk 模型 |
| `dfs` | `DistributedFs`、`DfsFileHandle` | DFS Backend、打开版本和 POSIX 文件操作编排 |
| `write` | `DfsWriteSession`、`InodeWriteState`、`DirtyExtentMap`、`CommitBatch` | owner 排序、共享 dirty view、同步屏障、后台 writeback 和版本提交 |
| `chunk` | `StagedChunk`、`ChunkObject`、`ChunkReceipt`、`ChunkStore` | CommitBatch 内部的 Chunk 构建、校验、Finalize、本地读写和持久性结果 |
| `replication` | `ReplicationEngine`、`ReplicaGroup` | R=N 数据复制、确认、修复和重配置 |
| `peer` | `PeerConnectionPool` | OwnerFs 与 DFS 可复用的连接管理；业务协议保持分开 |
| `cache` / `spill` | `CopyRecord` 对应的本地执行状态 | 缓存驱逐、外部写穿和容量分层 |

`DfsWriteSession`、`InodeWriteState` 和 CommitBatch 是 DFS 专属运行时状态。OwnerFs 只复用共享 FUSE handle 生命周期，使用自己的 `OwnerFsHandle`，不经过 DFS 的 WriteLease、FileVersion、Extent 或 ChunkStore 状态机。

## DFS 数据模型

```text
Dentry
  -> InodeRecord
       -> head_version
            -> FileVersion
                 -> LayoutRoot / inline Extents
                      -> Extent[]
                           -> ChunkObject[]
```

可变对象：Dentry、`InodeRecord.head_version_id`、WriteLease、Node 上的 InodeWriteState/DirtyExtentMap、DfsWriteSession、CommitBatch、ChunkStore 内部 StagedChunk、Placement 和 Copy Catalog。

不可变对象：FileVersion、LayoutRoot、Extent Tree Node、Extent、ChunkObject 和 RootManifest。

## 写入数据路径

```text
user      open        write       read         fsync              ok       close
───────────●────────────●───────────●────────────●──────────────────●──────────●────>

node     session       dirty      overlay       freeze    chunk   commit    release
───────────●────────────●───────────●────────────●─────────●────────●──────────●────>

meta     lease / V7                                         V8
───────────●─────────────────────────────────────────────────●──────────────────────>
```

普通 write 由 inode owner 排序并进入共享 dirty overlay，不创建 FileVersion。`fdatasync`、`fsync`、同步 write 或后台 writeback 冻结一个写入前缀，经 ChunkStore 获得 ChunkReceipt，再用一次 Meta CAS 创建并发布新 FileVersion。只有同步调用向用户提供对应的完成保证；文件同步与目录项 `fsync(dir)` 是两个合同。

R=1 与 R=N 在 `ChunkStore::put` 以下分叉。文件布局层只消费满足策略的 ChunkReceipt。

## 固定版本的多源读取

```text
resolve path
  -> fix FileVersionId
  -> map range through LayoutRoot
  -> obtain ChunkIds
  -> select DurableReplica / VerifiedCache / ExternalCommitted
  -> read ranges
  -> verify Chunk identity and digest
```

当前数据模型不定义 `ReadSlice`、`ReadPlan` 或 `ChunkReadTask`。实现可以直接遍历 Extent；只有后续证明并行调度、合并和任务级重试需要显式执行计划时，才增加 Node 私有运行时类型。

## OwnerFs 路径

```text
Home local access:
Application -> /mnt/ownerfs -> OwnerFs FuseSession -> OwnerFs -> local filesystem

Remote access:
Application -> local OwnerFs mount -> local Node -> P2P -> Home Node -> local filesystem
```

OwnerFs 与 DFS 当前没有转换合同。跨后端 Snapshot、rename 和 hard link 不在本阶段设计范围内。

## 接口层

| 入口 | 使用者 | 主要用途 |
| --- | --- | --- |
| OwnerFs POSIX/FUSE mount | 1～4 节点 Agent Workspace | Home 本地亲和或远端回 Home |
| DFS POSIX/FUSE mount | 普通 Linux 应用 | 通用目录和文件操作 |
| Native Async SDK | 数据加载器和高性能应用 | Batch Range、共享内存、异步完成 |
| Block Adapter | Firecracker 等 MicroVM | 固定基础版本和私有写层 |
| Runtime API | Sandbox 管理器 | DFS Snapshot、Pin、Alias、Restore |
| Management API | 控制面和调度器 | Node、Home、Replica、容量和 Placement 查询 |
