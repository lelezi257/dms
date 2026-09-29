# AFS 当前状态

更新时间：2026-09-29

## 当前架构

- OwnerFs 与 DistributedFs 使用两个独立 mount 和 FuseSession。两者复用 `src/node/fuse.rs` 与 `Backend` 接口，各自维护 FUSE connection、inode/handle table 和缓存策略。
- DFS 公开类型为 `DistributedFs`；内部模块、feature、配置、CLI 和协议统一使用 `dfs` / `DfsMeta`。
- DFS 的可变 inode head 指向不可变 `FileVersion`；`FileVersion → LayoutRoot → Extent → ChunkObject` 形成已提交读取视图。
- 专题二的 R=1 基础框架已经实现：普通 write 进入 inode 共享 dirty view，CommitTrigger 才生成 Chunk 和 FileVersion；跨节点 owner routing 尚未实现。
- 专题三和 RFC-0004 已接受：R=1 与 R=N 的分叉位于 `ChunkStore::put_batch` 以下，文件层只消费 `ChunkReceipt`；R1 使用无 Peer 的 Local Fast Path，RN 使用 ReplicationEngine，二者复用 LocalChunkStore。
- 副本数由 `desired_copies/sync_required_copies` 配置；Meta 维护权威 PlacementSnapshot，Node 生成单次 ReplicationPlan。该多副本框架尚未实现。
- Node RPC 保持 `control.rs`、`data.rs`、`meta.rs`、`peer.rs` 四个职责文件；OwnerFiles 已合并回 `data.rs`，所有专项完成前不按 OwnerFs/DFS 提前拆文件。

## 能力矩阵

| 能力 | 状态 | 当前事实 | 主要缺口 |
| --- | --- | --- | --- |
| `afs-meta` / `afs-node` | Experimental | CLI/TOML、REST、gRPC、观测和退出已接入 | 生产部署、滚动升级和多 Meta 选主 |
| MetaStore | Experimental | etcd、local-file、memory 共用提交入口 | 多活动 Meta 与大规模故障注入 |
| 独立 FUSE mount | Experimental | OwnerFs 和 DFS 分别建立 session | 完整 POSIX 兼容矩阵 |
| OwnerFs | Experimental | 本机文件、P2P 回 Home、根授权和句柄回收 | 常用属性、根删除、全局列举、掉电与长稳 |
| DistributedFs R=1 | Experimental | WriteLease；inode 共享 dirty view；fdatasync 数据版本提交；fsync 完整属性同步；本机不可变 Chunk；Meta CAS | 远端 owner、目录同步、Extent 树、R=N、多节点读取 |
| R=N 副本协议 | Accepted Design | 可配置 N/M 策略、ACK/Receipt、PlacementEpoch、异步补副本与故障合同已确定 | ReplicationEngine、Peer Chunk RPC、Repair 和故障注入 |
| 固定版本多源 P2P | Accepted Design | 读取先固定 FileVersion，再按 Chunk 选来源 | tracker、选源、限流和产品 E2E |
| UDS + SHM SDK | Experimental foundation | 本地 API、memfd 和 FD passing 已接线 | 正式文件批量异步 API |
| RDMA transport | Experimental foundation | RXE 握手、READ/WRITE 和诊断链通过 | DFS 文件内容路径和硬件吞吐 |
| 对象存储 spill | Research | 独立兼容对象存储实验存在 | 外部提交、逐出、recall 和灾难恢复 |

## DFS R=1 已验证链路

```text
FUSE create /hello.txt
  → DfsMeta::Create 建立 InodeRecord + WriteLease
  → 分段 write 汇入 inode 共享 InodeWriteState / DirtyExtentMap
  → 另一个 handle 读取 committed base + dirty overlay
  → fdatasync 冻结当前水位，构建 StagedChunk 并落入 LocalChunkStore
  → DfsMeta::CommitFileVersion 在同一事务写入
       ChunkObject + CopyRecord + PlacementRecord
       + LayoutRoot + FileVersion + inode head CAS
  → 覆盖写进入新的 dirty 水位，fsync 提交第二个 FileVersion 和完整属性
  → close/reopen 读取新的 committed FileVersion
  → Extent 定位 Chunk 并校验内容身份后读取
```

Linux 真 FUSE 验收使用 local-file MetaStore，验证同步前跨 handle dirty read、`fdatasync(V1)`、覆盖写、`fsync(V2)`、关闭、重新打开、读取、文件长度及两份不可变 Chunk 内容。入口：`scripts/dfs/r1_e2e.py`。

## 当前语义与限制

- `fdatasync/fsync` 是用户可依赖的同步边界：存在 dirty data 时提交新的 FileVersion；数据已经由 `fdatasync` 提交后，紧接的 `fsync` 只用 fenced Meta CAS 补齐完整 inode 属性，不虚构第二个 FileVersion。后台 writeback 可以提交内部版本但不产生用户同步保证。`flush` 只报告已知后台错误，`release` 只释放 handle，并在最后一个 writer 关闭时请求后台提交。
- 当前已实现 WriteLease、本地 inode owner、共享 InodeWriteState、跨 handle dirty read、`O_DSYNC/O_SYNC` 和 sticky error；跨节点 owner routing 与 `fsync(dir)` 尚未实现。
- 当前 FileVersion 使用 inline Extent 列表。CommitPlanner 已对脏范围分块并继承未覆盖的 base Extent；Extent 树和布局 compaction 尚未实现。
- Chunk 身份已使用带算法标识的 BLAKE3-256；旧 16 字节 FNV 原型格式不兼容。
- 当前 commit 使用 FrozenCommit 把磁盘 I/O 和 Meta RPC 移出 inode 写状态锁；同一 inode 的第二个同步请求当前返回 busy，等待与合并策略尚未实现。
- LocalChunkStore 已有批量 durable finalize、LocalCatalog 恢复和 reader FD pin；Pack、删除状态机、orphan reconciliation 与掉电故障矩阵尚未实现。
- 只支持默认 namespace 下最小普通文件链路。目录、rename、unlink、完整 truncate、跨 Node writer 和完整 POSIX 尚未闭合。
- R=N、多源读取、cache/spill、Pin/Alias/RootManifest、Native SDK 文件数据面和 MicroVM 块设备不在本阶段。
- Meta 单活动围栏与选主尚未完成，不能宣称生产 HA。

## 代码入口

| 领域 | 入口 |
| --- | --- |
| DFS 公共数据模型 | `src/dfs.rs` |
| Meta 服务与事务 | `src/meta/dfs.rs`、`src/meta/rpc.rs`、`src/meta/store.rs` |
| DFS Node | `src/node/vfs/dfs.rs` |
| 本机 ChunkStore | `src/node/chunk.rs` |
| 独立 FUSE session | `src/node/fuse.rs`、`src/node/fuse/state.rs` |
| OwnerFs | `src/node/vfs/ownerfs.rs`、`src/node/vfs/ownerfs/` |
| 协议 | `common/protocol/proto/meta.proto` |
| E2E | `scripts/dfs/r1_e2e.py` |

设计入口：[专题一](architecture/01-file-version-chunk-model.md)、[RFC-0002](rfcs/0002-file-version-chunk-model.md)、[专题二](architecture/02-write-durability-publication.md)、[RFC-0003](rfcs/0003-write-visibility-durability.md)、[专题三](architecture/03-replication-state-machine.md)和 [RFC-0004](rfcs/0004-replication-state-machine.md)。工程优先级见[实现任务](next.md)与[Roadmap](../ROADMAP.md)。
