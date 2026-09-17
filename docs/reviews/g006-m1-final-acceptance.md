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

## 3. 业界测试集与覆盖边界

本轮不是只运行项目自研脚本。最终验收固定并实际执行了以下上游测试集；版本、清单和原始结果都进入证据目录，
缺少测试集、缺少清单中的 case、上游返回 `Not run` 或任一 case 失败都会使验收失败，不能转成 SKIP。

| 测试集 | 拓扑与冻结身份 | 本轮实际执行 | 覆盖价值 | 结果 |
| :--- | :--- | :--- | :--- | :--- |
| `pjdfstest` | 单 VM；`pjd/pjdfstest@85a8aea9e685999ef0540392fd80535f873d7ff7`，工作树干净 | `mkdir/00`、`rmdir/00`、`open/00`、`unlink/00`、`rename/00`、`truncate/00`、`link/00`、`symlink/00`、`chmod/00`、`chown/00`、`utimensat/00` | 基础 namespace、文件身份、权限、时间和 size 语义；包含真实 uid/gid 身份矩阵 | 11/11 PASS |
| `xfstests` | 单 VM；`kdave/xfstests@a370dcbed43563f0462801e889e0eceb93c7cfad`，工作树干净 | `generic/001`、`generic/013`、`generic/075`，以 `./check -fuse` 在真实 DMS mount 执行 | 数据复制链与损坏检查；单/多进程 `fsstress`；随机读写、truncate、预分配组合 | 3/3 PASS |
| `fsx` | 单 VM；由上述冻结的 `xfstests generic/075` 提供，不是另一份自研替代程序 | 四轮：1,000 次普通随机操作、1,000 次带预分配、10,000 次/10 MiB 普通随机操作、10,000 次/10 MiB 带预分配 | 对同一文件反复随机 read/write/truncate 与预分配，持续核对模型数据和真实文件 | 4/4 轮 PASS |
| `fio` | 单 VM + 三 VM；两套环境均为 `fio-3.36` | 4 KiB 顺序写、1 MiB/4 KiB block 随机写、512 MiB/1 MiB block 顺序写；均使用 sync engine、CRC32C verify、`end_fsync=1` | 覆盖小写、随机覆盖和大文件顺序写，并在三 VM 拓扑核对跨 Node SHA-256；本轮主要用于 I/O 完整性，不拿它冒充跨产品吞吐排名 | 两种拓扑各 3/3 fio case PASS |

可复核入口：

- `pjdfstest` 冻结清单：[pjdfstest-supported.txt](../../scripts/validation/m1/posix/pjdfstest-supported.txt)；机器结果：[suite-result.json](../../evidence/m1/g006-release-single-b288313-r3-20260917/cases/posix-pjdfstest-supported/artifact/suite-result.json)。
- `xfstests/fsstress/fsx` 冻结清单：[fstests-generic-supported.txt](../../scripts/validation/m1/posix/fstests-generic-supported.txt)；机器结果：[suite-result.json](../../evidence/m1/g006-release-single-b288313-r3-20260917/cases/fstests-generic-supported/artifact/suite-result.json)。
- `fio` 单 VM 工作负载：[fio-workload.json](../../evidence/m1/g006-release-single-b288313-r3-20260917/cases/fio-integrity/artifact/fio-workload.json)；三 VM 工作负载：[fio-workload.json](../../evidence/m1/g006-release-three-b288313-20260917/cases/fio-integrity/artifact/fio-workload.json)；单 VM 判定结果：[evaluation.json](../../evidence/m1/g006-release-single-b288313-r3-20260917/cases/fio-integrity/artifact/evaluation.json)。

`fio` 之外又执行了三条 DMS 扩展完整性检查：truncate 缩小后再扩展、punch-hole 后洞区读零、
`MAP_SHARED` 修改后的跨挂载 SHA-256 一致。这些属于项目验收扩展，不冒充 fio 上游 case。

业界测试集负责验证标准 syscall 与 I/O 模式，因此放在单 VM 的真实 FUSE mount 上执行；跨 Node 可见性、
Peer pull、Watch 失效、重启 fencing、资源回落和 Agent 工作区混合负载没有现成上游套件能表达，继续由三 VM
DMS 自研 case 验证。两类证据互补，不能用自研 E2E 代替上游兼容性测试，也不能用单机 POSIX suite 代替分布式故障测试。

本轮声明边界是**冻结的 M1 支持子集**，不是完整 `pjdfstest`、完整 `xfstests`、LTP 文件系统全套、长期
`fsstress` soak 或 POSIX 认证。后续只有当新增上游 case 在固定版本、真实 DMS mount 和无 SKIP 条件下稳定通过，
才会加入受支持清单；当前报告不对未执行的 case 作兼容承诺。

## 4. 功能与故障闭环

三 VM故障矩阵同时证明以下状态转换：

1. 内容提交完成后才发布文件版本，重启后仍可恢复权威 binding。
2. 本地没有 Block 时，Node 根据权威布局执行 Peer pull；拉取完成后可在本地复用。
3. 远端修改先使 Node/FUSE kernel cache 失效，再 ACK Watch 事件。
4. 阻塞锁在释放后唤醒；Meta 重启与 Node incarnation 变化不会恢复旧 owner 的锁。
5. cached mmap 的 MAP_SHARED 写经 write-through 发布；远端失效、truncate 与越 EOF 行为由真实内核路径验证。

这轮独立 Review 还修复了两条 release 竞态：旧 Release 响应不能删除更新的 Set 镜像；已安全过期的
Release 不能被误判为需要重试的 RPC 失败。最终 Rust 全 workspace 测试与三 VM故障矩阵均覆盖修复后代码。

## 5. 性能结论

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

## 6. 资源与完整性

- 两个 Node 的 `arena_reservations_after=0`、`filesystem_inode_references_after=0`。
- Meta 的 `watch_lag_after=0`；两条 live Node session 符合测试拓扑。
- 两个 Node 的 FD 仅各增加 1，线程无增长；RSS 与 Arena allocated 增量均低于合同预算。
- fio 六类必需 case 全部通过，包含 512 MiB 顺序对象、1 MiB 随机写、4 KiB 顺序写和跨挂载校验。

## 7. Review 后仍然成立的边界

1. M1 是共享 POSIX foundation，不等于 M2 的多副本持久可靠性或多 Meta HA。
2. 当前采用 local-memory 数据可靠性；Node/机器永久故障后的 value 恢复属于 M2。
3. cached mmap 保持 write-through，不启用 writeback cache。
4. RDMA/UB 只保留 provider 扩展边界，本阶段没有实现或宣称性能。
5. 功能分支继续保留 `feat/native-filesystem`，由 PR 交付 Review，不在本次验收中自动合入 `main`。

## 8. 最终判定

M1.7 验收合同要求的已声明范围已经闭环：没有 planned case、skip、warning、数据损坏、明显资源泄漏，
也没有超过合同的非必要 RPC/复制。M1 Shared POSIX Foundation 可以进入人工 PR Review；下一产品阶段是
M2 Reliable Production，而不是继续无边界扩张 M1 验收。
