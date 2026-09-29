# 专题三：单副本与多副本写入状态机

状态：Accepted Design
实现状态：副本基础框架已实现；R1 可执行；RN 远端搬运、异步 worker 与完整 placement 尚未实现
专题入口：[架构设计专题](design-topics.md)
规范合同：[RFC-0004](../rfcs/0004-replication-state-machine.md)
数据模型：[RFC-0002](../rfcs/0002-file-version-chunk-model.md)
写入边界：[RFC-0003](../rfcs/0003-write-visibility-durability.md)

## 1. 目标

本专题定义 `ChunkStore::put_batch` 以下的副本协议，使同一个 FileVersion 提交流程可以使用本地单副本、同步多副本和异步补副本。副本数量由文件系统初始化配置决定，不在协议中写死为一份或三份。

```text
CommitBatch
    │
    ▼
StagedChunk
    │
    ▼
ChunkStore::put_batch
    ├── Local Fast Path
    └── Replication Path
    │
    ▼
ChunkReceipt
    │
    ▼
FileVersion CAS
```

专题一确定的 `FileVersion → LayoutRoot/ExtentMap → ChunkObject` 不变。专题二确定的 `write → CommitBatch → ChunkStore → FileVersion CAS` 不变。R=1 与 RN 只在 Chunk 副本协调层分叉。

## 2. 非目标

- 不建立第二套多副本存储引擎；每个副本复用同一个本地 Chunk 持久化原语。
- 不把 ReplicaGroup、Chain 顺序或副本数量写入 FileVersion、Extent 或 ChunkId。
- 不在同一个 ChunkId 下维护可变的 committed/pending 内容版本。
- 不要求每个 Chunk 在写入前同步访问 Meta 获取路由。
- 不在本专题确定本地普通 patch Chunk、物理 COW、Compaction 和持久文件格式；这些由[专题四](04-local-chunk-engine-cow.md)定义。
- 不在本专题实现外部对象存储 Spill；Spill 服从同一 Copy Catalog 和发布门禁。

## 3. 核心结论

1. R=1 与 RN 在副本协调层是两条不同执行路径；在单节点落盘层复用同一个 LocalChunkStore；在 FileVersion 提交层重新汇合。
2. 整个文件系统只有一份不可在线修改的 ReplicationConfig。R1、R2、R3、R4 是不同初始化值，不是 inode 级策略。
3. Meta 维护权威 PlacementSnapshot、PlacementRevision、PlacementEpoch 和节点代际；Node 根据缓存快照为具体 Chunk 生成临时 ReplicationPlan。
4. RN 的 Chain 是 ReplicationEngine 的传输拓扑，不是文件布局或 Chunk 身份的一部分。
5. 同步副本完成只产生 ChunkReceipt；只有 Meta CAS 推进 inode head 后，新 FileVersion 才对 committed read 可见。
6. AFS 的 ChunkObject 不可变，因此不复制 3FS CRAQ 的可变 Chunk pending/committed 版本切换；保留流水转发、反向 durable ACK、epoch fencing、幂等重试和 repair gating。
7. 异步补副本失败不会追溯改变已经成功的同步调用；它产生 under-replicated 状态。存在有效 Copy 时读取继续成功，没有任何有效源时读取返回 EIO。

## 4. 两个完成边界

### 4.1 ReplicationSatisfied

一个 Chunk 的物理副本已经满足文件系统 ReplicationConfig 的同步门槛：

- 本地单副本策略：一个本地副本完成；
- 同步 N 副本策略：N 个符合故障域要求的副本完成；
- 异步补副本策略：`sync_required_copies` 份完成，补到 `desired_copies` 的任务已准备登记。

此时 Chunk 可以完整存在于多个节点，但还没有新的 committed FileVersion 引用它。

### 4.2 MetaCommitted

Meta 在一个事务中校验 ChunkReceipt，创建或引用 ChunkObject、CopyRecord、PlacementRecord、LayoutRoot 和 FileVersion，并推进 `InodeRecord.head_version`。异步策略还必须在同一事务中登记 ReplicationTask。

```text
ReplicationSatisfied
      ≠
FileVersion visible
```

只有 `MetaCommitted` 后，普通 committed read、Snapshot、Pin 和 P2P seed discovery 才能发现该文件版本及其 Copy。

## 5. 文件系统级 ReplicationConfig

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

- `1 <= sync_required_copies <= desired_copies`；
- 同一个 Node 上的多个磁盘副本不能满足 `min_distinct_nodes > 1`；
- 只有符合系统配置声明故障域的 ReplicaAck 才能计数；
- `local_copy=Required` 时，调用 Node 的本地副本必须属于同步完成集合；
- Meta 初始化文件系统时持久化唯一配置；后续启动值必须完全相同；
- 修改配置需要停止并重新初始化文件系统，不提供动态 policy revision。

下表是常见初始化值：

| 别名 | desired | sync required | local | 同步返回语义 |
| --- | ---: | ---: | --- | --- |
| `R1_LOCAL` | 1 | 1 | Required | 本地一份持久副本 |
| `R2_SYNC` | 2 | 2 | Required | 两节点同步完成 |
| `R3_SYNC` | 3 | 3 | Required | 三节点同步完成 |
| `R4_SYNC` | 4 | 4 | Required | 四节点同步完成 |
| `LOCAL_ASYNC_R3` | 3 | 1 | Required | 本地一份完成，后台补到三份 |
| `ASYNC_4_SYNC_2` | 4 | 2 | Preferred | 两份同步完成，后台补到四份 |

核心协议基于 `desired_copies/sync_required_copies`，不能出现只识别 1 或 3 的分支。当前执行能力只开放 R1；持久类型、RPC 和校验逻辑已经按一般 N/M 建模，RN 数据搬运完成后无需修改文件层。

## 6. R=1 与 RN 的分叉和复用

```text
                    CommitBatch
                         │
                    StagedChunk
                         │
              ChunkStore::put_batch
                         │
              ┌──────────┴──────────┐
              │                     │
        Local Fast Path       Replication Path
              │                     │
       local finalize       plan + local + peers
              │                     │
       one-copy receipt     aggregate receipt
              └──────────┬──────────┘
                         │
                 FileVersion CAS
```

### 6.1 Local Fast Path

当策略解析为唯一目标就是本地节点时：

```text
StagedChunk
  → write temporary bytes
  → verify length and digest
  → fsync data
  → atomic finalize
  → fsync directory
  → ReplicaAck
  → one-copy ChunkReceipt
```

该路径不生成多节点 ReplicationPlan，不访问 Peer，不依赖 PeerConnectionPool，也不等待其他副本。

### 6.2 Replication Path

当策略需要多个目标或唯一目标是远端时：

```text
StagedChunk
  → derive ReplicationPlan
  → local persistence and peer forwarding
  → collect required ReplicaAck
  → aggregate ChunkReceipt
```

RN 中每个节点仍执行同一个本地持久化原语。ReplicationEngine 负责协调多个 LocalChunkStore，不实现另一套磁盘格式。

## 7. Placement 权威与 ReplicationPlan

### 7.1 Meta 权威对象

```text
PlacementSnapshot {
  revision
  replication: ReplicationConfig
  replica_groups[] {
    replica_group_id
    placement_epoch
    targets[]
  }
}
```

Meta 决定：

- ReplicaGroup 的成员和故障域；
- PlacementRevision 与 PlacementEpoch；
- 节点、设备的当前代际和服务资格；
- 哪些 Target 可以计入同步可靠性或作为 repair source。

### 7.2 Node 临时对象

ChunkId 生成后，Node 使用缓存的 PlacementSnapshot 计算：

```text
ReplicationPlan {
  chunk_id
  placement_revision
  placement_epoch
  replica_group_id
  config
  ordered_targets
}
```

ReplicationPlan 只存在于一次 Node 写操作中，不持久化到 FileVersion。Node 可以根据 Transport 能力选择 Chain 或并行 Fan-out，但不能选择 Snapshot 之外的目标，也不能降低同步要求。

稳态每个 Chunk 不访问 Meta：

```text
cold path: Node ──fetch──> PlacementSnapshot

hot path:  ChunkId
              → derive ReplicationPlan locally
              → replicate
              → Meta validates config/epoch/acks at FileVersion CAS
```

Peer 返回 stale epoch 或 Meta 拒绝旧 epoch 时，Node 刷新 PlacementSnapshot、重新生成 ReplicationPlan 并按同一个 OperationId 重试。

## 8. 核心数据结构

### 8.1 Node：ReplicaTarget

```text
ReplicaTarget {
  node_id
  node_epoch
  data_endpoint
  device {
    device_id
    device_epoch
    catalog_revision
    failure_domain
  }
}
```

### 8.2 Wire：ReplicaAck

```text
ReplicaAck {
  operation_id
  chunk_id
  placement_revision
  placement_epoch
  node_id
  node_epoch
  device_id
  device_epoch
  catalog_revision
  persisted_bytes
  verified_digest
}
```

ReplicaAck 是 Peer 协议上的物理完成证明，不是 Meta 的长期 CopyRecord。

`ReplicaTarget.device.catalog_revision` 是生成计划时 Meta 已知的目录下界；`ReplicaAck.catalog_revision` 是目标 LocalCatalog 完成本次 finalize 后的确切 revision。Node 与 Meta 校验后者不小于前者，并用 NodeEpoch 与 DeviceEpoch 防止把其他进程或重建设备的 revision 混入当前计划；两者不要求相等。

### 8.3 Node/File commit boundary：ChunkReceipt

```text
ChunkReceipt {
  operation_id
  chunk
  placement_revision
  placement_epoch
  replica_group_id
  durable_acks: Vec<ReplicaAck>
}
```

ChunkReceipt 证明本次 `ChunkStore::put_batch` 已经满足同步策略。FileVersionManager 不感知 Chain 顺序和 Repair 过程。

### 8.4 Meta：PlacementRecord

```text
PlacementRecord {
  chunk_id
  replica_group_id
  placement_epoch
  desired_copies
  copies
  health
}
```

### 8.5 Meta：CopyRecord

```text
CopyRecord {
  copy_id
  chunk_id
  node_id
  node_epoch
  device_id
  device_epoch
  state
  persisted_bytes
  verified_digest
  catalog_revision
}
```

Meta 校验 ReplicaAck 后创建或更新 CopyRecord。ACK 可以因为 FileVersion CAS 失败而成为孤儿结果；CopyRecord 是 Meta 接受后的全局目录事实，因此两个类型不能合并。

### 8.6 Meta：ReplicationTask

```text
ReplicationTask {
  task_id
  chunk_id
  placement_epoch
  desired_copies
  existing_copies
  state
  attempt
  next_retry_unix_ms
  last_error
}
```

ReplicationTask 只用于异步补副本、Repair 和 Rebalance，不进入同步 write-all 的正常数据热路径。

## 9. 状态机

### 9.1 Node 本地物理状态

```text
Absent
  → Receiving
  → Verified
  → FinalizedLocal
```

这些状态属于 Node 和 LocalChunkStore。`FinalizedLocal` 表示本机物理数据已经原子持久化，但 Meta 可能尚未登记 CopyRecord。

### 9.2 Meta Copy Catalog 状态

```text
DurableReplica
  ├──→ Corrupt
  └──→ Deleting
```

- Staging 数据不进入 Meta，不读、不 seed、不计入可靠性；
- DurableReplica 可以读、seed，并按策略计入可靠性；
- Node 或 Device 暂时不可达由健康状态派生，不需要批量改写每个 CopyRecord。

### 9.3 单次 Chunk 写入状态

```text
Planned
  → Replicating
  → ReplicationSatisfied
  → MetaCommitted

Replicating
  ├──→ Retryable
  └──→ Aborted

ReplicationSatisfied
  └──→ Orphaned     // FileVersion CAS 最终未引用
```

### 9.4 异步任务状态

```text
Pending
  → Running
  → Completed

Running
  ├──→ RetryWaiting ──→ Running
  └──→ BlockedNoSource
```

目标节点失败、容量不足和网络超时通常进入 RetryWaiting。只有没有任何有效源时进入 BlockedNoSource。

## 10. Case 1：`/model.bin` 使用本地单副本

```text
user     write          fsync                                   ok
──────────●───────────────●──────────────────────────────────────●────>

node     dirty          freeze      chunk      local      receipt
──────────●───────────────●──────────●──────────●──────────●──────────>

meta       V7                                               V8
───────────●─────────────────────────────────────────────────●─────────>
```

1. CommitTrigger 冻结 `seq<=S` 的 dirty prefix，使 fsync 有确定等待边界。
2. ChunkBuilder 产生一个或多个 StagedChunk；ChunkId 固定内容身份。
3. Local Fast Path 写临时数据、校验、fsync 和原子 finalize。
4. Node 生成包含一个 ReplicaAck 的 ChunkReceipt。
5. Meta 一次 CAS 创建或引用 Chunk、Copy、Placement、LayoutRoot 和 FileVersion，并将 head 从 V7 推进到 V8。
6. fsync 返回。此时只承诺该本地故障域；节点或磁盘永久损坏可以使内容不可用。

RPC 预算：

```text
Peer data hop: 0
Meta transaction: 1 per CommitBatch
Placement RPC: first use 1, cache hit 0
```

## 11. Case 2：`/snapshot.img` 使用同步 N 副本

以 N=3 的一个配置实例说明，具体 N 可配置。对同一个 Chunk C28：

```text
A → B → C
```

A、B、C 是 C28 的三个副本。连续文件 Chunk 可以选择不同 ReplicaGroup：

```text
C28 : A → B → C
C29 : B → D → A
C30 : C → A → D
```

### 11.1 时间线

```text
user     fsync                                                        ok
──────────●────────────────────────────────────────────────────────────●──>

node A   plan      local/forward                              receipt
──────────●────────────●─────────────────────────────────────────●────────>

node B              receive/forward                    durable/ack
───────────────────────●────────────────────────────────────●──────────────>

node C                         receive          durable/ack
────────────────────────────────●──────────────────●──────────────────────>

meta                                                               V8
────────────────────────────────────────────────────────────────────●─────>
```

### 11.2 本地优先不是串行等待

本地优先表示 Placement 包含当前计算节点，并以本地节点作为写入入口。gRPC data plane 可以使用 frame pipeline：

```text
frame 1: A 写本地并转发 B
frame 2: A 写本地并转发 B；B 写本地并转发 C
...
EOF:     各节点校验、fsync、finalize
```

RDMA data plane 使用已协商的内存描述符和单边搬运，不具有 frame stream 语义。两者都不应退化为“先完整写完 A，再顺序复制完整文件到 B、C”。

### 11.3 ACK 聚合

```text
C durable → ACK to B
B durable + C ACK → ACK to A
A durable + downstream ACKs → ChunkReceipt
```

gRPC 的 ACK 可以由原流式请求返回；RDMA 在 DMA completion 之后通过业务确认消息返回 ACK。ACK 证明目标端完成校验、fsync、finalize 和 catalog 更新，不触发第二轮 Chunk 内容版本切换。

### 11.4 FileVersion 提交

只有 `sync_required_copies` 个有效 ACK 满足 PlacementRevision、PlacementEpoch、NodeEpoch、DeviceEpoch 和故障域约束后，Node 才能将 ChunkReceipt 放入 FileVersion CAS。CommitBatch 中任一 Chunk 未满足系统配置，整个新 FileVersion 都不能推进 head。

以 N=3 Chain 为例：

```text
Peer request/response exchanges: 2
Data hops: 2
Meta transactions: 1 per CommitBatch
Placement RPC: 0 on cache hit
```

多 Chunk 使用连接池、长连接、batch 和 pipeline；逻辑 Chunk 数不等于短连接数。

## 12. Case 2 故障分支：中间副本退出与重配置

原计划：

```text
epoch 41: A → B → C
```

B 在复制期间退出。A 或 C 上可能已有完整的 FinalizedLocal，但 Meta head 仍然是 V7，fsync 尚未成功。

Meta 将 B 移出服务集合并生成新 placement：

```text
epoch 42: A → D → C
```

旧 epoch 41 的 ACK 不能直接满足 epoch 42，因为它证明的是旧成员关系和旧故障域。物理内容仍可复用：已经持有完整 C28 的 C 可以在新 epoch 下执行 ConfirmReplica，重新校验 ChunkId、摘要、NodeEpoch 和 DeviceEpoch，返回 epoch 42 的新 ACK，避免重复传输内容。

```text
physical content valid
        ≠
current placement satisfied
```

## 13. Case 3：本地同步、后台补到 N 副本

以 `desired=3, sync_required=1` 为例：

```text
user     fsync                            ok
──────────●────────────────────────────────●────────────────────────────>

node A   local      receipt       async task          replicate B/C
──────────●───────────●──────────────●────────────────────●──────────────>

meta                 V8 + tasks                       copies complete
──────────────────────●────────────────────────────────────●─────────────>
```

Meta 提交 V8 时必须在同一事务中写入：

- FileVersion V8 和 inode head；
- 已完成的本地 DurableReplica；
- `desired_copies` 与当前 `achieved_copies`；
- 补副本 ReplicationTask；
- OperationOutcome。

这样不会产生“版本已发布但补副本任务没有登记”的空窗。

同步返回只承诺 `sync_required_copies` 已完成。后台任务登记不能替代尚未存在的副本。

### 13.1 后台复制失败但仍有有效源

- 文件读取继续成功；
- Placement 处于 UnderReplicated；
- ReplicationTask 自动重试或更换 Target；
- 指标、管理 API 和告警暴露 `desired_copies`、`available_copies` 与欠副本时长；
- 普通 read 不因副本数不足而返回错误。

### 13.2 唯一有效源暂时不可达

读取依次尝试其他 Copy、外部 Spill 和限定时间内的节点恢复。所有候选暂时不可达后返回 EIO，但不立即把内容标记为永久丢失。

### 13.3 唯一有效源永久损坏

FileVersion 和 ChunkObject 元数据仍然存在，但没有可用数据源。任务进入 BlockedNoSource，后续读取返回 EIO，不能返回空数据、旧版本或未校验数据。过去已经成功的 fsync 不被追溯改写；它当时只承诺系统配置声明的一份同步副本。

## 14. Read、Seed 与 Repair 规则

1. 读取先固定 FileVersionId 和 ChunkId。
2. 只从当前可达的 DurableReplica、已校验 VerifiedCache 或 ExternalCommitted 读取。
3. Staging、Receiving、摘要不匹配和旧 NodeEpoch Copy 不可读取或 seed。
4. 同一个不可变 ChunkId 的合格 Copy 内容相同，因此 RN 不需要向 Tail 查询 committed version。
5. UnderReplicated 不阻止从现存合格 Copy 读取，同时提高 Repair 优先级。
6. Repair source 必须是当前合格 Copy；目标完成校验和持久化并经 Meta 登记后才计入可靠性。
7. 新加入节点可以接收新 Chunk 和 Repair，但单个 Copy 未完成前不参与该 Chunk 的普通读取或同步计数。

## 15. Fencing 与幂等

| 字段 | 保护对象 | 作用 |
| --- | --- | --- |
| FileVersionId | 文件内容版本 | 固定读取视图 |
| ChunkId | Chunk 内容 | 内容变化产生新身份 |
| WriteLeaseEpoch | inode owner | 拒绝旧 owner 提交 |
| PlacementRevision | placement 快照 | 拒绝旧路由证明 |
| PlacementEpoch | 副本拓扑 | 拒绝旧成员和旧顺序提交 |
| NodeEpoch | Node 实例 | 拒绝节点重启前的 ACK |
| DeviceEpoch | 存储设备实例 | 拒绝重建或重新格式化前的 ACK |
| OperationId | 逻辑操作 | 重试、去重和 result-unknown 查询 |

不增加独立 ChunkVersion：ChunkId 已经表达内容版本。不增加独立 ChainVersion：PlacementEpoch 已经表达副本拓扑版本。

同一个 OperationId 的重试必须满足：

- 相同 ChunkId 和 placement 返回相同已知结果；
- 同一 Target 已经完成时返回缓存 ReplicaAck，不重复写入；
- OperationId 被用于不同 ChunkId 时拒绝；
- Meta CAS 响应丢失时可以查询 OperationOutcome；
- NodeEpoch 或 DeviceEpoch 变化后，旧 ACK 不再有效。

## 16. 3FS CRAQ 的借鉴边界

3FS 对稳定逻辑 Chunk 原地更新，同一 Chunk 可同时存在 committed version `v` 和 pending version `v+1`。Tail 提交后，commit 从 Tail 返回 Head，各副本将 pending 切换成 committed。该协议解决同一 Chunk 身份下的可变内容一致性。

AFS 内容变化产生新的 immutable ChunkId：

```text
old content = C27
new content = C28
```

读取先由 FileVersion 固定 C27 或 C28，因此不需要 CRAQ 的 Chunk pending/committed 版本切换和 Tail version query。

| 3FS | AFS 对应机制 |
| --- | --- |
| committed chunk version | ChunkId |
| pending chunk version | StagedChunk / FinalizedLocal |
| chain version | PlacementEpoch |
| ReliableUpdate request identity | OperationId |
| storage target generation | NodeEpoch / DeviceEpoch |
| Tail→Head commit | Tail→Head durable ACK |
| write-all/read-any | replication satisfied / fixed ChunkId read-any |
| SYNCING target | per-copy Repair/ReplicaAck/CopyRecord gating |

AFS 保留：

- 本地 Head 和流水转发；
- 反向 durable ACK；
- PlacementEpoch fencing；
- OperationId 去重；
- Repair、checksum 和加入门禁。

AFS 不采用：

- 同一 ChunkId 下的 mutable pending version；
- Tail 驱动的第二轮内容 commit；
- 普通读取向 Tail 查询版本；
- 把 Chain 版本写入文件布局。

## 17. RPC 与 Data Hop 预算

| 场景 | 关键路径 Peer exchange | Data Hop | Meta 事务 | Placement RPC |
| --- | ---: | ---: | ---: | ---: |
| 本地单副本 | 0 | 0 | 1 / CommitBatch | 0 |
| 同步 N 副本 Chain | N-1 | N-1 | 1 / CommitBatch | 缓存命中为 0 |
| desired=N, sync=1 | 0 | 0 | 1 / CommitBatch | 0 |
| 异步补到 N | 后台 N-1 | 后台 N-1 | 批量更新 Task/Copy | 按需刷新 |
| stale placement | 原操作外加一次刷新 | 取决于复用 Copy | 最终仍一次成功 CAS | 1 次刷新 |

计数规则：

- ACK 是 request/response 的响应，不单独计为新建连接 RPC；
- PeerConnectionPool 按 Node 复用连接，业务层不维护每条 Chain 的专属连接；
- Chunk frame 在长连接上 multiplex；
- 一个 CommitBatch 只执行一次成功的 FileVersion CAS；
- PlacementSnapshot 按 revision 缓存和 watch，不按 FUSE WRITE 或 Chunk 查询。

## 18. 故障矩阵

| 故障点 | FileVersion 可见性 | 处理 |
| --- | --- | --- |
| 本地 staging 失败 | 否 | 删除 staging，同步调用返回错误 |
| Peer 接收中退出 | 否 | 刷新 PlacementEpoch 后重试 |
| 多副本完成、Meta CAS 前 owner 退出 | 否 | 形成 orphan；按 OperationId 恢复或 GC |
| Meta CAS 成功、响应丢失 | 是 | 查询 OperationOutcome，返回原结果 |
| 旧 PlacementEpoch 提交 | 否 | Meta 拒绝，刷新 placement |
| 旧 NodeEpoch/DeviceEpoch ACK | 否 | 不计入策略，重新确认或复制 |
| committed 后一个副本损坏 | 是 | 标记 Corrupt，从健康 Copy Repair |
| 本地单副本永久损坏 | 元数据仍在，内容不可用 | read 返回 EIO，暴露策略故障边界 |
| Async 任务失败但仍有源 | 是且可读 | UnderReplicated、重试、换目标、告警 |
| Async 唯一源永久损坏 | 是但内容不可读 | BlockedNoSource，read 返回 EIO |
| 多 Chunk 中部分满足策略 | 否 | 不推进 head；完成 Chunk 重用或 Orphan GC |
| Repair 目标只有 staging | 旧版本继续可读 | 目标不计入可靠性和 seed |
| Repair 完成、Meta 更新丢失 | 旧 Copy 仍可读 | 按 OperationId 重放 promotion |

## 19. 模块关系

```text
┌──────────────────────────── Meta ────────────────────────────┐
│ ReplicationConfig        PlacementSnapshot / PlacementRecord │
│ CopyRecord              ReplicationTask                     │
│ FileVersion CAS         OperationOutcome                    │
└──────────────────────────────▲───────────────────────────────┘
                               │ validate receipt / publish
┌──────────────────────────── Node ────────────────────────────┐
│ CommitBatch → ChunkStore                                    │
│                  ├─ Local Fast Path → LocalChunkStore       │
│                  └─ ReplicationEngine                       │
│                       ├─ ReplicationPlan                    │
│                       ├─ PeerConnectionPool                 │
│                       └─ LocalChunkStore + remote targets   │
│ ReplicaAck → ChunkReceipt                                   │
└──────────────────────────────────────────────────────────────┘
```

保留的抽象：

- ReplicationConfig：定义文件系统级同步完成承诺，初始化后不可在线修改；
- PlacementSnapshot/PlacementEpoch：定义权威拓扑；
- ReplicationPlan：定义单次 Node 执行计划；
- ReplicaAck：Peer wire 完成证明；
- ChunkReceipt：文件提交层聚合证明；
- CopyRecord：Meta 长期副本事实；
- ReplicationTask：异步和修复工作项；
- ReplicationEngine：RN 协调；
- LocalChunkStore：所有副本共用的单节点持久化原语。

不引入的抽象：

- 可变 ChunkVersion；
- 独立 ChainVersion；
- 第二轮 Chunk visibility commit；
- FileVersion 内的副本列表；
- Meta 中的 StagedChunk；
- 每条 Chain 专属的业务连接对象。

## 20. 当前实现边界

已经实现：

- Meta 初始化并持久化唯一 ReplicationConfig，后续启动配置不一致时拒绝；
- Node 注册 storage device，Meta 返回 PlacementSnapshot，Node 缓存快照；
- DfsChunkStore 在 R1 Local Fast Path 和 RN ReplicationEngine 之间分叉；
- ReplicaTarget、ReplicationPlan、ReplicaAck、ChunkReceipt、CopyRecord、PlacementRecord 和 ReplicationTask 数据结构；
- Meta 在 FileVersion CAS 同一事务中校验 ACK 并写入副本目录及欠副本任务；
- gRPC stream 与 RDMA one-sided 两个 ReplicaDataPlane adapter，以及 DfsChunks 入站 RPC 骨架；
- RN 未实现时在任何本地或远端副本副作用前返回错误。

尚未实现：

- 多节点 placement 选择、健康信息和 watch 刷新；
- RN 远端传输、目标端 staging/finalize、幂等 replay；
- 异步补副本、Repair 和 Rebalance worker；
- Copy read/seed 选择与管理 API。

## 21. 验收标准

### 21.1 功能

- N=1、2、3、4 的同步配置使用同一个 ReplicationConfig 和 ChunkReceipt 合同；
- R=1 不访问 Peer，多副本不改变 FileVersion 和 Extent 接口；
- 同步 N 副本只有满足系统配置后才能提交 FileVersion；
- Local+Async 的 FileVersion、当前 Copy 和 ReplicationTask 在一个 Meta 事务中提交；
- 读取固定 FileVersion/ChunkId，并可从任一合格 Copy 读取。

### 21.2 故障

- Head、Middle、Tail 在 receive、finalize、ACK 和 Meta CAS 前后退出均有确定结果；
- stale PlacementRevision、PlacementEpoch、NodeEpoch 和 DeviceEpoch 被拒绝；
- response lost 按 OperationId 返回原结果；
- 多 Chunk 部分完成不能产生半个 FileVersion；
- UnderReplicated、BlockedNoSource、Corrupt 和 Repair promotion 可以自动化验证。

### 21.3 性能与可观测性

- R=1 保持无 Peer 的 Local Fast Path；
- RN 复用长连接；gRPC 使用 frame pipeline，RDMA 使用已协商 MR，不按 Chunk 建连；
- Placement 缓存命中时每个 Chunk 没有 Meta 路由 RPC；
- 指标分别统计逻辑 put、Peer exchange、Data Hop、Meta transaction、retry、under-replicated age 和 repair throughput；
- N=2、3、4 分别报告吞吐、p50/p95/p99、网络放大和本地磁盘开销。

## 22. 后续专题边界

- [专题四](04-local-chunk-engine-cow.md)：定义 LocalChunkStore 的 staging、finalize、Patch、COW、Compaction 和 crash recovery；
- [专题五](05-length-truncate-seal.md)：定义 append、truncate、EOF、Pin 和 RootManifest；
- [专题六](06-reliability-performance-path.md)：定义具体 Transport、Inline/SHM/RDMA、P2P 多源、Cache、Spill 和规模验收。
