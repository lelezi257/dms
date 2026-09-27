# OwnerFs 多文件 P2P 并发优化

## 结论与适用范围

**事实：** Linux 三 VM、同场 MooseFS、200 个独立 4 KiB 文件、8 worker、12 轮完整 W2，最终 AFS/MooseFS 总耗时 p50 为 **0.780**（124.76/160.04 ms），达到本轮 ≤0.80 的目标。配套 W1 两次独立会话为 0.475/0.497，顺序 W2 为 1.080，最新二进制三 VM 功能验收 15/15、无缺口或清理错误。机器判定在[最终评估](../../../experiments/results/2026-09-27-afs-p2p-concurrency/evaluation-080.json)，原始分段在同目录的 `candidate-final.json`、`w1-latest.json`、`w2-sequential-latest.json` 和 `acceptance-latest.json`。

**边界：** 前一版本的独立 12 轮结果是 0.812；最终版本首次 0.780 的 MooseFS 参考漂移 +5.2%，在预定的 ±10% 范围内。随后相同最终二进制在新运行 ID 下两次独立 12 轮复测为 **0.765/0.780**，对照漂移仅 +0.3%/−0.2%，功能和守护项仍通过。参见[复测及架构审视](../reviews/2026-09-27-ownerfs-p2p-retest-architecture.md)。三次固定场景都低于 0.8，但不外推到不同文件大小、并发数或单大文件。

## 白盒定位与实现

初始同场六轮总量比为 2.083，首次 FUSE 并发改动后仍为 1.641。`fuser::Session::run` 的单接收线程在同步 FUSE 回调里等待远端 P2P RPC，导致独立文件本可并发的打开、读写和释放被串行化。A 端 `OwnerFiles.Open` 约 19 µs，B 端 RPC 往返约 229 µs；先优化 Home 上的文件格式或分块不能解除接收线程阻塞。

最终保留单 Home 普通文件和原 P2P 协议。在 FUSE 接收线程外用 8 个有界 worker 执行可能等待远端的 LOOKUP/OPEN 和句柄读写、FLUSH/FSYNC/RELEASE；上述 read/write/flush/fsync/release 按同一文件句柄 FIFO 顺序处理，`getattr/setattr(fh)` 尚未纳入。启用内核并行目录请求。OwnerFs 句柄操作使用每句柄锁，不跨文件共享一个长期持有的全局锁；本机只读路径和本机 LOOKUP 直接执行，避免 W2 中 A 回读反而进入队列。远端 RELEASE 作为已确认 I/O 后的清理操作有界异步发送，失败会记录日志。

阶段证据（同一脚本与 8 worker；中间候选为六轮，最终为十二轮）：

| 候选 | AFS/MooseFS 完整 W2 p50 | 判断 |
| --- | ---: | --- |
| 首次只读 OPEN 并发 | 1.641 | 只消除部分入口串行，写入仍占 149 ms |
| 远端回调并发、内核目录并发 | 0.865 | 写阶段降至 46 ms |
| 增加远端 LOOKUP 批量 RPC | 0.897 | 317 次实际成批，仍变慢，已撤销 |
| A 本机只读操作不排队 | 0.806 | A 回读阶段降至 25 ms |
| 远端 RELEASE 有界异步 | 0.787；独立十二轮 0.812 | 有收益但靠近门槛 |
| A 本机 LOOKUP 不排队，最终十二轮 | **0.780** | 完整验收通过 |

把 FUSE worker 从 8 增到 16 也在同场回归（0.806→0.841），已撤销。独立小文件没有引入 3FS 的 chunk/PioV：该机制解决大文件范围拆分与存储并行，不是此次阻塞点。

## 验证与剩余风险

**事实：** 最终版本 Linux `cargo fmt --all -- --check`、严格 Clippy、AFS 全特性测试、workspace 默认特性测试、release 构建以及三 VM 15/15 功能验收通过；W1 和顺序 W2 均在既定门槛内。阶段正确性与环境收据保存在原始 JSON。未 push、merge 或 release。

**待验证：** 远端 RELEASE 若发送失败，原 FUSE close 已返回，远端句柄可能直到 Home 会话清理/重启才释放；需要故障注入和明确回收策略。`setattr(fh)` 尚未纳入同句柄队列，Home 句柄表也尚未把请求授权与实际文件根绑定；见[代码审视](../reviews/2026-09-27-ownerfs-p2p-retest-architecture.md)。不同负载仍待验。旧实验 etcd 的 MetaStore 全量快照增长触发请求过大，说明控制面规模问题，未把它归因于 P2P 性能。完整 POSIX、VM 掉电和长稳不在本轮通过项。
