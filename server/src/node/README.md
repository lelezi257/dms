# dms-node 源码导航

当前只有一条 Node 业务主线：Worker、Peer 和后台 Repair 都通过 `NodeHandle` 进入唯一
`run_node/NodeState`。Peer 网络传输与 Meta Watch 接收在 actor 外执行，完成后再用短命令
修改本地状态。

## 当前调用链

```text
Client ─▶ worker_service.rs ─┐
                             ├─▶ NodeHandle ─▶ mpsc<NodeCommand> ─▶ NodeState
Peer ───▶ peer_service.rs ───┘                                  │
                                                                 ├─ ArenaManager
Meta Watch ─▶ node.rs ─▶ repair/pull outside actor ──────────────┘
```

| 文件 | 当前责任 |
| --- | --- |
| `../node.rs` | 进程组合根；创建 Node、监听 UDS/TCP、注册 Service、消费 Meta Watch 并驱动 Repair。 |
| `worker_service.rs` | Client→Node 的 KV/range/batch/Hash 协议边界；只做 DTO 转换、能力校验和错误映射。 |
| `peer_service.rs` | Node→Node Probe/Pull/Prepare/Activate/Abort/Status 协议边界；与 Worker 共享同一 NodeHandle。 |
| `runtime.rs` | 唯一 Node mailbox/状态 owner；Session、KV、batch、range、Peer import、Repair 状态和失效 barrier。 |
| `arena_manager.rs` | 唯一 payload bytes owner；RegionGroup、Region、Allocation、Staging、Block、FD grant 与回收。 |
| `version_layout.rs` | 唯一无状态 `VersionLayout` 算法；校验 Extent 覆盖、随机写 overlay 和 layout digest。 |
| `kkv_operations.rs` | Node-owned KKV field-map 语义；Merge/Replace、HGet/HScan/HWriteAt 和 read-modify-CAS。 |
| `metadata_client.rs` | Node→Meta session/heartbeat/resolve/commit/batch/report/watch ACK 代理。 |

## 领域链与关键不变量

```text
ObjectVersion → VersionLayout → Extent → immutable Block
                                      └→ Arena Allocation → Region → RegionGroup
```

- `VersionLayout` 是逻辑版本的数据组成；`Extent` 只描述逻辑区间到 Block 区间的映射。
- `Block` 是已提交的不可变 bytes 身份；`Allocation` 只是 Region 内的物理位置，两者不能合并。
- `RegionGroup` 是 Region 上方的配额/安全/设备策略域；首版只有 Host-memory 默认组，但边界保留。
- Region ID 在一个 NodeConnection 生命周期内单调且不复用；Allocation ID 每次分配都唯一，拒绝旧回执误提交复用 Slot。
- SHM 只是同一 Arena bytes 的 mmap 访问方式；普通 gRPC upload 和 SHM write 最终都提交同一 Staging/Allocation。
- `ViewEpoch` 已贯穿 Client release watermark，但驱动旧 Block 物理回收仍是明确 TODO；不能把“记录了水位”误写成“已完成回收”。
- `SetRange` 只增加 patch Block，并复用 base Extent；不复制完整 value。
- KKV 在 Node 内编码为一份有版本的 field map，并复用普通对象 commit/CAS；不是 SDK 拼接多个独立 key。
- Repair 完成 prepare→pull→activate→report 后才 ACK；失败不 ACK，由 Meta Watch 重放。
- 同步 durability 当前只有 `local-memory`；更强策略明确拒绝。

## 阅读边界

`version_layout.rs` 是无状态算法，不是第二个状态 owner。有关功能、安全与回收限制，
先看 [产品边界](../../../docs/product.md)，不要仅凭目录里存在一个模块就认为生产能力完整。

## 推荐阅读顺序

1. `../node.rs`
2. `worker_service.rs`
3. `runtime.rs`
4. `arena_manager.rs`
5. `version_layout.rs`
6. `kkv_operations.rs`
7. `peer_service.rs`
8. `metadata_client.rs`
