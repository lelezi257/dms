# DMS Native Filesystem 性能优化 Roadmap

> 状态：**P3 已完成，P4 是下一项**。本 Roadmap 从三 VM 白盒基线出发，目标不是无限追逐更高数字，而是先消除实现放大、兑现架构优势，再为可靠性和 Agent Workspace 建立持续门禁。

GitHub 跟踪入口：[性能 Roadmap #32](https://github.com/lelezi257/dms/issues/32)。源码中的本文是稳定决策正文，Issue 只维护执行状态和阶段证据。

## 1. 为什么需要单独的性能 Roadmap

[产品 Roadmap](https://github.com/lelezi257/dms/issues/23)回答“交付哪些能力”；本文回答“每条关键路径为什么慢、先改什么、达到什么条件就停止”。两者不能混为一张实现清单。

当前事实来自 [Native Filesystem 与 MooseFS 三 VM 基线](native-filesystem-vs-moosefs.md)：

- 512 MiB 本地热读为 1215.49 MiB/s，高于 MooseFS 的 803.96 MiB/s。
- 512 MiB 跨节点接管后的复读比值为 0.969，已达到持平门槛。
- P1 已让小文件本地稳定热读和 Peer 复读恢复 0 前台 Meta/Peer RPC；两次独立运行的 p50 比值分别为 0.741/0.716 与 0.724/0.718。
- 512 MiB Peer 首读出现逐 Block `PullBlock` 和 `ReportReplicas` 放大。
- 当前大文件写比较并非完全等语义：DMS 每个 FUSE write callback 都完成 write-through；对端可以先进入客户端写缓存，最后再 `fsync`。

因此，性能优化的主线不是重写数据路径，而是按白盒证据依次收掉控制路径放大；任何新优化都必须说明它属于架构成本、实现成本还是语义差异。

## 2. 永久保留的判断框架

### 2.1 三类成本必须分开

| 类型 | 定义 | 当前例子 | 处理原则 |
| --- | --- | --- | --- |
| 架构固有成本 | 近计算分布式系统完成该语义不可消除的步骤 | Peer 首读解析 Exact Version，并从 owner Node 拉取缺失 Block | 保留正确性，只减少重复往返和串行等待 |
| 实现放大 | 业务语义不要求，但当前实现重复执行的动作 | 普通读逐文件 GetXattr；未持锁文件 close 仍 ReleaseLockOwner | 优先删除，并用 RPC 计数门禁防回归 |
| 语义不对等 | 两端返回成功时提供的保证不同 | DMS write-through 对比对端 buffered write | 先增加等语义 lane，不能直接据此改架构 |

### 2.2 每轮都按同一顺序判断

1. 先确认两端输入、拓扑、介质和完成语义是否相同。
2. 再画出一次用户操作的 RPC、状态 owner、完整字节复制和等待屏障。
3. 用 Metrics/Trace/系统采样确认每一段真实耗时，不能只从总时延猜测。
4. 先减少不必要的次数，再优化单次 RPC、内存分配或字节复制。
5. 只有 profile 证明某个原语进入主要预算，才引入 allocator、并行复制或新 provider。

### 2.3 不允许无限优化

- 一个阶段只解决该阶段列出的主因；达到退出门槛后停止，不为了额外几个百分点扩大架构修改。
- 预估收益低于 5%，且不影响尾时延或资源上限的候选项，不进入当前阶段。
- 成功热路径的日志、Metrics、Trace 只有被 profile 证明占用超过 2% CPU，才允许继续削减。
- RDMA/UB 只在真实硬件和对等 TCP 基线存在后进入实现；当前只保持 provider 边界。
- 每轮优化必须同时通过功能、故障、一致性和资源门禁；不能用缓存失效、少做校验或异步丢义务换数字。

## 3. 基准与统计合同

### 3.1 三条 lane

| Lane | 用途 | 是否作为当前性能结论 |
| --- | --- | --- |
| memory fairness | DMS 与 MooseFS 的 metadata/payload 均驻内存，隔离磁盘差异 | 是，当前主要比较 |
| deployment reality | MooseFS 使用 VM 虚拟磁盘，DMS 保持内存系统 | 只展示部署效果，不宣称可靠性等价 |
| durability equivalent | 双方按相同副本数、持久屏障和 `fsync` 语义完成 | M2 建立后才作为可靠产品门禁 |

### 3.2 当前固定工作负载

- Agent workspace：create、local hot、stat、peer first、peer repeat、patch、create/delete。
- 大对象：512 MiB write、local read、peer first、peer repeat。
- 文件范围保持从 4 KiB 小文件到最大 512 MiB；1 GiB 只作为数据路径压力补充，不替代 Agent workload。
- memory lane 至少 5 轮，后端顺序交替；发布判断要求同一 commit 完成两次独立全量运行。

### 3.3 统计边界

- 小文件以 p50、p95 和 RPC 次数为门禁；样本量足够时才报告 p99。
- 512 MiB 当前每轮只有 1 个样本、5 轮共 5 个样本，p99 只记录，不作产品承诺。
- 所有比值使用同一轮配对样本汇总；不能拿历史最佳值与本轮对端比较。
- 主机 CPU、上下文切换、RSS、网络字节和关键 RPC 总耗时必须随端到端结果保留。

## 4. 总体顺序

| 阶段 | 目标 | 优先级 | 产品关系 |
| --- | --- | --- | --- |
| P0 | 冻结可复现基线与判定器 | 已完成 | 所有阶段基础 |
| P1 | 恢复小文件稳定热路径 0 Meta/Peer 合同 | 已完成 | Preview 热读阻塞项已解除 |
| P2 | 收敛 namespace 与 mutation 固定成本 | 已完成 | Agent 小文件体验 |
| P3 | 分离并优化 write-through 写路径 | 已完成 | POSIX 写与同步语义 |
| P4 | 收敛跨节点首次读取控制放大 | 立即执行 | P2P 架构优势 |
| P5 | 验证并发、Actor、内存与背压上限 | P4 后 | 单机/多客户端扩展性 |
| P6 | 建立可靠性性能包络 | M2 实现时 | 多副本、持久层、真实 fsync |
| P7 | 建立 Workspace/Snapshot/Lazy-load 业务门禁 | M3 实现时 | Agent Workspace |
| P8 | 把已通过门槛变成持续回归 | 贯穿执行 | 发布与长期维护 |

P1～P5 串行执行。只有上一阶段的根因、代码和机器证据收口后，才启动下一阶段；P8 只固化已经证明稳定的门槛。

### GitHub 阶段状态

- [x] P0：冻结可复现基线与判定器（#30 / PR #31）。
- [x] P1：恢复小文件稳定热路径 0 Meta/Peer 合同（commit `095b8ca`，双轮 evaluator `PASS`）。
- [x] P2：收敛 namespace 与 mutation 固定成本（#35；双轮 evaluator `PASS`）。
- [x] P3：建立等语义 write-through lane 并优化单次提交（双轮 evaluator `PASS`）。
- [ ] P4：收敛跨节点首次读取的 Pull/Report 控制放大；这是下一项且唯一激活项。
- [ ] P5：验证并发、Actor、内存、gRPC 与背压上限。
- [ ] P6：随 M2 建立可靠性性能包络。
- [ ] P7：随 M3 建立 Workspace/Snapshot/Lazy-load 业务门禁。
- [ ] P8：把已经通过的门槛固化为持续回归和发布停止线。

## 5. P0：基线治理

### 已完成

- 三台相同 Lima ARM64 VM，A 为 writer/data owner，B 为 remote reader，C 为 Meta。
- memory/disk lane 分离；5 轮交替配对；相同 POSIX workload。
- 端到端分位数、吞吐、资源、网络、DMS RPC 和 MooseFS operation 同时入账。
- runner、assembler、evaluator、合同、Markdown/HTML 与精简机器证据已经落盘。

### 退出门槛

- [x] 证据完整性 `PASS`。
- [x] 基线 commit、二进制哈希、VM 配置和比较语义可追溯。
- [x] Preview 结论由机器判定，不由人工挑选数字。

### 不再重复

- 不重新发明另一套小文件 workload。
- 不把 memory lane 与 disk lane 合并成单一排名。
- 不把 payload 已达标的 local read / peer repeat 当成首要优化对象。

## 6. P1：小文件稳定热路径

### 问题与证据

800 次普通文件读取分别产生 800 次 `GetFilesystemXattr` 和 800 次 `ReleaseFilesystemLockOwner`。Peer repeat 已没有 Block 传输却仍慢 5.422 倍，因此主因是控制面实现回归，不是 P2P 架构税。

### 已实施的收敛

- access ACL 随 inode grant 一次返回，并与 mode/revision 共用 BindingCache、Watch revoke 和 lease expiry；普通权限检查不再单独调用 `GetFilesystemXattr`。
- Node 只为本地镜像中真实存在或正在获取的锁 owner 访问 Meta；普通 close 的 `ReleaseFilesystemLockOwner` 在本地短路。
- FUSE entry/attr TTL 使用 Node grant 的**剩余租约**，不会在缓存命中时凭空续出新的陈旧窗口。
- Meta Watch 在 ACK 前同时撤销 inode 和精确 `(parent,name)` dentry；远端 namespace mutation 不依赖 TTL 才被看见。
- `workspace.local_hot` 明确定义为稳态热读：DMS 与 MooseFS 都在 before snapshot 前完整 warmup 一次，warmup 结果单独保存且不进入正式样本/RPC 差值。创建阶段超过租约后的首次目录恢复属于冷启动，不伪装成稳态成本。

### 允许修改

- 普通 read/open 使用有效的 inode attributes、binding/grant 与 Node cache；显式 xattr/ACL 查询才访问 xattr API。
- 只为真实获取过锁的 owner 发送 release；普通 close 不创建伪锁生命周期。
- 保留 `default_permissions`、Meta mutation authorization、Watch invalidate、lease expiry 与 generation fence。
- 增加针对 RPC 次数和断流/失效的失败回归。

### 机器门槛

- `workspace.local_hot` 与 `workspace.peer_repeat` 的前台 Meta RPC = 0、Peer RPC = 0；Heartbeat/lease renew 作为后台周期流量单独统计。
- 800 次普通读的 `GetFilesystemXattr` = 0、`ReleaseFilesystemLockOwner` = 0。
- local hot 与 peer repeat 的 DMS/MooseFS p50、p95 latency 均 <= 1.10。
- CPU、RSS 和网络字节不得出现无法解释的增长。
- 权限、ACL、真实文件锁、两 Node 写后失效、Node/Meta 重启与 stale generation 测试全部通过。

### 退出条件

两个热路径门槛连续两次独立全量运行通过，且没有通过扩大 TTL、跳过授权或关闭失效通知获得结果。

### 完成证据（2026-09-17）

| 独立运行 | Case | p50 DMS/MooseFS | p95 DMS/MooseFS | 前台 Meta RPC | 前台 Peer RPC |
| --- | --- | ---: | ---: | ---: | ---: |
| run-1 | `workspace.local_hot` | 0.741 | 0.650 | 0 | 0 |
| run-1 | `workspace.peer_repeat` | 0.716 | 0.476 | 0 | 0 |
| run-2 | `workspace.local_hot` | 0.724 | 0.585 | 0 | 0 |
| run-2 | `workspace.peer_repeat` | 0.718 | 0.499 | 0 | 0 |

- [x] 两次运行均为 5 轮 memory lane + 1 轮独立 disk 观察 lane，服务和远端目录重新部署。
- [x] `GetFilesystemXattr=0`、`ReleaseFilesystemLockOwner=0`，所有前台 RPC 为 0。
- [x] 权限/ACL/锁/revoke/lease/generation 语义保留；`dms-server` 459 项单测通过。
- [x] 机器基线：[`native-fs-hot-path-lima-aarch64-2026-09-17.json`](../../benchmarks/whitebox/baselines/native-fs-hot-path-lima-aarch64-2026-09-17.json)。
- [x] 机器合同：[`native-fs-hot-path-contract.json`](../../benchmarks/whitebox/native-fs-hot-path-contract.json)。

## 7. P2：namespace 与 mutation 固定成本

### 目标

在 P1 排除读路径噪声后，逐个审计 create、stat、patch、create/delete。目标不是让分布式权威 mutation 变成本地 memcpy，而是让一次逻辑操作只承担必要的 authority 与数据提交。

### 允许修改

- 合并同一逻辑 mutation 中重复的 inode/attrs/binding 查询。
- 对同一请求已经获得的数据在 handler 内复用，不跨一致性边界重新查询。
- 保持 `NodeState`、`MetaState` 单 owner 和单次权威版本发布；不得新增第二套 namespace 或绕过 Meta authorization。
- 若多个排队的小对象请求可被同一个协议批次承载，可在控制通道做有界批处理；不能让首个请求固定等待毫秒级窗口。

### 机器门槛

- `workspace.stat`：有效 grant 下稳定 stat 不访问 Peer；Meta 调用次数由合同固定，不能随重复次数线性增长。
- create、patch、create/delete：每次逻辑 mutation 只有一次权威 mutation/版本发布，不出现额外 xattr、伪锁 release 或重复 resolve。
- 在冻结 RPC 合同后，三项操作的 p50 至少比 P0 改善 50%；随后再以 MooseFS 比值 <= 1.25 作为 Preview 目标。若固有 Node→Meta 跳数使目标不可达，必须用分段耗时证明，并提交设计决策，而不是继续盲调。
- 稀疏写、rename、link/unlink-open、权限和重启恢复不回归。

### 退出条件

RPC 次数达到理论最小合同，剩余耗时能由一次本地 FUSE/Node 处理、必要 Meta authority 和数据提交的分段账本解释。

### 完成证据（2026-09-17）

- 稳定 `stat` 不访问 Meta 或 Peer，两次独立运行 p50 分别为 2.875 µs、2.792 µs。
- `create` 只保留一次 negative lookup、一次 namespace create 和一次随后到达的 write-through commit；无重复 inode/xattr/resolve。
- `patch` 只保留一次 write-through commit；远端真实 holder 的一次 revoke ACK 是强一致可见性屏障，无关 Node 不再收到事件或阻塞提交。
- `create/delete` 只保留 create、write-through commit、unlink 和孤儿 inode 的 reference release；普通 close 不访问 Meta。
- 三项 mutation 的同轮分段账本分别覆盖端到端 mean 的 86.6%～87.7%；剩余 12.3%～13.4% 明确归属 kernel↔FUSE 调度与 syscall 外壳，不存在未解释的重复 RPC。
- 纯延迟门槛没有伪装成达标：DMS 仍比 memory lane 对端慢约 34%～71%。P2 的结论是“固定 RPC 合同和实现放大已收敛”，gRPC/Actor 单跳效率与 write-through 屏障继续由 P3/P5 处理。
- 完整结果与源码/二进制身份见 [P2 namespace mutation 报告](native-filesystem-p2-namespace-mutation.md)；机器合同为 [`native-fs-namespace-mutation-contract.json`](../../benchmarks/whitebox/native-fs-namespace-mutation-contract.json)，精简机器基线为 [`native-fs-namespace-mutation-lima-aarch64-2026-09-17.json`](../../benchmarks/whitebox/baselines/native-fs-namespace-mutation-lima-aarch64-2026-09-17.json)。

## 8. P3：write-through 写路径

### 先纠正比较口径

当前 DMS 对每个 1 MiB FUSE write callback 都完成 write-through；512 MiB 文件因此有 512 次可见版本发布。对端可以缓存多个 write，最后一次 `fsync` 才承担稳定屏障。2560 次 `CommitFilesystemVersion` 对 5 个文件而言首先是已选择语义的成本，不应直接标记为 2560 个“多余 RPC”。

### 本阶段分两条 lane

| Lane | DMS | 对端 | 用途 |
| --- | --- | --- | --- |
| synchronous write-through | 每个 write 返回前跨 Node 可见并满足当前 Meta WAL 屏障 | 每个相同 chunk 后执行等价同步屏障 | 判断实现效率 |
| buffered product experience | 保持当前 write-through，或经单独设计启用 writeback | 普通 buffered write + 文件末 fsync | 判断产品体验，不混淆语义 |

### 允许修改

- 优化一次 commit 内的数据准备、Journal、Meta handler、ACK 和缓存回填；复用连接与请求对象。
- 在不改变“callback 返回即发布”的前提下，流水化 callback 内部可并行的准备工作。
- 调整 FUSE capability/max_write 只能基于内核协商结果，并保持一份统一配置。
- 跨 callback 合并并延迟发布属于 writeback，不得作为普通性能修复；若确有必要，单独提交 writeback 设计、崩溃语义和 dirty-budget 门禁。

### 机器门槛

- 单 callback 的 1 MiB synchronous lane 吞吐达到等语义对端的 0.90 以上。
- 更大文件若因多个 callback/commit 低于 0.90，必须以同轮账本覆盖至少 90% 端到端耗时，证明剩余是已选择的 write-through 发布边界，而不是隐藏的重复 RPC。
- 单次 1 MiB write 的分段账本覆盖 FUSE callback、Node 数据准备、Meta commit/Journaling、ACK 与缓存回填，未解释时间低于总时延 10%。
- `O_SYNC`、`O_DSYNC`、fsync/fdatasync、跨 Node 写后可见、崩溃恢复与 ENOSPC 保持正确。
- buffered lane 只记录，不在 writeback 未批准前作为阻塞门槛。

### 退出条件

1 MiB 等语义 lane 达标；更大流式写的差距由 callback/commit 数与同轮分段完整解释。若产品 lane 仍显著落后，形成是否引入可选 writeback 的明确决策，不继续用 write-through 微优化掩盖语义上限。

### 完成证据（2026-09-17）

- 无 holder 的 1 MiB 同步写两轮 DMS/MooseFS 吞吐比为 1.108、1.139，兑现本地 payload 不经过中心数据节点的优势。
- 8 MiB 被内核拆成 8 个 1 MiB callback/commit，两轮比值为 0.732、0.710；512 MiB 流式写有 512 个 callback/commit，两轮比值为 0.455、0.475。
- 真实远端 holder 每次逻辑写恰好产生一次失效 ACK；1 MiB 写额外均值为 446.6 µs、357.1 µs。
- 删除 Meta 单次 commit 内按 Extent 重复扫描全部 proof/保留版本的实现放大后，512 MiB Meta business 下降 70.8%～77.3%，DMS 吞吐提高 38.8%～95.2%。
- 两轮 512 MiB 分段覆盖 98.9% 以上；Journal 只有 1.3～1.6 ms，剩余主要是 2560 次 FUSE/Node、RPC 和 Meta commit 累计成本。
- 稳定 stat、本地读和 Peer 复读继续保持 0 前台 Meta/Peer RPC，并稳定优于同环境 MooseFS。
- 完整结论见 [P3 同步写报告](native-filesystem-p3-write-through.md)；机器合同为 [`native-fs-write-through-contract.json`](../../benchmarks/whitebox/native-fs-write-through-contract.json)，精简机器基线为 [`native-fs-write-through-lima-aarch64-2026-09-17.json`](../../benchmarks/whitebox/baselines/native-fs-write-through-lima-aarch64-2026-09-17.json)。

## 9. P4：跨节点首次读取

### 架构成本与实现放大

- 必要：B 解析固定 ObjectVersion，发现本地缺 Block，从 A 拉取并校验 bytes。
- 可优化：连续 Block 每块独立建立控制请求、前台逐块 `ReportReplicas`、重复 resolve/layout 解析和串行等待。

### 允许修改

- 一次读取计划固定 Exact Version、连续 Block 列表和 source hints。
- 小对象控制请求可有界批处理；大对象先形成读取计划，再通过一个流或少量有界并发传输 chunk。
- `ReportReplicas` 从前台关键路径移出，使用已有有界 Reporter 批量上报；延迟只影响其他 Node 何时选择该副本，不能影响当前读返回的正确性。
- 保留 Block checksum、版本 fence、错误重试、source fallback 和 Node cache 授权。

### 机器门槛

- 512 MiB 首读不再每 Block 建立独立控制 RPC；前台同步 `ReportReplicas` = 0。
- `workspace.peer_first` DMS/MooseFS p50 latency <= 1.15。
- `sequential_512m.peer_first` DMS/MooseFS throughput >= 0.90。
- peer repeat 继续 >= 0.90，local read 继续保持 P0 的优势区间，不能用首读优化破坏热读。
- 错块、缺副本、source Node 中断、重试与 stale location 测试通过。

### 退出条件

两项既有 Preview 门槛连续两次独立全量运行通过，控制 RPC 次数与 payload chunk 数解耦，剩余差距可以由一次 resolve、网络吞吐和校验成本解释。

## 10. P5：并发、Actor、内存与背压

### 为什么放在控制路径之后

当前单请求已经存在明确的多余 RPC。此时直接改 Actor、allocator 或线程数会把请求放大隐藏在更高并发里，无法判断真实收益。

### 验证范围

- 1、8、32 个客户端的 local hot、peer repeat、peer first 和小文件 mutation。
- `NodeState`/`MetaState` mailbox queue wait、handler time、外部等待、gRPC channel 复用和 backpressure。
- Region/Arena 分配、空闲合并、RSS、碎片、allocation rate 与长时稳定性。
- CPU profile、allocator profile、上下文切换和锁竞争。

### 决策门槛

- Actor queue wait p95 小于端到端 p95 的 10%；超过才允许拆出新的并发执行面，状态更新仍回单 owner。
- 8 客户端吞吐在未触及网络/内存带宽前至少达到单客户端 4 倍；若未达到，证据必须定位到 mailbox、锁、RPC 或内存带宽之一。
- 只有 allocator 占 CPU 超过 5%，或稳定 RSS/碎片违反预算，才评估 jemalloc arena/tuning；不能用更换 allocator 替代 Slot/Region 生命周期正确性。
- 512 MiB/1 GiB 并行复制只在单核 memcpy 或单 stream 明确成为主要瓶颈后实现，并设有有界线程数与小对象旁路。

### 退出条件

已找到或排除 Actor、gRPC 和 allocator 的系统瓶颈；每项调整都有单/多客户端收益与资源代价，未发现瓶颈的层不做预防性重构。

## 11. P6：可靠性性能包络

本阶段随产品 M2 进入，不用当前 memory-only 数字冒充可靠产品性能。

### 新增 lane

- replication=1/2/3 的内存副本写入与读取。
- 持久冷层关闭/开启、同步/异步落盘。
- Meta 单点与 HA/共识模式。
- 正常、单 Node 故障、重建/repair、rolling restart。

### 门槛

- 每种 durability grade 清楚记录成功返回前等待的副本和持久屏障。
- 相同 durability 下与对标系统比较；不同 durability 只展示成本曲线。
- local hot 不因后台 repair 出现前台 Meta/Peer 回归；repair 有速率限制和资源隔离。
- fsync、故障切换和恢复的性能门槛必须与数据不丢失机器验证同时通过。

## 12. P7：Agent Workspace 工作负载

本阶段随产品 M3 进入。它不重新发明底层性能路径，而是验证 Snapshot/Fork/Lazy-load 是否复用已经达标的 DataCore。

### 代表性 workload

- 以大量小文件为主、单文件不超过 512 MiB 的 create/read/patch/delete 循环。
- O(1) workspace fork，不复制全部 bytes。
- 新 Node 首次访问 lazy-load；同一 range 第二次访问不再产生 Peer payload。
- snapshot pin、rollback、TTL/GC 与并发前台 I/O。

### 门槛

- fork 延迟与总数据量解耦，bytes copied 接近 0。
- lazy-load 首次读遵守 P4 首读预算，复读遵守 P1/P4 热路径预算。
- snapshot/GC 后台工作不破坏前台 p95、缓存一致性或 Block 生命周期。

## 13. P8：持续回归与发布停止线

### 两级门禁

- 每个 PR：静态合同、单元测试、请求次数回归、合成微基准；不依赖 GitHub runner 访问本地 Lima VM。
- 阶段/发布：固定三 VM 完整 runner、两次独立 5 轮 memory lane、一次 deployment lane、机器 evaluator 与 HTML 报告。

### Baseline 规则

- Baseline 文件带 commit、环境和合同版本；只有人工 Review 后才能提升。
- 性能提升不能自动放宽正确性、资源或故障门槛。
- 环境变化先跑旧 commit 与新 commit 的 A/A 校验，再建立新 baseline。
- 回归阈值按 case 固定；不能因本轮失败临时修改 evaluator。

### Preview 性能放行线

至少满足：

- local hot <= 1.10；
- peer repeat <= 1.10；
- small-file peer first <= 1.15；
- 512 MiB peer first throughput >= 0.90；
- 512 MiB peer repeat throughput >= 0.90；
- P1 的 0 Meta/Peer 合同、P4 的无同步 ReportReplicas 合同和全部正确性/故障门禁通过。

达到上述停止线后，停止 Preview 性能专项；P5～P7 只随相应产品阶段和实测瓶颈继续，不能让“还可以更快”阻止用户试用。

## 14. 代码与证据入口

| 关注点 | 当前入口 |
| --- | --- |
| 三 VM runner | `scripts/performance/run_native_vs_moosefs_3vm.py` |
| workload | `scripts/performance/native_vs_moosefs_workload.py` |
| 汇总与白盒解释 | `scripts/performance/assemble_native_vs_moosefs_result.py` |
| 机器合同 | `benchmarks/whitebox/native-vs-moosefs-contract.json` |
| 当前机器结果 | `evidence/native-vs-moosefs/latest/result.json` |
| xattr/lock/commit Meta client | `server/src/node/metadata_client.rs` |
| FUSE read/write/release | `server/src/node/filesystem/fuse.rs` |
| write-through 业务入口 | `server/src/node/filesystem/shared.rs` |
| Peer handler | `server/src/node/peer_service.rs` |
| Peer pull 与 Node actor | `server/src/node/runtime.rs` |
| 副本批量 Reporter | `server/src/node/replica_reporter.rs` |
| RPC typed metrics | `common/metrics/src/lib.rs` |

## 15. GitHub 跟踪规则

- 本文是稳定的性能决策正文；GitHub 总 Issue 保存阶段状态与链接。
- 每次只为当前阶段创建一个实现 Issue 和一个短期分支/PR；合入 `main` 后删除分支。
- 后续阶段不提前拆成大量空 Issue；前一阶段通过后再从总 Issue 激活下一项。
- 每个阶段 Issue 必须链接本 Roadmap、基线、机器结果和退出证据。
- 产品 Roadmap #23 只挂性能 Roadmap 链接，不复制这里的白盒细节。
