# RFC-0004：Chunk 单副本与多副本状态机

状态：Accepted / Framework Implemented
目标 Milestone：M3
研究依据：[专题三](../architecture/03-replication-state-machine.md)
数据模型：[RFC-0002](0002-file-version-chunk-model.md)
写入合同：[RFC-0003](0003-write-visibility-durability.md)

## 摘要

DistributedFs 在 `ChunkStore::put(StagedChunk)` 内部分成两条路径：本地单副本快速路径和多副本协调路径。文件层只得到 `ChunkReceipt`，不感知副本数量、目标节点或传输方式。

整个文件系统只有一份 `ReplicationConfig`。它在 Meta 初始化文件系统时持久化，后续启动只能使用完全相同的值。改变配置需要停止并重新初始化文件系统，不支持 inode 级策略或在线策略 revision。

Chunk 内容不可变。每个副本复用 `LocalChunkStore` 的校验、fsync 和原子 finalize 原语。gRPC 流和 RDMA 单边传输只是 `ReplicaDataPlane` 的两种字节搬运实现，不改变 `ReplicationPlan → ReplicaAck → ChunkReceipt → FileVersion CAS` 状态机。

## 1. 规范边界

```text
CommitBatch
  → StagedChunk
  → ChunkStore::put
      ├── R1 Local Fast Path
      └── RN ReplicationEngine
  → ChunkReceipt
  → FileVersion CAS
```

- R1 与 RN 只在 ChunkStore 内部分叉。
- `LocalChunkStore` 是每个物理副本共用的本地持久化原语。
- `ReplicationEngine` 只负责 placement、传输和 ACK 聚合。
- Meta CAS 是新 FileVersion 的唯一 committed visibility point。
- FileVersion、Extent、ChunkId 不包含 ReplicaGroup、Chain 或传输类型。

## 2. 文件系统级 ReplicationConfig

```text
ReplicationConfig {
  desired_copies
  sync_required_copies
  min_distinct_nodes
  min_distinct_failure_domains
  local_copy: Required | Preferred | NotRequired
}
```

约束：

1. `1 <= sync_required_copies <= desired_copies`。
2. 两个 distinct 下限必须非零且不大于 `desired_copies`。
3. 同一 Node 上的多个设备不能满足 `min_distinct_nodes > 1`。
4. `local_copy=Required` 时，调用 Node 的副本必须在同步 ACK 集合内。
5. `sync_required_copies < desired_copies` 时，Meta 在 FileVersion CAS 同一事务中登记 `ReplicationTask`。
6. N=1、2、3、4 使用同一配置类型、receipt 类型和 Meta 校验逻辑。

配置由 `afs-meta` 的 `dfs_desired_copies`、`dfs_sync_required_copies`、`dfs_min_distinct_nodes`、`dfs_min_distinct_failure_domains` 和 `dfs_local_copy` 初始化。持久值与启动值不同则 Meta 拒绝启动 DFS authority。

## 3. Placement 权威

Meta 返回权威 `PlacementSnapshot`：

```text
PlacementSnapshot {
  revision
  replication
  replica_groups[] {
    replica_group_id
    placement_epoch
    targets[] {
      node_id / node_epoch
      data_endpoint
      device_id / device_epoch / catalog_revision
      failure_domain
    }
  }
}
```

Node 缓存快照，再为一个 StagedChunk 生成临时 `ReplicationPlan`。热路径不按 Chunk 查询 Meta。Node 不能降低同步数量，不能自行加入快照外 Target。

`NodeEpoch` 表示 Node 进程 session。正常心跳只延长 lease，不推进 epoch；新的 `session_id` 才推进 epoch。`DeviceEpoch` 表示设备重建代际，`PlacementEpoch` 围栏 ReplicaGroup 拓扑。PlacementSnapshot 中的 `catalog_revision` 是 Meta 最近确认的本地目录下界；ReplicaAck 中的值是 finalize 后 LocalCatalog 返回的确切 revision，因此 ACK revision 可以大于快照下界，不能要求两者相等。

## 4. R1 Local Fast Path

当配置为一份本地同步副本时：

1. 从缓存 PlacementSnapshot 取得本地 `ReplicaTarget`；
2. 写 staging 文件；
3. fsync 内容；
4. 原子 finalize 为 ChunkId；
5. fsync Chunk 目录；
6. 重读并校验 digest；
7. 产生一个 `ReplicaAck` 和 `ChunkReceipt`。

该路径没有 Peer RPC，也不进入 RN transport adapter。生产 wiring 始终使用 `DfsChunkStore` 包装 `LocalChunkStore`，避免文件层绕过 placement 与系统配置。

## 5. RN Replication Path

当同步完成需要远端副本时：

1. `ReplicationPlan::derive` 校验配置和目标数量；
2. `ReplicaDataPlane::prepare` 在任何本地或远端持久化前检查完整传输计划是否可执行；
3. 本地目标复用 `LocalChunkStore::persist`；
4. 远端目标通过选定 data plane 接收同一不可变 Chunk；
5. `ReplicationEngine` 聚合满足同步数量的 `ReplicaAck`；
6. 返回一个聚合 `ChunkReceipt`；
7. 文件层执行一次 FileVersion CAS。

当前代码提供 RN 状态机边界与两种 transport adapter，但远端复制执行尚未实现。gRPC、RDMA adapter 和入站 `DfsChunks` 服务都在第一项副作用前返回 `UNIMPLEMENTED`，因此不会形成被误认成 durable 的半副本。

## 6. gRPC 与 RDMA

`ReplicaDataPlane` 表达逻辑 `put_remote_replicas`，不承诺流语义：

- gRPC：`PutReplicaStream` 使用 header + data frames，适合 pipeline 和 backpressure；
- RDMA：`PutReplicaRdma` 携带已协商 MR 的 session、offset 和 length，由接收方执行单边搬运；
- 两条路径最终都必须在目标端完成 digest 校验、fsync、finalize 和 catalog 更新，才返回相同语义的 `ReplicaAck`；
- RDMA CQ completion 只证明 DMA 完成，不等于 durable replica。

业务状态机只观察 `ReplicaAck`，不观察 gRPC frame 或 RDMA descriptor。

## 7. 完成与可见性

协议区分两个完成点：

- `ReplicationSatisfied`：物理副本达到本次同步门槛；
- `MetaCommitted`：Meta 接受 receipt，并在同一事务中持久化 Chunk、Copy、Placement、可选 ReplicationTask、LayoutRoot、FileVersion 和新的 inode head。

前者不使候选 FileVersion 对 committed read 可见。Meta CAS 失败时，已 finalize 的 Chunk 是 orphan，可供幂等重试、扫描接管或 GC。

## 8. ReplicaAck、ChunkReceipt 与 CopyRecord

```text
ReplicaAck {                         ChunkReceipt {
  operation_id                         operation_id
  chunk_id                             chunk
  placement_revision                   placement_revision
  placement_epoch                      placement_epoch
  node_id / node_epoch                 replica_group_id
  device_id / device_epoch             durable_acks[]
  catalog_revision                   }
  persisted_bytes
  verified_digest
}
```

`ReplicaAck` 是 wire 上的本次物理完成证明。`ChunkReceipt` 是 Node 交给 Meta 的聚合证明。`CopyRecord` 是 Meta 校验 ACK、当前 Node session、设备代际和故障域后建立的长期目录事实。三者不能合并。

## 9. Meta 原子提交

Meta 提交 FileVersion 时必须同时：

1. 校验 WriteLease、expected inode revision 和 expected head；
2. 校验每个 ACK 与 Chunk、operation、placement、NodeEpoch、DeviceEpoch、长度和 digest 一致，并要求 ACK catalog revision 不小于 PlacementSnapshot 中对应设备的已知下界；
3. 校验 distinct node、failure domain 和 local-copy 约束；
4. 写入 `ChunkObject`、`CopyRecord` 和 `PlacementRecord`；
5. 欠副本时写入 `ReplicationTask(Pending)`；
6. 写入 LayoutRoot、FileVersion 和新的 inode head；
7. 记录 OperationOutcome。

任一条件失败都不能推进 inode head。

## 10. 异步补副本

当 `sync_required_copies < desired_copies` 时，同步调用只承诺已确认的副本数。成功 commit 后，Placement 状态为 `UnderReplicated`，同事务内的 `ReplicationTask` 负责补到 desired 数量。

- 后台失败且仍有有效源：读取继续，任务重试并告警；
- 没有有效源：任务进入 `BlockedNoSource`，读取返回 EIO；
- 后台失败不会撤销已成功的 fsync；
- `ReplicationTask` 只是待办工作，不能当成副本 ACK。

后台 worker、目标重选和 repair 执行留待后续实现。

## 11. Read、Seed 与 Repair

1. 读取先固定 FileVersionId 和 ChunkId。
2. 只有 Meta 接受的 `DurableReplica` 才计入当前可靠性。
3. staging/receiving 数据、digest 不匹配副本和旧 epoch 副本不可读、不可 seed。
4. UnderReplicated 不阻止从健康 Copy 读取。
5. Repair 目标完成本地持久化并由 Meta 登记后才计入可靠性。
6. 同一个 immutable ChunkId 的健康 Copy 内容相同，不需要向 Tail 查询 committed version。

## 12. 幂等、错误与重试

- 相同 OperationId 与 ChunkId 的重试必须返回原 ACK 或继续未完成步骤；
- OperationId 被用于不同 ChunkId 时必须拒绝；
- FileVersion CAS 响应丢失时按 OperationId 查询原 OperationOutcome；
- 多 Chunk CommitBatch 中部分 Chunk 完成不能推进 inode head；
- stale placement、node 或 device epoch 必须刷新快照后重试；
- 结果未知的写不能换 transport 盲目重放。

## 13. RPC 预算

- R1 首次写可能需要 1 次 `GetPlacementSnapshot`，后续使用缓存；每个 Chunk 无 Peer RPC；一个 CommitBatch 只有 1 次 FileVersion CAS。
- RN 的数据 Hop 数取决于 topology；gRPC 可以在一条流中 pipeline frame，RDMA 使用已协商连接和内存窗口。
- PlacementSnapshot 按 revision 缓存或 watch，不按 Chunk 查询。
- P2P 连接由公共连接池按 Node 管理，ReplicationEngine 不为每个 Chunk 新建连接。

## 14. 模块合同

### Meta

- `DfsService`：初始化不可变 ReplicationConfig、生成 PlacementSnapshot、校验 ChunkReceipt、原子提交文件版本；
- `MetaStore`：持久化 ReplicationConfig、CopyRecord、PlacementRecord 和 ReplicationTask；
- `meta.proto`：Node 注册设备、placement 查询和聚合 receipt wire 合同。

### Node

- `DfsChunkStore`：R1/RN 分叉；
- `LocalChunkStore`：单设备不可变 Chunk 持久化；
- `ReplicationEngine/ReplicationPlan`：RN 协调；
- `PlacementProvider`：缓存与刷新 Meta placement；
- `ReplicaDataPlane`：隔离 gRPC stream 与 RDMA one-sided 搬运；
- `DfsChunks`：远端副本入站 RPC 边界。

### 文件层

- `CommitBatch` 生成 `StagedChunk`；
- 只消费 `ChunkReceipt`；
- 所有 Chunk 达到同步门槛后执行一次 FileVersion CAS。

## 15. 当前实现边界

已实现：

- 文件系统级 ReplicationConfig 初始化与不可变校验；
- Node 设备注册、PlacementSnapshot RPC 与缓存；
- R1 Local Fast Path；
- ReplicationPlan、ReplicaAck、ChunkReceipt、CopyRecord、PlacementRecord、ReplicationTask 类型和 Meta 原子提交；
- gRPC/RDMA transport 分层和 fail-fast RPC 骨架。

未实现：

- RN 远端内容搬运与目标端持久化；
- placement 对多节点和健康信息的完整选择；
- stale placement 自动刷新重试；
- 异步补副本、repair、rebalance worker；
- Copy read/seed 选择与管理 API。

## 16. 验收标准

- R1 没有 Peer RPC，FileVersion 成功后能从本地 immutable Chunk 重读；
- N=2、3、4 复用相同 ReplicationConfig、Plan、ACK、Receipt 和 Meta 提交类型；
- RN 未实现期间在任何 replica side effect 前失败；
- 完成远端实现后覆盖 sync-all、async-fill、stale epoch、response lost、多 Chunk 部分完成和节点退出故障注入；
- Placement 缓存命中时每 Chunk 没有 Meta 路由 RPC；
- N=2、3、4 分别报告延迟、吞吐、Data Hop、网络放大和 Repair 影响。
