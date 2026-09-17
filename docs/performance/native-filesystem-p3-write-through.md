# Native Filesystem P3：同步写优势、架构边界与推荐 Workload

> 结论：**PASS（1 MiB 吞吐达标；更大写入由 98.9% 以上同轮分段账本解释）**。本地 owner、无远端 holder、一次 FUSE callback 可承载的 1 MiB 同步写，两轮 DMS/MooseFS 吞吐比为 1.108、1.139；8 MiB 和 512 MiB 流式写会被拆成多次权威版本提交，控制面累计后反而处于劣势。稳定本地读、Peer 复读和 stat 保持优势，且前台 Meta/Peer RPC 为 0。

## 1. 先回答“本地 SHM 写是否应该越大越有优势”

只对了一半。DMS 的 payload 在 writer 所在 Node 的 Arena 中生成，不经过中心数据节点，这是架构优势；但一次 `pwrite` 返回前仍要完成：

```sequence
participant App as Application
participant Fuse as FUSE handler
participant Node as Local dms-node
participant Meta as dms-meta
participant Holder as Remote holder
App ->> Fuse: pwrite(bytes)
Fuse ->> Node: 把本次请求 bytes 写入本地 Arena
Node ->> Meta: CommitFilesystemVersion(layout, replica proof)
Meta ->> Meta: 校验、Journal、发布 Exact Version
Meta ->> Holder: 有真实 holder 时发送 invalidation
Holder -->> Meta: ACK 已撤销旧 binding
Meta -->> Node: commit success
Node -->> Fuse: write success
Fuse -->> App: pwrite returns
```

因此，优势随“**每次权威提交承载的 payload**”增大而增加，不是随“文件总大小”单调增加：

- 1 MiB 恰好由一个 FUSE write callback 承载，只付一次 Meta commit，DMS 的本地 payload 优势能够覆盖固定控制面。
- 8 MiB 被拆成 8 个 1 MiB callback，当前 write-through 合同要求 8 次版本发布；payload 变大，但控制面也同步放大。
- 512 MiB 流式写使用 512 次 `1 MiB pwrite + fdatasync`，因此每个文件有 512 次权威提交。这个 workload 更适合允许 writeback/buffering 的系统，不是当前 DMS write-through 的优势区。

本轮没有把 write-through 偷换成 writeback，也没有改变公开接口、Meta 单 owner、权限、失效或故障语义。

## 2. 公平比较合同

| 项目 | DMS | MooseFS |
| --- | --- | --- |
| 拓扑 | A=writer/owner，B=远端 holder/reader，C=Meta | A=writer/唯一 ChunkServer，B=远端 client，C=Master |
| 介质 | Node Arena 驻内存 | Master/Chunk 数据放 tmpfs |
| 调用序列 | `open -> pwrite -> fdatasync -> close` | 相同 |
| writeback | 关闭 | 每次相同 chunk 后执行同步屏障 |
| 运行 | 5 轮，后端顺序交替 | 同轮配对 |
| 独立性 | 两次全量三 VM 运行 | 相同源码和二进制身份 |

MooseFS `goal=1`，只在 A 启动 ChunkServer，避免 B 的跨节点读取偶然命中本地副本。本文只比较 memory fairness lane，不把 DMS 内存与对端磁盘部署混为可靠性结论。

## 3. 双轮结果

写入比值是 DMS/MooseFS 吞吐，越大越好；读/stat 比值是 DMS/MooseFS p50，越小越好。

| Case | run-1 | run-2 | 结论 |
| --- | ---: | ---: | --- |
| 无 holder，4 KiB 同步写 | 16.430 | 16.265 | DMS 明显快，但主要包含 MooseFS 每次同步的固定成本，不能外推为普遍规模规律 |
| 无 holder，64 KiB 同步写 | 0.879 | 0.840 | 固定控制面尚未被 payload 摊薄，DMS 略慢 |
| 无 holder，1 MiB 同步写 | 1.108 | 1.139 | DMS 稳定领先 10.8%～13.9% |
| 无 holder，8 MiB 同步写 | 0.732 | 0.710 | 8 个 callback/commit，DMS 慢 26.8%～29.0% |
| 无 holder，512 MiB 流式写 | 0.455 | 0.475 | 512 个 callback/commit，DMS 慢约 2.1～2.2 倍 |
| 稳定 stat 4 KiB | 0.331 | 0.336 | DMS 快约 3 倍 |
| 稳定本地读 4 KiB | 0.744 | 0.760 | DMS 快约 1.3 倍 |
| 稳定本地读 1 MiB | 0.851 | 0.864 | DMS 快 15.7%～17.5% |
| Peer 接管后复读 4 KiB | 0.694 | 0.701 | DMS 快约 1.4 倍 |
| Peer 接管后复读 1 MiB | 0.667 | 0.662 | DMS 快约 1.5 倍 |

这修正了“GET 大家应该持平”的预期：稳定读已经不访问中心 Meta，数据在本地 Node 或已接管 Node 上，因此 DMS 仍有明确优势；真正需要持平或承担首访税的是 P4 的 Peer 首读，不是这里的复读。

## 4. 远端 holder 的强一致成本

B 先读取并持有旧 binding 后，A 的下一次写必须等 B 收到 invalidation 并 ACK，才能向应用返回“新版本已可见”。每个样本实测恰好 1 个 `AcknowledgeNodeEvent`。

| Payload | run-1 额外均值 | run-2 额外均值 | 架构含义 |
| --- | ---: | ---: | --- |
| 4 KiB | 195.990 µs | 261.416 µs | 固定失效往返占比高 |
| 64 KiB | 286.826 µs | 492.151 µs | 仍主要是控制面 |
| 1 MiB | 446.618 µs | 357.109 µs | payload 已摊薄一部分固定成本 |
| 8 MiB | 1737.545 µs | 865.201 µs | 8 次 callback 的失效/提交累计且抖动更明显 |

这个成本只影响写返回时间和强一致可见性，不代表 bytes 经过 holder 或 Meta。当前只测试了一个远端 holder；高扇出写不能从这组数据线性外推，应该在 P5 单独测量。

## 5. 本轮修掉的是实现放大，不是架构步骤

旧实现校验一个不断增长的 immutable 文件布局时，对 candidate 的每个 Extent 都重新线性扫描 replica proofs 和全部保留版本。单次 commit 内形成嵌套扫描，文件越大，Meta handler 的重复目录查询越严重。

现在每个 actor turn 只建立三份临时只读索引：保留 Block 集合、按 Block 分组的 replica proof、按 Block 分组的新 replica report。它们不跨请求缓存，不成为第二份权威状态；`versions`/`replicas` 仍是唯一真相，校验与失败语义不变。

| 两轮 512 MiB 对比 | 优化前 | 优化后 | 改善 |
| --- | ---: | ---: | ---: |
| Meta business 总耗时 | 10.96～11.21 s | 2.54～3.20 s | 降低 70.8%～77.3% |
| pwrite 总耗时 | 15.89～16.47 s | 8.31～11.27 s | 降低 29.1%～49.5% |
| DMS 吞吐 | 153.2～158.8 MiB/s | 220.3～299.1 MiB/s | 提升 38.8%～95.2% |

优化后仍要遍历本次完整 layout；连续追加很多 immutable 版本的累计成本仍会增长。这属于当前“完整 VersionLayout + 每 callback 发布”的设计边界，不应继续用局部 HashMap 掩盖。

## 6. 512 MiB 剩余成本已经分段

| run | pwrite 总耗时 | FUSE/Node | RPC 传输/编解码 | Meta business | Journal | commit 数 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| run-1 | 8314.944 ms | 3214.796 ms | 2559.342 ms | 2539.551 ms | 1.256 ms | 2560 |
| run-2 | 11266.354 ms | 4540.096 ms | 3526.338 ms | 3198.290 ms | 1.630 ms | 2560 |

两轮覆盖率均为 100%（机器 evaluator 按每个大对象 Case 计算为 98.9% 以上）。Journal 不是瓶颈；剩余主要是 2560 个 FUSE callback 对应的 Node 处理、gRPC 往返/编解码和 Meta 业务。删除其中任何一次权威 commit 都会改变 write-through 语义，因此 P3 在这里停止，不继续做无边界微调。

## 7. 推荐场景和天然劣势

### 推荐

- Agent workspace 的小/中型文件，writer 与数据 owner 同机，远端活跃 holder 很少。
- 每次写完成后立即需要跨进程可见，单次写通常不超过当前 1 MiB FUSE callback。
- 写后在本地反复读取，或 Peer 首次接管后在同一 Node 反复读取。
- 大量稳定 stat/read，能够复用 Node 的授权缓存和本地/已接管数据。

### 条件适用

- 4 KiB 且每次都要求同步屏障时，本轮 DMS 明显优于 MooseFS，但收益包含对端的固定同步成本，不代表一般 KV 写结论。
- 64 KiB 同步写仍略慢，说明 payload 尚不足以摊薄一次 Meta authority；如果业务以此尺寸为主，需要结合实际并发与批量特征再判断。
- 存在一个远端 holder 时，1 MiB 在一轮接近持平、一轮略慢；业务应接受强一致 invalidation 的固定往返。

### 当前不推荐

- 把 8 MiB～512 MiB 文件拆成很多 1 MiB callback，并要求每个 callback 都同步发布新版本的顺序流式写。
- 高频多写共享、远端 holder 很多的热点文件；强一致 revoke/ACK 会把写延迟绑定到最慢 holder。
- 只追求最终吞吐、允许长时间 writeback 的批量导入。此类 workload 天然偏好合并发布，不能用当前 write-through 路径与其硬拼。

### 为什么

DMS 的天然优势是“数据留在计算节点，稳定访问不经过中心数据服务”；天然劣势是“每个新 Current 仍由中心 Meta 权威发布，强一致共享写还要撤销远端旧 binding”。推荐 workload 应让本地数据复用次数高于权威版本切换次数。

## 8. 代码与机器证据导读

| 内容 | 位置 |
| --- | --- |
| Meta 单次 commit 的临时索引优化 | `server/src/meta/runtime.rs:3803` |
| 三 VM runner | `scripts/performance/run_native_fs_write_through_3vm.py` |
| POSIX workload 与 holder 建立 | `scripts/performance/native_fs_write_through_workload.py` |
| 结果装配与分段账本 | `scripts/performance/assemble_native_fs_write_through_result.py` |
| 机器 evaluator | `scripts/performance/evaluate_native_fs_write_through.py` |
| 性能合同 | `benchmarks/whitebox/native-fs-write-through-contract.json` |
| 精简双轮基线 | `benchmarks/whitebox/baselines/native-fs-write-through-lima-aarch64-2026-09-17.json` |

执行入口：

```text
bash scripts/performance/validate_native_fs_write_through.sh
```

源码测量身份为 `3e1d80c3588d2026b6204ae2b41aa5056a9fbd4d`。原始证据位于 `evidence/native-fs-write-through/candidate-run-1` 与 `candidate-run-2`，不作为发布包内容。

## 9. 停止线与下一步

P3 已完成：1 MiB 等语义同步写兑现本地 payload 优势，稳定热读没有回归；更大流式写的劣势已由精确 callback/commit 数和 98.9% 以上分段账本解释。是否引入 writeback 是独立产品设计，不在本轮暗改。

下一项是 P4：跨节点首次读取。它要消除连续 Block 的逐块控制 RPC 和前台 replica report 放大，同时保持 checksum、Exact Version、source fallback 与稳定复读优势。
