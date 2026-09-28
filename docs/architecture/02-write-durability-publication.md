# 专题二：写入完成、持久化与发布语义

状态：Research  
实现状态：Not Implemented  
专题入口：[架构设计专题](design-topics.md)

## 目标

为 POSIX、Native SDK、Blob 构建和 Runtime Publish 建立同一套完成级别，避免将客户端接收、Chunk 提交、介质持久化、文件长度同步和不可变发布混为一个“写成功”。

## 待定义的完成级别

```text
ACCEPTED
  客户端或本机 Node 已接受数据

LOCAL_COMMITTED
  本地 Chunk Store 已提交目标版本

POLICY_COMMITTED
  当前 ReplicationPolicy 要求的同步副本已提交

MEDIA_DURABLE
  要求的设备 flush/barrier 已完成

SEALED
  Blob 内容、长度和 digest 已固定

PUBLISHED
  Manifest 与可靠性证明通过门禁并全局可见
```

## 必须覆盖的操作

| 操作 | 需要明确的问题 |
| --- | --- |
| `write/pwrite` | 返回 ACCEPTED 还是 POLICY_COMMITTED；错误如何延迟上报 |
| 内部 buffer flush | 数据进入哪一级提交状态 |
| FUSE `flush` | 是否只排空句柄，是否同步文件属性 |
| `fdatasync/fsync` | 数据、长度、时间和目录项分别保证什么 |
| `close` | 是否等待后台错误；明确不隐式 publish |
| `seal` | 如何确定最终 length、digest 和 Chunk 集合 |
| `publish` | Manifest、copy state 和 Namespace 何时原子可见 |

## 代表性 Case

1. 新文件写入 100 KiB：Chunk 已提交但 Meta length 尚未更新。
2. 已有 1 GiB 文件中间覆盖 4 KiB：长度不变时 `fsync` 的新增工作。
3. Append writer 与并发 reader：committed-length 水位和 EOF。
4. R=1 本地写、R=3 同步写、Local+AsyncReplica 的返回差异。
5. Blob 收到 EOF 后 seal，seal 成功但 publish 失败。

## 预期设计产物

- 操作 × Profile × ReplicationPolicy 语义矩阵；
- 用户可见错误和未知结果合同；
- FUSE、SDK、Runtime API 的 completion 映射；
- 介质持久化和故障域定义；
- 对应 Draft RFC 与验收 Case。
