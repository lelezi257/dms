# M1.7 产品级总验收结果

> 结论（2026-09-17）：`feat/native-filesystem` 的干净提交
> `b288313a9610f1056bd187f8ee2ba819e221d4d6` 已完成最终单 VM 与三 VM release 验收。
> 单 VM 13/13、三 VM 14/14 全部通过，均为 0 FAIL、0 SKIP、0 warning。

## 1. 这次证明了什么

M1.7 不是“脚本跑完”这一条结论，而是把 M1.1～M1.6 的产品声明重新放到同一个最终源码提交上验证：

| 维度 | 最终结果 | 关键证明 |
| :--- | :--- | :--- |
| 单 VM POSIX | PASS，13/13 | namespace、size、identity、attrs、space/sync、lock、mmap、pjdfstest、xfstests、fio |
| 三 VM分布式 | PASS，14/14 | 跨 Node 可见、Peer pull、Watch 失效 ACK、Meta/Node 重启、epoch fencing |
| 数据完整性 | PASS | fio verify、跨 Node SHA-256、truncate、打洞、MAP_SHARED |
| Agent 工作区 | PASS | 180 个固定种子混合操作，190 次跨 Node 校验，最终目录树 digest 一致 |
| 资源回落 | PASS | Reservation、inode reference、watch lag 回到 0；RSS/FD/thread/Arena 在预算内 |
| 白盒路径 | PASS | FUSE、DataCore、Meta、Peer 和复制次数没有超过冻结合同 |
| 性能 | PASS | 六轮对称交替；读路径保持领先，write-through mutation 固定控制成本在预算内 |
| 交付 | PASS | 从干净发布包完成启动、挂载、跨节点读取、停止与卸载，不借用源码运行依赖 |

## 2. 最终执行身份

```text
branch: feat/native-filesystem
commit: b288313a9610f1056bd187f8ee2ba819e221d4d6
dirty:  false

single-vm: PASS 13 / FAIL 0 / SKIP 0
three-vm:  PASS 14 / FAIL 0 / SKIP 0
```

机器结果：

- `evidence/m1/g006-release-single-b288313-r3-20260917/evaluation.json`
- `evidence/m1/g006-release-three-b288313-20260917/evaluation.json`
- 冻结摘要：`benchmarks/whitebox/baselines/m1.7-release-lima-aarch64-2026-09-17.json`

## 3. 功能与故障闭环

三 VM故障矩阵同时证明以下状态转换：

1. 内容提交完成后才发布文件版本，重启后仍可恢复权威 binding。
2. 本地没有 Block 时，Node 根据权威布局执行 Peer pull；拉取完成后可在本地复用。
3. 远端修改先使 Node/FUSE kernel cache 失效，再 ACK Watch 事件。
4. 阻塞锁在释放后唤醒；Meta 重启与 Node incarnation 变化不会恢复旧 owner 的锁。
5. cached mmap 的 MAP_SHARED 写经 write-through 发布；远端失效、truncate 与越 EOF 行为由真实内核路径验证。

这轮独立 Review 还修复了两条 release 竞态：旧 Release 响应不能删除更新的 Set 镜像；已安全过期的
Release 不能被误判为需要重试的 RPC 失败。最终 Rust 全 workspace 测试与三 VM故障矩阵均覆盖修复后代码。

## 4. 性能结论

最终性能不是单轮数字，而是六轮 `Native→Glue` / `Glue→Native` 对称交替，同轮配对后取中位数。

| 场景 | 4 KiB | 64 KiB | 1 MiB | 结论 |
| :--- | ---: | ---: | ---: | :--- |
| 本地热读 p50 提升 | 45.45% | 45.29% | 58.30% | Native 全部领先 |
| 跨 Node 首读 p50 提升 | 23.78% | 26.88% | 20.04% | Native 全部领先 |
| create p50 固定开销 | +276.53 µs | +286.30 µs | -101.86 µs | 低于 +400 µs 门禁 |
| create p95 固定开销 | +425.20 µs | +255.47 µs | +128.23 µs | 低于 +500 µs 门禁 |
| 64 KiB 中段覆盖 | — | p50 +225.17 µs / p95 +193.19 µs | — | 低于 +250/+300 µs 门禁 |

读路径体现进程内 Native 入口的价值；同步 create/overwrite 仍承担 Meta 发布和 write-through 控制成本。
当前结论不把固定控制成本隐藏成百分比，也没有为了通过而删除 RPC 或降低一致性语义。白盒放大门禁独立为 PASS。

## 5. 资源与完整性

- 两个 Node 的 `arena_reservations_after=0`、`filesystem_inode_references_after=0`。
- Meta 的 `watch_lag_after=0`；两条 live Node session 符合测试拓扑。
- 两个 Node 的 FD 仅各增加 1，线程无增长；RSS 与 Arena allocated 增量均低于合同预算。
- fio 六类必需 case 全部通过，包含 512 MiB 顺序对象、1 MiB 随机写、4 KiB 顺序写和跨挂载校验。

## 6. Review 后仍然成立的边界

1. M1 是共享 POSIX foundation，不等于 M2 的多副本持久可靠性或多 Meta HA。
2. 当前采用 local-memory 数据可靠性；Node/机器永久故障后的 value 恢复属于 M2。
3. cached mmap 保持 write-through，不启用 writeback cache。
4. RDMA/UB 只保留 provider 扩展边界，本阶段没有实现或宣称性能。
5. 功能分支继续保留 `feat/native-filesystem`，由 PR 交付 Review，不在本次验收中自动合入 `main`。

## 7. 最终判定

M1.7 验收合同要求的已声明范围已经闭环：没有 planned case、skip、warning、数据损坏、明显资源泄漏，
也没有超过合同的非必要 RPC/复制。M1 Shared POSIX Foundation 可以进入人工 PR Review；下一产品阶段是
M2 Reliable Production，而不是继续无边界扩张 M1 验收。
