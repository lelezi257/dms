# P4：Native Filesystem 跨节点首次读取收口

> 结论：**P4 已完成并通过两次独立三 VM 全量验收。** 小文件 Peer 首读延迟已经与 MooseFS 持平，512 MiB Peer 首读吞吐稳定高于 MooseFS；一次 512 MiB 读取每轮只建立一条 `PullBlocks` 流，旧 `PullBlock` 为 0，副本登记不再阻塞读返回。

## 1. P4 解决的到底是什么

跨节点首次读取有一项无法删除的架构成本：Node B 本地没有数据时，必须先解析文件绑定的 Exact Version，再从拥有对应 Block 的 Node A 拉取并校验 bytes。

P4 不删除这项正确性成本。它删除的是此前随 Block 数增长的实现放大：

- 一个 512 MiB 文件由多个 Block 组成，旧实现为每个 Block 单独调用一次 `PullBlock`。
- 每个 Block 接管完成后，读请求还会在前台逐块等待 `ReportReplicas`。
- FUSE 的连续小 read callback 可能反复进入相同的控制流程。

因此，本阶段的停止条件不是“首次读等于本地 memcpy”，而是：**一次逻辑读取只承担一次版本解析、一个有界 payload 流和必要校验；控制 RPC 数量不再跟 Block 数量线性增长。**

## 2. 修改前后时序

### 2.1 修改前

```sequence
participant F as FUSE
participant B as Node B
participant M as Meta
participant A as Node A
F ->> B: read(file, range)
B ->> M: resolve Exact Version
M -->> B: layout + replica locations
B ->> A: PullBlock(block-1)
A -->> B: block-1 bytes
B ->> M: ReportReplicas(block-1)
B ->> A: PullBlock(block-2)
A -->> B: block-2 bytes
B ->> M: ReportReplicas(block-2)
B ->> A: ... 每个 Block 重复 ...
B -->> F: bytes
```

RPC 数量与 Block 数量绑定；512 MiB / 1 MiB Block 会把一次文件读取放大成约 512 次 Peer RPC 和多次前台副本登记。

### 2.2 修改后

```sequence
participant F as FUSE
participant B as Node B
participant M as Meta
participant A as Node A
participant R as Replica Reporter
F ->> B: read(file, range)
B ->> M: resolve Exact Version + bounded directory hints
M -->> B: exact layout + replica locations
B ->> A: PullBlocks(exact plan)
A -->> B: ordered chunk stream for all missing Blocks
B ->> B: checksum validate + install verified Blocks
B -->> F: requested bytes
B ->> R: enqueue verified replica facts
R ->> M: background batched ReportReplicas
```

`ReportReplicas` 仍然存在，因为 Meta 最终要知道 Node B 已经拥有这些副本；变化是它只影响其他 Node 何时能选择 B，不再决定当前用户读取何时返回。

## 3. 关键设计

### 3.1 `PullBlocks` 是 payload 传输协议，不是新的业务状态层

- 请求固定本次计划中的 Exact Version、Block 顺序和 source hints。
- Node A 按请求顺序返回 chunk；大对象不会被塞进单个 gRPC message。
- Node B 逐 Block 完成长度、身份和 checksum 校验后才安装到 Arena。
- source 失败或位置过期时沿原有 fallback 规则重新选择，不把坏数据登记成副本。

公开 SDK、`DataCoreHandle`、`NodeState` 和 `MetaState` 的 owner 都没有变化。

### 3.2 FUSE read-ahead 只减少 callback 放大

内核可能把一个顺序读取拆成多个 FUSE `read` callback。Node 为同一个 open handle 保留有界 read-ahead window：默认窗口 8 MiB，总预算 64 MiB；后续相邻 callback 直接消费已经接管的 bytes。inode invalidation epoch 变化后，旧窗口立即失效。

它不是第二份长期文件缓存，也不会改变 Exact Version。对于 512 MiB 大文件，Node 先形成完整 layout 的一次 `PullBlocks` 计划，read-ahead 只负责把已经校验的数据连续交给 FUSE。

### 3.3 目录预取有边界

一次目录 lookup 可以附带少量 sibling binding 和短期 inode reference reservation，用于 Agent workspace 的连续小文件访问。超过 8 MiB 的对象不做目录 payload 预取，避免“先拉大文件前缀，再为完整文件另建一条流”。

## 4. 两次独立三 VM结果

环境为三台相同 Lima aarch64 VM；A 是数据 owner，B 是远端 reader，C 是 Meta。每次运行包含 5 轮 memory fairness lane 和 1 轮独立 disk 观察 lane，两个运行重新部署服务和数据。

| 指标 | 门槛 | run E | run F | 结论 |
| --- | ---: | ---: | ---: | --- |
| `workspace.peer_first` p50 DMS/MooseFS | <= 1.15 | 0.970 | 0.968 | PASS |
| 512 MiB peer-first 吞吐 DMS/MooseFS | >= 0.90 | 1.161 | 1.142 | PASS |
| `workspace.peer_repeat` p50 DMS/MooseFS | <= 1.10 | 0.717 | 0.725 | PASS |
| 512 MiB peer-repeat 吞吐 DMS/MooseFS | >= 0.90 | 1.022 | 1.019 | PASS |
| `workspace.local_hot` p50 DMS/MooseFS | <= 1.10 | 0.751 | 0.725 | PASS |

512 MiB Peer 首读的 5 轮 RPC 计数在两个运行中完全一致：

| RPC | 5 轮次数 | 含义 |
| --- | ---: | --- |
| `GetFilesystemInode` | 5 | 每轮一次 Exact Version/inode 解析 |
| `LookupFilesystemEntry` | 5 | 每轮一次 pathname lookup |
| `PullBlocks` | 5 | 每轮一条 payload stream |
| `PullBlock` | 0 | 旧逐 Block RPC 已退出正常路径 |
| `ReportReplicas` | 5 | 每轮一次后台批量登记，不在前台等待 |

## 5. 正确性与故障门禁

以下合同全部通过：

- 错误 checksum 的 Block 被拒绝，不进入本地 Arena。
- 缺少副本时返回明确错误，不伪造空数据。
- source Node 中断时可以选择其他副本继续。
- stale location 失败后可以重新解析并 fallback。
- 后台副本登记被阻塞时，已校验的当前读取仍可完成。
- 多 Block 正常读取只使用一条 `PullBlocks` stream。

## 6. 代码导读

| 关注点 | 代码入口 |
| --- | --- |
| `PullBlocks` 协议 | `protocol/proto/dms/v1/node_peer.proto:14` |
| Peer stream handler | `server/src/node/peer_service.rs:383` |
| Node 接收、校验、安装与 fallback | `server/src/node/runtime.rs:2674` |
| 后台批量副本登记 | `server/src/node/replica_reporter.rs:1` |
| FUSE read-ahead 与失效 | `server/src/node/filesystem/fuse.rs:936` |
| 目录绑定预取 | `server/src/node/filesystem/shared.rs:125` |
| Meta sibling hints | `server/src/meta/runtime.rs:2100` |
| typed RPC metrics | `common/metrics/src/lib.rs:375` |
| P4 evaluator | `scripts/performance/evaluate_native_fs_peer_first.py:1` |
| P4 合同验证 | `scripts/performance/verify_native_fs_peer_first_contracts.sh:1` |

## 7. 机器证据与复现

- 精简基线：[`native-fs-peer-first-lima-aarch64-2026-09-18.json`](../../benchmarks/whitebox/baselines/native-fs-peer-first-lima-aarch64-2026-09-18.json)
- 机器合同：[`native-fs-peer-first-contract.json`](../../benchmarks/whitebox/native-fs-peer-first-contract.json)
- 来源 commit：`93537228ced0cf92c050a62c171558238cf227c0`
- `dms-node` SHA-256：`27e731ac17b50a9dbe4a34d1ad0e4e641e60cd149ba016c2bc4126085b10e23d`
- `dms-meta` SHA-256：`dcb8ff2233d0ef53cb555da0cade7935de75832a175f75ab1ea34151700cda94`

在已经配置好的三 VM 环境中，使用 P4 runner profile 分别运行两次完整基准，再执行：

```bash
python3 scripts/performance/evaluate_native_fs_peer_first.py \
  --result /path/to/run-e/result.json \
  --result /path/to/run-f/result.json \
  --output /path/to/evaluation.json

bash scripts/performance/verify_native_fs_peer_first_contracts.sh
```

## 8. 收口结论

P4 已兑现预期：Peer 首读仍保留一次 Exact Version 解析和一次远端 payload 传输，但消除了逐 Block 控制 RPC 和前台副本登记等待。剩余路径已经能由一次 pathname/版本解析、一条数据流、checksum 和 FUSE 交付解释。

下一项进入 P5：并发客户端、Actor mailbox、gRPC、Arena/allocator 与背压上限。P5 不重新优化已经达到停止线的 local hot、peer repeat 或单请求 Peer 首读。
