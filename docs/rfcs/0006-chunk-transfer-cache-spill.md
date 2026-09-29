# RFC-0006：固定版本 Chunk 的传输、P2P、Cache 与 Spill

状态：Accepted

目标 Milestone：M5

研究依据：[专题六](../architecture/06-reliability-performance-path.md)

上游合同：[RFC-0002](0002-file-version-chunk-model.md) · [RFC-0003](0003-write-visibility-durability.md) · [RFC-0004](0004-replication-state-machine.md) · [RFC-0005](0005-local-chunk-engine-cow.md)

实现边界：第一阶段框架已经接入 `DfsReadEngine`、本机 DurableReplica、Meta 批量选源和 gRPC Peer Range Read 的最小路径。它用于验证固定版本、选源、Range 协议和失败换源这些接口能连通；VerifiedCache、SeedLease、ExternalCommitted/Spill、SHM/RDMA、完整授权校验和生产级故障状态机尚未实现。

## 摘要

固定版本读取必须先把路径/inode 固定到 FileVersionId，再由 Extent 映射成 `ChunkId + Range`。FUSE、Native SDK 和 Block Adapter 共享 DfsReadEngine；它从 Ready DurableReplica、Ready VerifiedCache 和 Ready ExternalCommitted 中选择来源，通过 Inline、SHM、RDMA 或 Stream 搬运字节，并在 completion 通过身份与校验后发布结果。

Seed 是 Ready 本地 Copy 的短期服务能力，不是持久 Copy 类型。只有完整 Chunk 通过内容摘要校验后才能安装 VerifiedCache 和成为 Seed。Cache 不计入同步持久副本数；ExternalCommitted 默认不计入集群内 `sync_required_copies`。Spill 必须先完成外部发布、校验和 Meta 提交，才能允许本地逐出。

## 1. 规范不变量

1. 一个读取结果只能来自同一 FileVersionId；换源不能改变 FileVersionId、ChunkId 或 Range。
2. Transport adapter 不创建 FileVersion、ChunkObject、CopyRecord 或持久性证明。
3. DfsReadEngine 不按每次 read/Chunk 同步访问 Meta；它使用有 revision/epoch 的目录快照与增量更新。
4. Copy role 与 Copy state 正交：role 为 DurableReplica、VerifiedCache 或 ExternalCommitted；state 为 Ready、Corrupt 或 Deleting。
5. StagedChunk 不进入 Meta Copy Catalog，也不能被普通读取、Seed、Snapshot 或可靠性计数引用。
6. SeedLease 只能引用 Ready DurableReplica 或 Ready VerifiedCache；它是可过期软状态，不计入可靠副本数。
7. 部分 Range 可以服务读取，但只有完整 Chunk 通过 Chunk digest 校验后才能形成 VerifiedCache/Seed。
8. Cache 不满足 `sync_required_copies`。ExternalCommitted 只有在显式 tiering policy 下才计入整体 durability，且默认不替代集群内同步副本。
9. RDMA/SHM/Stream completion 不等于内容校验、持久化或状态提交。
10. Spill 的外部 Copy 达到 Ready 前，不能据此删除任何本地可靠副本。

## 2. 运行时对象

```text
ChunkReadOp {
  file_version_id
  chunk_id
  chunk_offset
  length
  destination_offset
}

ReadBatch {
  read_id
  operations[]
  deadline
  priority
  max_inflight_bytes
}

SourceCandidate {
  copy_id
  role
  location
  node_epoch / device_epoch / catalog_revision
  transport_capabilities
  load_hint
}

PayloadDescriptor {
  Inline | LocalShm | RdmaRegion | Stream
}

TransferCompletion {
  read_id
  operation_index
  attempt_id
  source_copy_id
  transferred_bytes
  range_checksum
  result
}
```

这些对象是 Node 运行时接口，不进入 FileVersion 或持久布局。`ChunkReadOp` 用于调度和任务级重试，不恢复 `ReadSlice` 持久抽象。

## 3. Copy Catalog

```text
CopyRole = DurableReplica | VerifiedCache | ExternalCommitted
CopyState = Ready | Corrupt | Deleting
```

CopyRecord 同时保存 role 和 state。现有把 DurableReplica、Corrupt、Deleting 放在一个枚举中的代码需要迁移；旧 LegacyStaging 只允许解码兼容，不能生成。

- DurableReplica/Ready：可读、可 seed，并在 placement 合格时计入本地持久副本；
- VerifiedCache/Ready：可读、可 seed、可淘汰，不计入同步副本；
- ExternalCommitted/Ready：可回源并支持安全逐出，计数由 tiering policy 决定；
- Corrupt：不可读、不可 seed、不可计数；
- Deleting：不接受新读取，已有 pin 排空后删除。

## 4. 固定版本读取

规范顺序：

1. Resolve inode/path 并固定 FileVersionId；
2. 读取 LayoutRoot/ExtentMap；
3. 将 Range 映射为 ChunkReadOp，Hole 由文件层填零；
4. 从本机 durable、本机 cache、Peer seed/replica、external 中选择来源；
5. 按来源与 Transport 合批，受 backpressure 限制；
6. 搬运到 attempt-scoped 目标 Buffer；
7. 校验长度、Range checksum 和必要的完整 Chunk digest；
8. 只发布当前 attempt 的 completion；
9. 满足完整 Chunk 门禁时安装 cache 并异步 announce seed。

Source 默认偏好本机 durable、本机 cache、近端 seed、远端 durable、远端 cache seed、external；调度器可以按负载与失败动态调整。

## 5. SeedLease

SeedLease 至少包含 `chunk_id/copy_id/node_id/node_epoch/endpoint/expires_at/load_hint`。

- announce、renew、revoke 使用异步批量更新；
- Node 重启后旧 lease 因 node_epoch 或过期失效；
- Copy 进入 Corrupt/Deleting、Node drain 或压力过高时停止服务；
- 请求端仍须检查授权、版本身份与 completion，不能只信 Seed Directory。

## 6. Transport

- 小数据允许 Inline；
- 同节点 SDK 使用 SHM/memfd；
- 大型跨节点 Range 在能力允许时使用 RDMA registered buffer；
- 通用回退是带背压的 Stream。

阈值是配置与实测结果，不进入文件语义。PeerConnectionPool 统一负责连接复用、capability negotiation、keepalive、LRU、重连、Node epoch、并发、排队字节和 circuit breaker。BufferPool 统一负责普通/SHM/RDMA buffer、pinned memory 上限和 attempt 生命周期。

每个 completion 必须带 attempt_id。超时换源后，迟到 completion 不得覆盖或发布新 attempt 的目标范围。

## 7. Cache

```text
full Chunk receive
  → verify length + Chunk digest
  → no-replace local publish
  → LocalChunkRecord(VerifiedCache, Ready)
  → local read enabled
  → async SeedLease announce
```

Range-only 结果可以进入临时请求级缓存，但不能登记为 VerifiedCache 或 Seed。若以后需要可验证 segment cache，必须单独 RFC 定义 Merkle/segment digest 持久格式。

同一 Node 对相同 Chunk 的并发 miss 必须 coalesce；相同 ChunkId 的缓存安装以内容身份幂等。

## 8. Spill 与 Recall

Spill 顺序：

1. 创建 operation-scoped 外部临时对象；
2. 上传完整 Chunk；
3. 校验长度和 Chunk digest；
4. 原子发布外部对象；
5. Meta 提交 `ExternalCommitted + Ready`；
6. 依据 durability、pin、引用和 in-flight I/O 判断本地逐出；
7. 本地 Copy 进入 Deleting，排空后删除。

外部写成功、Meta 提交失败时，外部对象是 orphan，本地 Copy 保持不变。Recall 可以直接服务 Range；只有完整校验后才能形成 VerifiedCache 或经 Promotion 成为 DurableReplica。

## 9. RPC 与资源预算

- 稳态 FUSE read：0 Meta RPC；
- Copy/Seed 目录：快照 + 增量缓存，不按 Chunk 同步查询；
- 同一 ReadBatch/来源：尽量一次合批 Peer 请求；
- 连接：按 Peer 复用，不按 Chunk 建立；
- Seed/Cache 目录变更：异步批量；
- 背压：全 Node、每 Peer、每设备、每租户四层字节/并发预算；
- 大数据：避免多次中间 Vec 拷贝，直接写入受管理目标 Buffer。

## 10. 幂等与故障

- 普通读重试使用新的 attempt_id；
- cache install、promotion、spill、recall、repair 和 relocation 使用持久 OperationId；
- Seed lease 是软状态，不要求持久 OperationId；
- 校验失败的来源退出本次候选并触发 copy health 处理；
- 所有来源耗尽时返回明确 I/O 错误，不返回旧版本、零数据或 staging；
- 外部提交 result-unknown 通过 OperationId 查询原结果；
- Node 重启从 LocalCatalog 恢复 Ready Copy，再重新 announce Seed。

## 11. 模块边界

- DistributedFs：固定 FileVersion、遍历 Extent、处理 Hole；
- DfsReadEngine：ReadBatch、选源、合批、限流、重试、校验和发布；
- DfsChunkStore/LocalChunkStore：本机 DurableReplica；
- ChunkCache：VerifiedCache 生命周期和 in-flight coalescing；
- ChunkTransfer：Peer Range/Chunk 搬运与 PayloadDescriptor；
- SpillStore：外部提交、回源、删除和对账；
- PeerConnectionPool/BufferPool：传输公共资源；
- rpc::data/rpc::peer：入站/出站协议适配。

Source Resolver、Retry Controller 和 Batch Scheduler 第一版留在 DfsReadEngine 内部，不提前拆成独立核心模块。

## 12. 被拒绝方案

- Seed 作为持久 Copy 类型；
- Cache 自动计入持久副本；
- partial range 宣布完整 Seed；
- 每个 read/Chunk 同步访问 Meta；
- RDMA completion 直接视为内容正确或持久化；
- Range checksum 替代 Chunk digest；
- 外部上传完成但 Meta 未提交就逐出本地数据；
- 独立镜像 Blob 数据面；
- 恢复持久 ReadSlice/ReadPlan。

## 13. 验收标准

- FUSE、SDK、Block Adapter 对同一 FileVersion/Range 返回一致结果；
- local durable、cache、Peer 与 external 可按规则换源；
- partial range 不可晋升，完整 Chunk 校验后才能 cache/seed；
- 迟到 completion 被 attempt fencing 拒绝；
- spill 每个故障切点都不删除唯一事实源；
- 热路径无 per-read Meta RPC 和 per-Chunk 建连；
- 大规模启动报告 origin/P2P bytes、seed growth、回源比例、完成时间、CPU 和内存；
- SHM/RDMA/Stream 报告吞吐、延迟、复制次数、pinned memory 和降级结果。
