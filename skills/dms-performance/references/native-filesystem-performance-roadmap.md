# Native Filesystem 性能 Roadmap 使用规则

开始任何 Native Filesystem 性能修改前，先读：

1. [`docs/performance/native-filesystem-performance-roadmap.md`](../../../docs/performance/native-filesystem-performance-roadmap.md)
2. [`docs/performance/native-filesystem-vs-moosefs.md`](../../../docs/performance/native-filesystem-vs-moosefs.md)
3. [`evidence/native-vs-moosefs/latest/result.json`](../../../evidence/native-vs-moosefs/latest/result.json)

## 固定顺序

1. 小文件稳定热路径 0 Meta/Peer 合同已在 commit `095b8ca` 完成双轮门禁。
2. 下一项只收敛 namespace/mutation 固定成本。
3. 写路径先建立等语义 write-through lane，不把 buffered write 的优势误判成实现缺陷。
4. 再优化 Peer 首读的连续 Block pull 和副本上报。
5. 单请求控制路径收口后，才审计 Actor、gRPC、allocator 和并行复制。

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
