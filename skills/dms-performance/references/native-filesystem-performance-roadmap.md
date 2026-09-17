# Native Filesystem 性能 Roadmap 使用规则

开始任何 Native Filesystem 性能修改前，先读：

1. [`docs/performance/native-filesystem-performance-roadmap.md`](../../../docs/performance/native-filesystem-performance-roadmap.md)
2. [`docs/performance/native-filesystem-vs-moosefs.md`](../../../docs/performance/native-filesystem-vs-moosefs.md)
3. [`evidence/native-vs-moosefs/latest/result.json`](../../../evidence/native-vs-moosefs/latest/result.json)

## 固定顺序

1. 小文件稳定热路径 0 Meta/Peer 合同已完成双轮门禁。
2. namespace/mutation 固定成本与最小 RPC 合同已冻结。
3. 等语义 write-through lane 已证明 1 MiB 本地 owner 写优势，并量化多 callback 与 holder ACK 的架构边界。
4. Peer 首读连续 Block pull 和后台副本登记已经完成双轮门禁。
5. 下一项只审计并发、Actor、gRPC、allocator 和背压；不得重新调整已冻结的单请求语义。

## 禁止捷径

- 不关闭权限、锁、校验、Watch/lease 或 generation fence 换性能。
- 不把 write-through 偷换为 writeback。
- 不把 memory lane 与 disk lane 合并宣称优劣。
- 不在没有 profile 证据时替换 allocator、拆 Actor 或实现 RDMA/UB。
- 达到 Roadmap 停止线后结束当前专项，不进行无限优化。

## P1 已冻结结论

- 两次独立三 VM 运行中，`workspace.local_hot` 和 `workspace.peer_repeat` 的前台 Meta/Peer RPC 均为 0。
- 两轮 p50 比值分别为 local hot 0.741/0.724、peer repeat 0.716/0.718；p95 也全部低于 1.0。
- access ACL 复用 inode grant；无锁 release 本地短路；FUSE TTL 只取剩余 lease；远端变更仍由精确 Watch revoke 驱动。
- 稳态 case 在 before snapshot 前对双方执行同一 warmup，warmup 证据独立保存，不计入正式样本。

## P3 已冻结结论

- 相同同步 POSIX 序列下，无 holder 的 1 MiB 写两轮 DMS/MooseFS 吞吐比为 1.108、1.139。
- 当前 FUSE callback 上限为 1 MiB；8 MiB/512 MiB 文件会产生多个权威 commit，不能用文件总大小推断本地 SHM 优势。
- 真实 holder 每次逻辑写恰好等待一个 invalidation ACK；无 holder 不等待 ACK。
- 512 MiB 剩余成本由 FUSE/Node、RPC、Meta business 和 Journal 覆盖 98.9% 以上；不再用局部微优化掩盖 write-through 语义上限。
- 推荐 workload 与劣势边界见 [`docs/performance/native-filesystem-p3-write-through.md`](../../../docs/performance/native-filesystem-p3-write-through.md)。

## P4 已冻结结论

- 512 MiB Peer 首读每轮只有一次 `PullBlocks` server stream；旧逐 Block `PullBlock` 为 0。
- 副本登记在 checksum 校验并安装成功后进入有界后台 Reporter；正常读取前台同步等待 `ReportReplicas` 的次数为 0。
- 两次独立三 VM 运行中，小文件 Peer 首读 p50 比值为 0.970/0.968，512 MiB Peer 首读吞吐比为 1.161/1.142。
- local hot、peer repeat 均未回归；checksum mismatch、缺副本、source 中断和 stale location fallback 合同通过。
- 完整结论见 [`docs/performance/native-filesystem-p4-peer-first.md`](../../../docs/performance/native-filesystem-p4-peer-first.md)。
- P5 只有在并发证据证明 mailbox、gRPC、allocator 或背压成为瓶颈时才允许调整相应层；不得通过拆状态 owner 或跳过校验换吞吐。
