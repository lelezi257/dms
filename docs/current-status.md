# AFS 当前状态

更新时间：2026-09-29

## 当前架构

- OwnerFs 与 DistributedFs 使用两个独立 mount 和 FuseSession。两者复用 `src/node/fuse.rs` 与 `Backend` 接口，各自维护 FUSE connection、inode/handle table 和缓存策略。
- DFS 公开类型为 `DistributedFs`；内部模块、feature、配置、CLI 和协议统一使用 `dfs` / `DfsMeta`。
- DFS 的可变 inode head 指向不可变 `FileVersion`；`FileVersion → LayoutRoot → Extent → ChunkObject` 形成已提交读取视图。
- 专题二已经接受 WriteLease + inode owner 模型：普通 write 进入共享 dirty view，CommitTrigger 才生成 Chunk 和 FileVersion；代码尚未实现该模型。
- R=1 与未来 R=N 的分叉位于 `ChunkStore::put` 以下，文件层只消费 `ChunkReceipt`。
- Node RPC 保持 `control.rs`、`data.rs`、`meta.rs`、`peer.rs` 四个职责文件；OwnerFiles 已合并回 `data.rs`，所有专项完成前不按 OwnerFs/DFS 提前拆文件。

## 能力矩阵

| 能力 | 状态 | 当前事实 | 主要缺口 |
| --- | --- | --- | --- |
| `afs-meta` / `afs-node` | Experimental | CLI/TOML、REST、gRPC、观测和退出已接入 | 生产部署、滚动升级和多 Meta 选主 |
| MetaStore | Experimental | etcd、local-file、memory 共用提交入口 | 多活动 Meta 与大规模故障注入 |
| 独立 FUSE mount | Experimental | OwnerFs 和 DFS 分别建立 session | 完整 POSIX 兼容矩阵 |
| OwnerFs | Experimental | 本机文件、P2P 回 Home、根授权和句柄回收 | 常用属性、根删除、全局列举、掉电与长稳 |
| DistributedFs R=1 | Experimental | create/write/fsync/reopen/read；本机不可变 Chunk；Meta 原子发布 FileVersion | 覆盖写、目录操作、Extent 树、R=N、多节点读取 |
| 固定版本多源 P2P | Accepted Design | 读取先固定 FileVersion，再按 Chunk 选来源 | tracker、选源、限流和产品 E2E |
| UDS + SHM SDK | Experimental foundation | 本地 API、memfd 和 FD passing 已接线 | 正式文件批量异步 API |
| RDMA transport | Experimental foundation | RXE 握手、READ/WRITE 和诊断链通过 | DFS 文件内容路径和硬件吞吐 |
| 对象存储 spill | Research | 独立兼容对象存储实验存在 | 外部提交、逐出、recall 和灾难恢复 |

## DFS R=1 已验证链路

```text
FUSE create /hello.txt
  → DfsMeta::Create 建立 InodeRecord
  → 当前实现将分段 write 汇入 handle 私有 DfsWriteSession
  → 当前 fsync 直接构建 StagedChunk 并原子落入 LocalChunkStore
  → DfsMeta::CommitFileVersion 在同一事务写入
       ChunkObject + CopyRecord + PlacementRecord
       + LayoutRoot + FileVersion + inode head CAS
  → close/reopen 固定新的 FileVersion
  → Extent 定位 Chunk 并校验内容身份后读取
```

Linux 真 FUSE 验收使用 local-file MetaStore，验证两次分段写、`fsync`、关闭、重新打开、读取、文件长度及唯一不可变 Chunk 内容。入口：`scripts/dfs/r1_e2e.py`。

上图描述已经运行的 R=1 实现。目标合同由专题二定义：分段 write 进入 inode 共享的 `InodeWriteState/DirtyExtentMap`，同步或后台 `CommitTrigger` 冻结 `CommitBatch`，再构建不可变 Chunk 并提交新的 FileVersion。

## 当前语义与限制

- `fdatasync/fsync` 是用户可依赖的已提交版本边界；后台 writeback 可以提交内部版本但不产生用户同步保证。当前实现为避免 close 丢失 dirty 数据，仍在 `flush` 和 `release` 时执行相同提交，与 RFC-0003 的目标合同不同。
- 当前 DfsWriteSession 仍拥有 handle 私有 dirty data，尚未实现 WriteLease、inode owner、共享 InodeWriteState、跨节点 dirty read、`O_DSYNC/O_SYNC`、sticky error 和 `fsync(dir)`。
- 当前一个 FileVersion 物化为一个 whole-file Chunk 和 inline extent。覆盖写、分块、Extent 树和 compaction 尚未实现。
- 当前 digest 是依赖零外部库的 16 字节损坏检测。分布式去重和 P2P 前必须替换为带算法版本的强摘要。
- 当前 commit 在进程级 handle table 锁内执行磁盘 I/O 和 Meta RPC。扩展并发前必须改为每句柄同步。
- 只支持默认 namespace 下最小普通文件链路。目录、rename、unlink、append/truncate、并发 writer 和完整 POSIX 尚未闭合。
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

设计入口：[专题一](architecture/01-file-version-chunk-model.md)、[RFC-0002](rfcs/0002-file-version-chunk-model.md)、[专题二](architecture/02-write-durability-publication.md)和 [RFC-0003](rfcs/0003-write-visibility-durability.md)。工程优先级见[实现任务](next.md)与[Roadmap](../ROADMAP.md)。
