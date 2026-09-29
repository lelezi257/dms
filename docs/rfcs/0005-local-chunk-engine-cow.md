# RFC-0005：本地 ChunkEngine、COW 与崩溃恢复

状态：Accepted / Framework Implemented

目标 Milestone：M4

研究依据：[专题四](../architecture/04-local-chunk-engine-cow.md)

上游合同：[RFC-0002](0002-file-version-chunk-model.md) · [RFC-0003](0003-write-visibility-durability.md) · [RFC-0004](0004-replication-state-machine.md)

实现边界：per-chunk file、BLAKE3 内容身份、批量 durable finalize、LocalCatalog 恢复、reader 文件描述符 pin、dirty freeze、Layout COW 和 expected-base 校验已经接入。Pack/relocation、独立 Physical COW、orphan 与 Meta reconciliation、布局 compaction policy 和真实掉电恢复验证仍未实现。

## 摘要

DistributedFs 使用 immutable Chunk + Layout COW 表达文件修改。覆盖写产生普通的新 Chunk，新 ExtentMap 将它与旧 FileVersion 中未修改的 Chunk 组合；不增加 PatchChunk，也不在同一个 ChunkId 下维护可变内容版本。

每个 Node 使用 LocalChunkStore 将 StagedChunk 转换为 durable local copy。成功 finalize 必须完成数据持久化、no-replace 原子发布、目录持久化和 LocalChunkRecord 提交，随后才能返回 ReplicaAck。LocalChunkRecord 描述本地物理事实，Meta CopyRecord 描述全局接受的副本事实。

本地副本迁移和 Pack compaction 使用 Physical COW：保持 ChunkId 和逻辑内容不变，先创建并校验新位置，再原子切换 LocalChunkRecord，旧位置在 reader pin 释放后回收。

## 1. 规范不变量

1. `ChunkId → logical bytes` 永久不变。
2. 已 finalize 的 Chunk 禁止 overwrite 和 in-place append。
3. 文件修改生成新 Chunk 和新 FileVersion；未修改范围可以引用 expected base FileVersion 的旧 Chunk。
4. 只有本次新增 Chunk 必须提供 ChunkReceipt；旧 Chunk 复用由 Meta 从 expected base layout 验证。
5. Local finalize 完成不等于 FileVersion 可见；Meta CAS 是 committed visibility point。
6. 物理 relocation 不创建 FileVersion，也不修改 ChunkObject。
7. 未经 LocalChunkRecord 确认的字节不能 ACK、读取、seed 或计入副本数。

## 2. 对象边界

```text
ChunkObject          跨节点逻辑内容身份
StagedChunk          Node 内冻结字节 + 已确定 ChunkObject
LocalChunkRecord     Node 本地位置、编码、checksum 和设备代际
ReplicaAck           wire 上本次 durable proof
ChunkReceipt         同步副本策略的聚合证明
CopyRecord           Meta 接受后的长期全局副本目录
```

LocalChunkRecord 至少包含：

```text
chunk_id / logical_length / stored_length
physical_location
device_id / device_epoch
physical_encoding
stored_checksum
catalog_revision
created_at / deletion_state
```

`ChunkObject` 不保存 Node、Device、Position 或物理编码。

`ReplicaAck.catalog_revision` 必须使用本次 LocalChunkRecord 提交后 LocalCatalog 返回的 revision。PlacementSnapshot 中同一设备的 revision 是 Meta 已知下界；ACK 必须不小于该下界，不要求相等。

## 3. Layout COW

CommitPlanner 输入 expected base layout、冻结的 DirtyExtentMap 和目标长度，输出：

- 可以合法继承的 base Extent；
- 本次需要生成的新 Chunk 范围；
- 新 LayoutRoot；
- new chunk receipts 集合。

4 KiB patch 是普通 4 KiB Chunk。重复 patch 导致 Extent 数或 overlay 深度超过阈值时，CommitPlanner 在下一次正常 CommitBatch 中重写受影响窗口。第一阶段不运行独立的 head-changing compaction transaction。

## 4. Local finalize

规范顺序：

1. 创建 operation-scoped staging；
2. 写入完整字节并计算 digest/checksum；
3. 执行数据持久化屏障；
4. 校验内容；
5. no-replace 原子发布 final name 或 position；
6. 持久化目录或 allocator 变化；
7. 原子提交 LocalChunkRecord 和 catalog revision；
8. 返回 ReplicaAck。

相同 OperationId/ChunkId 的重试必须返回原成功或继续未完成步骤。final target 已存在时只有身份、长度和摘要全部一致才能幂等成功；禁止覆盖。

## 5. Meta commit

Meta 接收 FileVersion commit 时：

1. 校验 expected inode revision、expected head 和 WriteLease；
2. 读取 expected base FileVersion 和 LayoutRoot；
3. 将新布局中的 Chunk 分为 receipt-backed new chunks 与 base-inherited chunks；
4. 对 new chunks 执行 RFC-0004 的副本校验；
5. 校验 inherited Chunk 和 chunk range 确实可从 expected base 合法到达；
6. 原子提交新 Chunk/Copy/Placement、LayoutRoot、FileVersion、inode head 和 OperationOutcome。

Node 不提供可直接信任的 existing Chunk 白名单。未来跨文件 clone/copy-range 必须定义新的授权来源。

## 6. Physical COW

PhysicalBackend 可以从 per-chunk file 演进到 pack file。迁移流程：

```text
pin old LocalChunkRecord
  → allocate new position
  → copy or re-encode
  → durable write + verify logical digest
  → atomically replace LocalChunkRecord
  → release old pin
  → reclaim old position after all reader pins leave
```

同一时刻新的 reader 只取得一个完整 LocalChunkRecord。旧 reader 持有 position pin，因此 catalog 切换不能使正在执行的读取引用已回收空间。

## 7. 摘要与校验

- ChunkObject 使用带算法版本的强 content digest；候选为 BLAKE3-256；
- LocalChunkRecord 保存 stored checksum，检测本地编码或介质损坏；
- Pack backend 为范围读取保存小粒度 CRC32C 或等价 range checksum；
- 第一阶段只开放 Raw encoding；物理压缩或加密加入时不得改变逻辑 ChunkId。

旧无算法版本的 128-bit FNV 仅是实验原型，不是兼容格式。

## 8. 恢复

启动恢复按本地状态处理：

- partial staging：校验失败后清理；
- durable staging：按 operation journal 继续 publish 或等待超时清理；
- final bytes without catalog：扫描核对后补 catalog 或隔离，不能直接 ACK；
- catalog record without Meta CopyRecord：作为 local orphan 等待幂等重试或 reconciliation；
- Meta commit outcome 已存在：按 OperationId 返回原结果；
- deleting record：继续删除或撤销，不能供读取选择。

恢复不得把所有 writing/staging 数据无条件变成 committed Chunk。

## 9. GC 与 reconciliation

Node 周期性按 catalog revision 向 Meta 批量核对 inventory。没有 CopyRecord、活跃 Operation、repair 或 retention 引用的 Chunk 先进入 grace period，再二次确认并标记 Deleting；等待 reader pin 清空后删除物理数据和 LocalChunkRecord。

正常 FileVersion 热路径不增加 orphan registration RPC。

## 10. 物理后端

第一阶段实现 PerChunkFileBackend。未来 PackBackend 必须保持以下上层合同：

```text
finalize(StagedChunk, ReplicaTarget) -> ReplicaAck
open_verified(ChunkId) -> PinnedChunkReader
relocate(ChunkId, PhysicalTarget) -> LocalChunkRecord
mark_deleting(ChunkId) -> DeleteToken
finish_delete(DeleteToken)
recover()
```

后端差异不能泄漏进 FileVersionManager 或 ReplicationEngine。

## 11. RPC 与 I/O 预算

- 普通 write：0 Meta RPC、0 Peer RPC；
- CommitBatch：placement cache hit 时 0 路由 RPC；
- R1：每个 Chunk 只有本地 finalize，无 Peer RPC；
- RN：每个 Chunk 的数据 hop 由 RFC-0004 topology 决定；
- 一个 CommitBatch：一次 FileVersion CAS；
- orphan：后台批量 reconciliation，不增加同步提交 RPC；
- content digest 在 staging 写入时计算，正常 finalize 不完整重读 Chunk。

## 12. 被拒绝方案

- `PatchChunk` 专用类型：普通 Chunk + Extent 已能表达，新增类型会复制校验、复制与 GC 状态机；
- 同一 ChunkId 下 mutable pending/committed version：与 immutable Chunk 身份冲突；
- finalized Chunk 原地 append：掉电可能产生内容身份和字节不一致；
- `exists() + rename()` 去重：存在 TOCTOU，且 rename 可能覆盖目标；
- 每个新 FileVersion 都重写整文件：小随机写的写放大不可接受；
- 独立 compactor 直接推进 inode head：第一阶段会增加 CAS 冲突和不可解释的维护版本；
- FileVersion commit 后再发第二次 orphan registration RPC：增加热路径失败窗口，使用后台 reconciliation 处理。

## 13. 实现状态

R1 热路径框架已经接入：

- writable open 只加载 FileVersion/LayoutRoot，不加载完整 committed bytes；
- FrozenCommit 在 inode 锁内冻结写入前缀，CommitPlanner 只处理脏范围；
- 相邻脏范围合并为不超过 4 MiB 的普通 Chunk，未覆盖范围继承 expected base Extent；
- Chunk 使用带算法标识的 BLAKE3-256 内容身份；
- LocalChunkStore 使用 batch staging、数据屏障、no-replace publish、目录屏障和一次 LocalCatalog 提交；
- Node 启动先恢复 LocalCatalog，再注册真实 device epoch/catalog revision；
- `open_verified` 返回持有文件描述符的 PinnedChunkReader；
- Meta 区分 receipt-backed 新 Chunk 与 expected-base inherited Chunk。

Pack/relocation、pin 计数与删除状态机、orphan reconciliation、compaction policy 和故障注入尚未闭合，因此本 RFC 状态是 Accepted / Framework Implemented。

## 14. 验收标准

- 分块顺序写、4 KiB patch 和 compaction E2E 与专题四一致；
- 故障注入覆盖每个持久化边界并通过重启恢复；
- Meta 正确区分 new 与 inherited Chunk；
- R1/RN 共用 finalize 合同；
- 幂等 publish 永不覆盖不同内容；
- Pack relocation 在并发 range read 下不回收被 pin 的旧位置；
- 报告写放大、空间放大、recovery 时间和同步路径 RPC 数。
