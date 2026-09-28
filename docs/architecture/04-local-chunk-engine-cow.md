# 专题四：本地 ChunkEngine 与 COW

状态：Research  
实现状态：Not Implemented  
专题入口：[架构设计专题](design-topics.md)

## 目标

定义 Storage Service 在本地磁盘上保存 pending/committed Chunk、执行覆盖写、原子切换版本、恢复未完成写入和回收旧位置的机制。

## 两类 COW

### 可变 Chunk 的物理 COW

```text
Committed Position P1
  → allocate P2
  → copy unchanged ranges + apply write
  → persist pending record
  → satisfy replication policy
  → atomically publish ChunkId → P2
  → reclaim P1 after readers leave
```

### 不可变 Blob 的 Manifest COW

```text
Blob V1 = [C1, C2, C3, C4]
Blob V2 = [C1, C2', C3, C4]
```

未修改 Chunk 通过引用共享，不原地覆盖 sealed Blob。

## 核心未决问题

- COW 使用完整 Chunk、小 block、extent 还是混合粒度；
- append 在预留容量内能否安全原地写；
- writing log、正式映射、allocator bitmap 的提交顺序；
- 旧版本读者使用引用计数、epoch 还是 generation pin；
- 空间接近满时如何为 COW 预留双份容量；
- crash 后 pending Chunk 何时 commit、abort 或进入 repair；
- checksum、压缩和加密在哪一层计算；
- SSD、NVMe、HDD 的对齐、direct I/O、flush 和回收策略；
- spill 后本地 Position 与 external copy state 的关系。

## 预期设计产物

- 本地持久格式和 allocator 模型；
- prepare/commit/abort/recover 状态机；
- 读写并发和旧版本回收协议；
- 空间放大与写放大预算；
- 掉电故障矩阵和最小穿刺实现；
- 对应 Draft RFC。
