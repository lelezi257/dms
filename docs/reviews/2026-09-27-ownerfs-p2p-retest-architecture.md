# OwnerFs P2P 最终版本复测与架构审视

## 结论

**事实：** 相同最终二进制 SHA-256 `8e56128111de9791…` 在全新 Linux 三 VM 运行 ID `afs-piov-retest17/18` 中，两次独立 12 轮、200×4 KiB、8 worker 完整 W2 分别为 MooseFS 的 **0.765/0.780 倍**。W1 两份为 0.501/0.468，顺序 W2 为 1.086；最新功能验收 15/15，0 失败、0 缺口、0 清理错误。两次机器评估均为 PASS，MooseFS 参考漂移分别 +0.3%/−0.2%。加上上一轮 0.780，此固定场景已有三次 12 轮通过；不能据此承诺其他负载也低于 0.8。

**架构判断：** 无需重构 VFS、Meta、Storage 或改回分片/Blob。FUSE 负责请求调度，OwnerFs 持有文件身份与真实句柄，P2P 负责远端传输，仍符合既定分层。必须局部补齐同句柄跨操作排序与 Home 句柄授权绑定；远端释放失败后的回收需要独立生命周期设计。以上问题不需要新增通用业务层。

## 分段事实与适用场景

| 12 轮 p50 阶段 | AFS / MooseFS，复测 1 | 判断 |
| --- | ---: | --- |
| B 首读 | 32.00 / 39.22 ms | AFS 快 |
| B 重读 | 21.94 / 14.16 ms | AFS 慢 |
| B 覆盖写 | 45.91 / 39.05 ms | AFS 慢 |
| A Home 回读 | 20.25 / 63.06 ms | AFS 快约 3.1 倍 |
| 完整 W2 | 120.09 / 156.93 ms | AFS 0.765 倍 |

**推断：** 当前总量优势主要来自 Home 本机回读，远端并非所有阶段都领先。这与“多数工作留在 Home 节点”的工作负载假设一致。8 worker/独立小文件是已验证范围，不能把它解释为通用远端性能优势。

## 代码审视发现

1. **P1，优化引入的排序缺口：** [FUSE 句柄队列](../../src/node/fuse.rs)覆盖 read/write/flush/fsync/release，却未覆盖带 `fh` 的 `setattr`/`getattr`。`setattr(size, fh)` 会修改打开文件长度；若先到的 write 已入队，后到的 ftruncate 可在接收线程越过它。`getattr(fh)` 也可能等每句柄锁并阻塞整个接收线程。应使所有带 `fh` 的操作走相同的 per-fh 顺序边界，再做 write→truncate 的确定性测试。无需修改 VFS trait。
2. **P1，原有授权校验缺口：** [OwnerFiles RPC 适配](../../src/node/rpc/data.rs)把请求携带的 root/session 与 8 字节 handle 重新组装，[OwnerFs 句柄查找](../../src/node/vfs/ownerfs.rs)未把请求 root 与句柄表中实际 `LocalOpenFile.root_id` 比较。句柄按递增数字分配。**推断：** 持有效 peer 身份和某根授权的节点若构造另一根的 handle，可越过该根的句柄权限边界。应在 Home 的句柄表查找处绑定并校验 root、Home session 和访问 peer；这是局部安全修复，不改变传输协议的大分层。
3. **P2，异步 RELEASE 的回收边界：** [P2P client](../../src/node/rpc/peer.rs)在已确认 I/O 后异步发送 RELEASE；失败只记录日志。B 崩溃或网络失败时，[Home 打开句柄表](../../src/node/vfs/ownerfs.rs)没有按访问节点会话过期的回收路径，FD 可积累。应先界定会话与超时/重连对账，再决定清理实现；不可把 15/15 功能验收当成该故障切点已通过。
4. **P2，性能上限：** 部分远端 `getattr/create/readdir` 仍在 FUSE 接收线程执行；有界队列满时会在接收线程内联执行远端任务。优化后的 0.8 只代表当前 W2，后续扩负载时应按阶段热点决定是否拓宽调度，不能无证据增加 worker 或批量 RPC。16 worker 与 LOOKUP 批量的同场尝试已回归并撤销。

## 证据与下一步

原始结果和环境收据在 `../../../experiments/results/2026-09-27-afs-p2p-concurrency/`：`retest-w2-8-r1.json`、`retest-w2-8-r2.json`、`retest-w2-1.json`、`retest-w1.json`、`retest-acceptance.json`、`retest-evaluation-r1.json`、`retest-evaluation-r2.json`。二进制哈希、每轮顺序、每文件阶段样本和挂载验证均在 JSON 中。本轮未改产品代码；现有未提交修改保留，未 push、merge 或 release。

优先补同句柄排序和句柄授权绑定的定向测试/修复；随后确定远端句柄故障回收合同。完成局部修复后重跑该性能与功能门槛，以确认修复没有吃掉当前收益。
