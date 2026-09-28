# 专题五：文件长度、truncate 与 Blob seal

状态：Research  
实现状态：Not Implemented  
专题入口：[架构设计专题](design-topics.md)

## 目标

定义 Chunk 数据完成与文件全局长度收敛之间的协议，并将 Mutable File 的 append/truncate 与 Immutable Blob 的最终长度、稳定切点和发布门禁连接起来。

## 需要区分的水位

```text
accepted length
local committed length
policy committed length
metadata visible length
sealed length
published length
```

这些水位在简单顺序写中可以相同，在 buffered write、并发 append、部分失败、truncate、异步复制和 seal 中可能不同。

## 核心问题

- 文件扩容后何时更新 inode length；
- overwrite 不改变长度时是否避免 Meta 写；
- append reservation 如何避免多个 writer 重叠；
- hole 和 sparse range 如何进入 manifest；
- truncate version 如何隔离旧的延迟写；
- `fsync` 使用客户端 length hint，还是查询已提交 Chunk；
- close、unlink 和开放句柄如何影响长度与回收；
- Snapshot 如何取得目录树和文件数据的一致稳定切点；
- Blob seal 如何固定最终 length、digest、Chunk 列表和 copy proof；
- seal 成功、publish 失败时如何重试或 GC。

## 预期设计产物

- Mutable File 长度和 truncate 状态机；
- append reservation 和 EOF 可见性合同；
- Snapshot stable cut 与 Blob seal 状态机；
- length hint 与 Meta/Storage 对账协议；
- seal/publish 故障矩阵；
- 对应 Draft RFC。
