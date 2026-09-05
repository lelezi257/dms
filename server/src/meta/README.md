# dms-meta 源码导航

`../meta.rs` 创建 `MetaHandle`，`metadata_service.rs` 实现 generated gRPC Service，
`runtime.rs` 中唯一的 `run_meta/MetaState` 处理 Node session、Replica、Version、Operation、
Watch event、visibility ACK barrier 与 Repair。

## 当前真实调用链

```text
../meta.rs
    │
metadata_service.rs
    │  protobuf DTO
MetaHandle ─▶ mpsc<MetaCommand> ─▶ run_meta ─▶ MetaState ─▶ MetadataJournal
```

| 文件 | 当前责任 |
| --- | --- |
| `../meta.rs` | 进程组合根；绑定 health/gRPC listener、恢复 Journal、创建 MetaHandle 并进入 READY。 |
| `metadata_service.rs` | Node→Meta gRPC 协议边界；只做 generated DTO 与 MetaHandle 的转换。 |
| `runtime.rs` | 唯一元数据状态 owner；维护 Version、Replica、Operation、Session、Event、pending visibility 与 Repair。 |
| `metadata_journal.rs` | 状态机的最小可靠存储合同：append/load/snapshot/truncate/last_index。 |
| `in_memory_journal.rs` | 单中心开发模式的内存 Journal；支持 snapshot、前缀截断和单调 index。 |
| `local_wal_journal.rs` | 本地 WAL + snapshot；frame 校验、sync 后 ACK、snapshot 原子发布与恢复。 |

## MetaState 的核心状态

| 状态 | value 是什么 | 业务价值 |
| --- | --- | --- |
| `versions[key][version]` | `VersionLayout`，包含 kind、logical length、Extents、digest | 权威 Current/历史版本与 CAS 判断。 |
| `replicas[block_id]` | 多个 `{node_id,node_epoch,endpoint,checksum,length,catalog_revision}` | 告诉 Reader/Repair 从哪个存活 Node 取得整个 Block。 |
| `operations[operation_id]` | `{digest,result,visibility_cursor}` | 持久化写入幂等结果；相同 ID 重试不能产生第二次提交，也不能绕过失效 ACK。 |
| `sessions[node_id]` | `{session_id,node_epoch,endpoint,last_ack,last_heartbeat}` | fencing 旧 Node incarnation，并判断 Replica 是否可达。 |
| `events` | 可重放的 invalidation/repair 事件及单调 cursor | Watch 重连恢复与 visibility/repair 协调。 |
| `pending_commits[cursor]` | 进程内 reply、operation IDs 和尚未 ACK 的 Node 集合 | 只负责“何时回复这次调用”；重启后 reply 消失，durable Operation+Event 仍保留语义。 |
| `pending_repairs[block_id]` | repair ID、目标 Node/epoch、event cursor、TTL | 防止同一 Block 重复修复，并拒绝旧目标迟到上报；它不是用户 Operation。 |

CAS 不拆独立 Manager：唯一 `MetaState` 完成“读 Current → 校验条件 → append Journal → apply record”。
当前后端为内存 Journal 或本地 WAL；没有已接通的外部一致性存储或多 Meta 选主。持久化机制不接管业务状态机。

## visibility 与 durability 的边界

- Journal append/apply 后，Version 成为权威 Current；使用 WAL 后端时，Operation 有可恢复的持久化结果。内存 Journal 不保证进程重启后恢复。
- 但存在活跃远端 Watch 时，`visibility_cursor` 仍为 `Some(cursor)`；原调用和相同 OperationId
  的重试都必须等待这些 Node ACK，避免 Reader 继续返回旧 Current cache。
- `pending_commits` 不是第二份幂等表，只保存本进程的等待者；snapshot/WAL 恢复由保留的
  invalidation event 重建 Operation 的 visibility cursor。

S2 起可见性协调还包括旧 Current 租约与恢复时的保守等待；具体状态以 `runtime.rs` 为准，不能只把“收到某次 ACK”当作全部判据。

## Replica 与 Repair

普通本地 SET 通过一个 `CommitVersion` journal record 同时提交首次 Replica 和 Version；
`ReportReplicas` 只服务 Peer import、复制、迁移和 Repair 等独立副本生命周期。

Meta 持久化每个 Block 的 `desired_copies`。lease 过期后从 live Replica 选择 source、从 live
Session 选择 target，通过 Watch 下发定向 Repair。Target 完成 prepare→pull→activate→report
才 ACK；失败不 ACK。`node_epoch` 是唯一 Node incarnation/fencing 身份，不再维护重复的
`storage_generation`。

## Checkpoint、Retention 与恢复

- `MetaCheckpointPolicy` 只保留已经生效的 `every_records` 触发器。
- `MetaRetentionPolicy` 分别控制每 key 版本数、用户 Operation 窗口、副本上报 Operation
  窗口、Event ACK 裁剪条件和可选 Replica GC。
- 仍在 `waiting_visibility` 的用户 Operation 不会被 retention 删除。
- Meta 启动顺序固定为 `load_snapshot → restore → load_after → apply_record → READY`。

## 推荐阅读顺序

1. `../meta.rs`
2. `metadata_service.rs`
3. `runtime.rs` 的 `MetaCommand/MetaState/dispatch/register_pending/apply_record`
4. `metadata_journal.rs`
5. `in_memory_journal.rs`
6. `local_wal_journal.rs`
