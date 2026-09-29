# 专题五：文件长度、truncate 与稳定版本

状态：Merged

专题入口：[架构设计专题](design-topics.md)

## 结论

本专题不形成新的架构层、状态机或实现模块。有效结论已经归并到既有专题：

- [专题一](01-file-version-chunk-model.md)与 [RFC-0002](../rfcs/0002-file-version-chunk-model.md)：`FileVersion.length`、Extent 边界、隐式 Hole 和稀疏读取；
- [专题二](02-write-durability-publication.md)与 [RFC-0003](../rfcs/0003-write-visibility-durability.md)：owner 串行处理 write、append、truncate 与同步屏障，以及 visible/committed EOF；
- [专题四](04-local-chunk-engine-cow.md)与 [RFC-0005](../rfcs/0005-local-chunk-engine-cow.md)：Shrink/Grow 的 Layout COW、旧 Chunk 复用与 GC 边界。

## 不增加的抽象

- 不增加 `FileMutation` 操作日志；同一 inode 的操作由 owner 串行处理并立即归并到 `InodeWriteState`；
- 不增加 Meta AppendReservation；append offset 由 owner 原子分配；
- 不增加 LengthHint、TruncateVersion 或独立 LengthRecord；最终 length 与 LayoutRoot 由一次 FileVersion CAS 原子提交；
- 不增加 Blob、Seal、ImageFile 或 SnapshotFile；finalize 后的 Chunk、提交后的 LayoutRoot 和 FileVersion 已经不可变；
- Pin、Alias 和 RootManifest 仍是可选生命周期或业务能力，不进入普通 POSIX 写入路径。

## 实现入口

代码需要补齐 DFS `setattr(size)`、`truncate/ftruncate`、完整稀疏文件读取与对应 E2E，但复用现有 `InodeWriteState.logical_length`、`DirtyExtentMap`、CommitPlanner、FileVersion 和 LayoutRoot，不建立独立专题五模块。
