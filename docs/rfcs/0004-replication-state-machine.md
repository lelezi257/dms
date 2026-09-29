# RFC-0004：Chunk 单副本与多副本状态机

状态：Accepted
目标 Milestone：M3
研究依据：[专题三](../architecture/03-replication-state-machine.md)
数据模型：[RFC-0002](0002-file-version-chunk-model.md)
写入合同：[RFC-0003](0003-write-visibility-durability.md)

## 摘要

DistributedFs 在 `ChunkStore::put` 以下提供本地单副本快速路径和多副本协调路径。副本数量由 DurabilityPolicy 的 `desired_copies` 与 `sync_required_copies` 配置。Meta 维护权威 placement 和 epoch，Node 根据缓存的 PlacementSnapshot 为具体 Chunk 生成临时 ReplicationPlan。每个副本复用同一个 LocalChunkStore 持久化原语，多副本不会建立第二套存储引擎。

Chunk 内容不可变，内容变化产生新的 ChunkId。多副本协议使用 ReplicaAck 和聚合 ChunkReceipt 证明同步策略已经满足，随后由一次 Meta CAS 发布 FileVersion。协议不引入 CRAQ 式可变 Chunk pending/committed 版本或第二轮 Chunk visibility commit。

## 1. 规范边界

```text
CommitBatch
  → StagedChunk
  → ChunkStore::put(DurabilityPolicy)
  → ChunkReceipt
  → FileVersion CAS
```

- FileVersionManager 不感知 Chain、ReplicaGroup、Repair 和副本数量。
- R=1 与 RN 只在 ChunkStore 内部分叉。
- LocalChunkStore 是所有副本共用的本地持久化原语。
- ReplicationEngine 只协调多个本地持久化实例。
- Meta CAS 是新文件版本的唯一 committed visibility point。

## 2. DurabilityPolicy

实现必须支持：

```text
DurabilityPolicy {
  policy_id
  policy_revision
  desired_copies
  sync_required_copies
  min_distinct_nodes
  min_distinct_failure_domains
  local_copy
}
```

约束：

1. `1 <= sync_required_copies <= desired_copies`。
2. 副本数量不能写死为一份或三份；N=2、3、4 使用相同协议。
3. 只有满足节点、设备和故障域约束的 ReplicaAck 才能计数。
4. `local_copy=Required` 时，本地副本必须属于同步完成集合。
5. 当 `sync_required_copies < desired_copies` 时，Meta 必须在 FileVersion CAS 的同一事务中登记未完成副本任务。
6. 策略名称可以提供 R1/R2/R3 等 preset，但 Meta 和 Node 以字段值执行和校验。

## 3. Placement 权威

Meta 是以下事实的权威来源：

- PolicyRevision；
- ReplicaGroup 成员和故障域；
- PlacementEpoch；
- NodeEpoch 和 DeviceEpoch；
- Target 服务资格和健康状态。

Node 可以缓存 PlacementSnapshot，并根据 ChunkId、Policy、调用节点和缓存快照生成临时 ReplicationPlan。Node 不能选择快照之外的 Target 或降低同步要求。

稳态 Chunk put 不得同步查询 Meta 路由。Peer 或 Meta 发现 stale epoch 后，Node 刷新快照并按同一 OperationId 重试。

## 4. R=1 Local Fast Path

当策略唯一目标为本地节点时：

1. LocalChunkStore 接收 StagedChunk；
2. 写临时数据并校验长度、摘要；
3. fsync 数据并原子 finalize；
4. fsync 必要目录元数据；
5. 返回本地 ReplicaAck；
6. ChunkStore 包装为单 ACK ChunkReceipt。

该路径不得依赖 PeerConnectionPool 或多节点 ReplicationPlan。

## 5. RN Replication Path

当策略需要多个目标时：

1. Node 从缓存 PlacementSnapshot 生成 ReplicationPlan；
2. 本地落盘和 Peer 转发可以 frame pipeline；
3. 每个 Target 使用 LocalChunkStore 完成校验、fsync 和 finalize；
4. ReplicaAck 沿现有 request/response stream 返回；
5. ReplicationEngine 聚合达到 `sync_required_copies` 且满足故障域约束的 ACK；
6. ChunkStore 返回 ChunkReceipt；
7. FileVersionManager 执行一次 Meta CAS。

Chain 只是 ReplicationEngine 的可替换传输策略。FileVersion、Extent、ChunkId 和持久布局不能包含 Chain 顺序。

## 6. 完成与可见性

协议区分：

- `PolicySatisfied`：物理副本达到同步策略；
- `MetaCommitted`：Meta 接受 ChunkReceipt 并发布 FileVersion。

PolicySatisfied 不使候选 FileVersion 对 committed read 可见。Meta CAS 最终失败时，已完成的物理 Chunk 成为可复用或可 GC 的 orphan。

## 7. ReplicaAck、ChunkReceipt 与 CopyRecord

ReplicaAck 是 Peer wire 完成证明：

```text
ReplicaAck {
  operation_id
  chunk_id
  policy_revision
  placement_epoch
  node_id / node_epoch
  device_id / device_epoch
  persisted_bytes
  verified_digest
}
```

ChunkReceipt 聚合多个 ReplicaAck：

```text
ChunkReceipt {
  operation_id
  chunk
  policy_id
  policy_revision
  placement_epoch
  durable_acks
}
```

CopyRecord 是 Meta 接受 ACK 后建立的长期副本目录事实。实现不得把 wire ACK 与持久 CopyRecord 合并成一个类型。

## 8. Fencing

- WriteLeaseEpoch 围栏 inode owner；
- PolicyRevision 围栏持久性策略；
- PlacementEpoch 围栏 ReplicaGroup 成员和拓扑；
- NodeEpoch 围栏 Node 进程或实例代际；
- DeviceEpoch 围栏磁盘重建或重新格式化；
- OperationId 提供重试、去重和 result-unknown 查询；
- ChunkId 表达内容身份，因此不增加 ChunkVersion；
- PlacementEpoch 已表达拓扑版本，因此不增加独立 ChainVersion。

## 9. 异步补副本

当 `sync_required_copies < desired_copies` 时：

1. 同步返回只承诺已经完成的同步副本数量和故障域；
2. FileVersion、当前 Copy、Placement 状态、ReplicationTask 和 OperationOutcome 必须在一个 Meta 事务中提交；
3. 后台失败但仍有有效源时，文件处于 UnderReplicated，读取继续成功，任务自动重试或换目标；
4. 唯一有效源暂时不可达时，读取可以限时重试其他 Copy、Spill 或节点恢复；
5. 没有任何有效源时，任务进入 BlockedNoSource，读取返回 EIO；
6. 后台失败不能追溯改变已经成功返回的同步操作。

POSIX 不提供已完成 fsync 的异步撤销通道。系统通过指标、告警和管理 API 暴露 desired/available copy 数和欠副本时长。

## 10. Read、Seed 与 Repair

1. 读取先固定 FileVersionId 和 ChunkId。
2. 只有当前可达的 DurableReplica、VerifiedCache 和 ExternalCommitted 可以读取。
3. Staging、Receiving、摘要不匹配或旧代际 Copy 不可读取或 seed。
4. UnderReplicated 不阻止从有效 Copy 读取。
5. Repair 目标完成本地持久化并经 Meta 登记后才计入可靠性。
6. 同一个 immutable ChunkId 的合格 Copy 内容一致，因此不向 Tail 查询 committed version。

## 11. 幂等与结果未知

- 相同 OperationId、ChunkId 和 Policy 的重试返回原 ReplicaAck 或继续未完成步骤；
- OperationId 被用于不同 ChunkId 或 Policy 时必须拒绝；
- FileVersion CAS 响应丢失时按 OperationId 查询 OperationOutcome；
- 多 Chunk CommitBatch 中部分 Chunk 完成不能推进 inode head；
- 旧 epoch ACK 可以证明物理内容存在，但不能直接满足新 placement；目标必须在新 epoch 下重新确认。

## 12. RPC 合同

1. 本地单副本关键路径没有 Peer RPC。
2. N 副本 Chain 需要 N-1 个 Data Hop；ACK 使用同一 request/response stream。
3. 一个 CommitBatch 只执行一次成功的 FileVersion CAS。
4. PlacementSnapshot 按 revision 缓存或 watch，不按 Chunk 查询。
5. PeerConnectionPool 按 Node 复用连接，业务层不维护每条 Chain 的专属连接。
6. Chunk frame 可以 multiplex、batch 和 pipeline。
7. 指标区分逻辑 put、Peer exchange、Data Hop、Meta transaction 和 retry。

## 13. 3FS CRAQ 借鉴边界

采用：

- Head 本地优先和流水复制；
- 反向 durable ACK；
- epoch fencing；
- request idempotency；
- checksum、Repair 和加入门禁。

不采用：

- 同一个 ChunkId 下的 mutable committed/pending version；
- Tail 驱动的第二轮 Chunk 内容 commit；
- 普通读取向 Tail 查询版本；
- 将 Chain 版本写入文件布局。

## 14. 故障合同

| 故障 | 合同 |
| --- | --- |
| staging/receive 失败 | 不发布 FileVersion，清理或重试 |
| Peer 在 ACK 前退出 | 不计入策略，刷新 placement 后重试 |
| PolicySatisfied 后、Meta CAS 前退出 | FileVersion 不可见，Chunk orphan/recover |
| Meta CAS 响应丢失 | 按 OperationId 查询原结果 |
| stale policy/placement/node/device epoch | Meta 或 Peer 拒绝 |
| committed Copy 损坏 | 标记 Corrupt，从健康源 Repair |
| 异步欠副本且仍有源 | 读取成功，后台重试并告警 |
| 异步唯一源永久损坏 | BlockedNoSource，读取 EIO |
| 多 Chunk 部分完成 | 不产生半个 FileVersion |

## 15. 模块合同

### Meta

- DurabilityPolicy 和 PolicyRevision；
- PlacementSnapshot、ReplicaGroup、PlacementRecord 和 PlacementEpoch；
- CopyRecord、NodeEpoch、DeviceEpoch；
- ReplicationTask 和 OperationOutcome；
- FileVersion CAS 时校验聚合 ChunkReceipt。

### Node

- ChunkStore 统一接口；
- LocalChunkStore 单节点持久化；
- ReplicationEngine、ReplicationPlan 和 ReplicaTarget；
- ReplicaAck、ChunkReceipt 和 OperationId 去重；
- PeerConnectionPool 与 Chunk frame pipeline；
- PlacementSnapshot 缓存和 stale refresh。

### 文件层

- CommitBatch 生成 StagedChunk；
- 只消费 ChunkReceipt；
- 不感知副本数量、Chain 和 Repair；
- 所有 Chunk 满足策略后执行一次 FileVersion CAS。

## 16. 拒绝的方案

### 把 R=1 强制送入完整 RN 状态机

拒绝。它使本地单副本依赖 PlacementPlan、Peer 和重配置，破坏近计算快速路径。

### 为 RN 建立独立本地存储引擎

拒绝。每个副本都需要相同的校验、fsync、finalize 和恢复原语，重复实现会产生不同磁盘语义。

### 把副本数写死为一份或三份

拒绝。副本数量和同步门槛属于策略参数，N=2、3、4 使用同一个协议。

### 直接复制 CRAQ pending/committed Chunk version

拒绝。AFS 的内容版本由 immutable ChunkId 和 FileVersion 表达，不存在同一 ChunkId 下的可变内容。

### 异步任务登记等同于副本完成

拒绝。任务只表达未来工作，不能计入当前 fsync 的故障容忍度。

## 17. 验收标准

- N=1、2、3、4 使用同一策略和 receipt 类型；
- R=1 无 Peer，RN 不改变文件层接口；
- sync-all、async-fill、stale epoch、response lost 和多 Chunk 部分完成有自动化测试；
- Head/Middle/Tail 在 receive、finalize、ACK 和 Meta CAS 前后退出有故障注入；
- UnderReplicated 和 BlockedNoSource 对读取、告警和 Repair 的行为可观测；
- Placement 缓存命中时每 Chunk 没有 Meta 路由 RPC；
- N=2、3、4 分别报告延迟、吞吐、Data Hop、网络放大和 Repair 影响。
