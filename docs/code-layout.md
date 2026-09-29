# AFS 目录架构与模块职责

状态：Implementation Reference

本文描述当前源码模块和依赖方向。产品语义见[架构原则](../PRINCIPLES.md)与[架构总览](architecture/overview.md)，能力边界见[当前状态](current-status.md)。

## 顶层结构

```text
afs/
├── common/
│   ├── error/                 稳定错误身份
│   ├── protocol/              local/meta/node_control/node_data wire
│   ├── transport/             gRPC、SHM、可选 RDMA
│   ├── logging/
│   ├── metrics/
│   └── tracing/
├── client/                    本机 UDS + SHM SDK crate
├── src/
│   ├── dfs.rs                 DFS 公共域类型
│   ├── meta.rs                Meta 进程组装
│   ├── meta/
│   │   ├── dfs.rs             namespace、inode、FileVersion 提交事务
│   │   ├── owner_roots.rs     OwnerFs 根授权状态机
│   │   ├── rpc.rs             Meta RPC 校验与 wire/domain 转换
│   │   ├── store.rs           MetaStore 接口和持久实体
│   │   └── store/             memory、local-file、etcd 后端
│   ├── node.rs                Node 进程组装与独立 mount 生命周期
│   └── node/
│       ├── fuse.rs            共享 FUSE 实现；分别创建 OwnerFs/DFS session
│       ├── fuse/state.rs      单个 mount 的 inode/handle 映射
│       ├── chunk.rs           DFS R=1 immutable LocalChunkStore
│       ├── vfs.rs             单后端 `Backend` 接口
│       ├── vfs/
│       │   ├── dfs.rs         `DistributedFs`、DfsWriteSession、版本读取
│       │   ├── ownerfs.rs     OwnerFs 文件语义
│       │   ├── ownerfs/       根、catalog、文件和远端操作
│       │   └── types.rs       FUSE 无关的属性与句柄类型
│       ├── rpc/               Meta caller、Node 控制与 P2P 数据面
│       ├── api/               REST 与本机 SDK 服务端
│       └── storage/           OwnerFs/诊断普通文件机制
├── scripts/dfs/r1_e2e.py      DFS R=1 真 FUSE 纵向验收
└── tests/                     Rust 合同与 OwnerFs 验收
```

## FUSE 与后端边界

```text
/mnt/ownerfs
  → FuseSession<OwnerFs>
  → OwnerFs 自己的 inode/handle、notifier 和缓存策略

/mnt/dfs
  → FuseSession<DistributedFs>
  → DFS 自己的 inode/handle、DfsWriteSession 和 FileVersion
```

`src/node/fuse.rs` 是复用的实现模块，不是额外产品层。一个 session 只绑定一个 `Backend`，不在 mount 内做 OwnerFs/DFS namespace 路由。两个 mount 不能配置成同一路径。

`Backend` 定义 lookup、getattr/setattr、create/open/read/write/flush/fsync/release 和目录操作的共同接入形状。各后端自行决定数据模型、持久化、缓存和恢复语义；未实现操作明确返回 `UNIMPLEMENTED`。

## DFS 纵向路径

```text
FUSE request
  → node/fuse.rs
  → node/vfs/dfs.rs
      write fragments → InodeWriteState / DirtyExtentMap
      commit trigger → CommitBatch → ChunkBuilder
  → node/chunk.rs
      StagedChunk → durable ChunkReceipt
  → node/rpc/meta.rs
  → meta/rpc.rs
  → meta/dfs.rs
      one MetaStore transaction:
      Chunk + Copy + Placement + LayoutRoot + FileVersion + inode head CAS
```

读取时 `DistributedFs` 先从 Meta 固定 inode 当前 `FileVersion` 和 `LayoutRoot`，再按 Extent 定位本机 Chunk。当前 R=1 CommitPlanner 将相邻脏范围合并为不超过 4 MiB 的 Chunk，并继承 expected base 中未覆盖的 Extent；R=N 保持在 `ChunkStore::put_batch` 以下，不改变文件层合同。

当前代码已经把 dirty data 放入 inode 共享的 `InodeWriteState/DirtyExtentMap`；`DfsWriteSession` 只保存一次 open 的 flags、水位和错误游标。FrozenCommit 在 inode 锁内冻结写入前缀，Chunk I/O 与 Meta RPC 在锁外执行，后续 write 进入下一批；远端 owner routing 与并发 sync 等待尚未实现。实现状态和差异见[当前状态](current-status.md)与[专题二](architecture/02-write-durability-publication.md)。

## Meta 与协议

`common/protocol/proto/meta.proto` 按同一个 Meta 进程边界定义：

- `Meta`：Node 注册与查询；
- `OwnerRoots`：OwnerFs 根授权；
- `DfsMeta`：lookup、create、组合 OpenWrite、WriteLease renew、inode/FileVersion 查询和提交；lease acquire 是 Meta 内部步骤，不单独增加客户端冷路径 RPC。

RPC 层只负责认证、参数校验、错误映射和 wire/domain 转换。`meta/dfs.rs` 负责 DFS 业务条件与事务，`meta/store.rs` 提供统一的条件提交、请求去重和持久实体。文件数据不经过 Meta。

## Node 数据面

- OwnerFs 的跨节点文件操作位于 `node/rpc/data.rs` 和 `node/rpc/peer.rs`。
- DFS 当前只有本机 R=1 ChunkStore；后续副本协议仍进入 `node/rpc/data.rs` 和 `node/rpc/peer.rs`，不塞入 Meta RPC。所有专项完成前不按 OwnerFs/DFS 拆物理文件。
- `common/transport` 只提供 gRPC、SHM、RDMA 等传输机制，不决定文件版本、授权或提交成功。
- Node 本机 SDK 通过 UDS + sealed memfd 工作；当前尚未接入 DFS 文件批量 API。

## feature 与运行选择

- 默认 feature：`ownerfs,dfs`；
- 单独构建：`--no-default-features --features ownerfs` 或 `dfs`；
- 运行选择：`--fs ownerfs|dfs|all`；
- mount 参数：`--ownerfs-mount`、`--dfs-mount`；
- `rdma` 是独立可选 feature。

## 推荐阅读顺序

1. `src/dfs.rs`：稳定身份和对象关系；
2. `src/node/vfs.rs`、`src/node/fuse.rs`：共享接入边界与独立 session；
3. `src/node/vfs/dfs.rs`、`src/node/chunk.rs`：Node 写入和读取；
4. `common/protocol/proto/meta.proto`、`src/node/rpc/meta.rs`、`src/meta/rpc.rs`：RPC 边界；
5. `src/meta/dfs.rs`、`src/meta/store.rs`：原子发布与持久实体；
6. `scripts/dfs/r1_e2e.py`：真实进程和 FUSE 端到端行为。
