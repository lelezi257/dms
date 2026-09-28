# 专题二：写入完成、持久化与可见性

状态：Research
实现状态：Not Implemented
专题入口：[架构设计专题](design-topics.md)
数据模型：[RFC-0002](../rfcs/0002-file-version-chunk-model.md)

## 目标

为 POSIX、Native SDK 和 Runtime Snapshot 建立明确的完成级别，避免将客户端接收、Chunk Finalize、副本策略、FileVersion CAS、介质持久化和业务发布混为一个“写成功”。

## 已确定边界

- 普通 write 可以先进入 WriteSession；
- `O_SYNC/O_DSYNC` 和 `fsync/fdatasync` 必须完成相应的持久化合同；
- 满足策略的 ChunkReceipt 是构造 FileVersion 的前置条件；
- FileVersion CAS 是新文件布局的原子可见点；
- `fsync` 不自动 Pin、Alias 或构造 RootManifest；
- 普通 write 返回后的跨节点可见性必须通过 Writer Lease、Owner Routing、Sequencer 或逐写提交明确实现，不能留成隐含行为。

## 待定义的完成级别

```text
ACCEPTED
  WriteSession 已接受数据

LOCAL_FINALIZED
  本地 ChunkObject 已完成校验和 Finalize

POLICY_DURABLE
  当前 DurabilityPolicy 要求的副本已达到介质合同

VERSION_COMMITTED
  FileVersion 与 InodeRecord.head_version CAS 已提交

PINNED / PUBLISHED
  可选业务名称、保留或 RootManifest 已提交
```

## 必须覆盖的操作

| 操作 | 需要明确的问题 |
| --- | --- |
| `write/pwrite` | 返回 ACCEPTED 还是 VERSION_COMMITTED；延迟错误如何上报 |
| `O_SYNC/O_DSYNC` | 每次 write 需要达到哪一完成级别 |
| 内部 Buffer Flush | 是否只生成 Chunk，是否提交 FileVersion |
| FUSE `flush` | 是否只排空句柄，如何报告后台错误 |
| `fdatasync/fsync` | 数据、长度、时间、目录项和 FileVersion 分别保证什么 |
| `close` | 是否触发最后一次版本提交；明确不隐式业务 Publish |
| Snapshot/Pin | 如何固定一个或多个精确 FileVersion |
| Alias/RootManifest | 如何提供稳定名称和多文件一致视图 |

## 代表性 Case

1. 新文件写入 100 KiB：Chunk 已 Finalize，但 Head 尚未切换。
2. 已有 1 GiB 文件中间覆盖 4 KiB：长度不变，Extent Overlay 尚未提交。
3. Append Writer 与并发 Reader：Committed Length、EOF 与读可见性。
4. R=1、R=3 和 Local+AsyncReplica 的返回差异。
5. `fsync` 成功后 Alias 创建失败：文件版本存在，业务名称不存在。

## 预期设计产物

- 操作 × DurabilityPolicy × 完成级别语义矩阵；
- WriteSession、Lease、全局读可见性和错误上报合同；
- FUSE、SDK、Runtime API 的 Completion 映射；
- 介质持久化和故障域定义；
- 对应 Draft RFC 与验收 Case。
