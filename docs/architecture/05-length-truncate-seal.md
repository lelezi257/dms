# 专题五：文件长度、truncate 与稳定版本

状态：Research
实现状态：Not Implemented
专题入口：[架构设计专题](design-topics.md)
数据模型：[RFC-0002](../rfcs/0002-file-version-chunk-model.md)

## 目标

定义 Chunk 完成与文件全局长度收敛之间的协议，并连接普通 Append/Truncate、FileVersion 提交、Snapshot Pin 和多文件 RootManifest。

## 需要区分的水位

```text
accepted length
local finalized length
policy durable length
version committed length
pinned snapshot length
```

这些水位在简单顺序写中可以相同，在 Buffered Write、并发 Append、部分失败、Truncate 和异步复制中可能不同。

## 核心问题

- 文件扩容后何时更新逻辑 Length；
- Overwrite 不改变长度时是否避免不必要的属性写；
- Append Reservation 如何避免多个 Writer 重叠；
- Hole 和 Sparse Range 如何进入 ExtentMap；
- Truncate 如何隔离旧的延迟写和 Patch Chunk；
- `fsync` 使用 WriteSession Length，还是查询已完成 Chunk；
- Close、Unlink 和开放句柄如何影响 Length 与回收；
- Snapshot 如何取得目录树和精确 FileVersion 的一致稳定切点；
- Pin、Alias 和 RootManifest 如何固定版本而不复制单文件布局；
- Snapshot 元数据提交失败时如何重试或 GC。

## 预期设计产物

- 文件 Length、Append 和 Truncate 状态机；
- EOF 可见性与并发 Reader 合同；
- Snapshot Stable Cut、Pin 和 RootManifest 状态机；
- Length Hint 与 Meta/ChunkStore 对账协议；
- 故障矩阵和对应 Draft RFC。
