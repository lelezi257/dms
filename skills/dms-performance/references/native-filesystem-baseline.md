# Native Filesystem 性能基线

分析进程内文件入口时读取本文件。完整数据以
`benchmarks/whitebox/baselines/native-filesystem-vs-glue-lima-aarch64-2026-09-15.json`
及其链接的 evidence 为准。

## 已冻结的公平比较

- 三台相同 Lima aarch64 VM，各 2 vCPU、2 GiB。
- Native 与外置 Glue 共用相同 DMS 二进制来源、文件集合和轮次。
- 三轮交替顺序，双方元数据都使用 memory mode。
- 核心 case：create/write、同节点热读、跨节点首次接管、64 KiB 文件中段 4 KiB 覆盖。
- evaluator 顺序固定为正确性、环境可比性、路径合同、p50/p95。

## 已确认的路径上限

| 路径 | 前台最小控制路径 | 单数据段拷贝阶段 |
| --- | --- | --- |
| 同节点热读 | 0 Worker RPC、0 Meta、0 Peer | 1 |
| 跨节点首次接管 | 1 Meta Lookup + 每个缺失 Block 1 Peer Pull | 3 |
| 新建小文件并写入 | 1 Lookup + 1 Create + 每个 write-through callback 1 Commit | 2 |
| 中段覆盖 | 1 Commit；旧前后 Extent 复用 | 2 |

1 MiB 写当前被 Linux/FUSE 拆成两个有效 write callback，因此产生两个 Block、两个
Commit；跨节点首次接管也相应 Pull 两个 Block。这不是允许无限 RPC，而是当前
write-through 语义下的请求边界。若希望合并，必须先设计 writeback、脏页失败语义和
回收，不得在性能修改中暗改。

## 已确认结果

2026-09-15 基线的 10 个核心 case 全部 PASS：Native p50 比外置 Glue 快
26.4%～62.2%。支持性 peer hot 与覆盖后远端读也全部更快。

## FUSE 请求放大账本

2026-09-15 的 5 轮补充审计把元数据、打开关闭、目录枚举和全部既有 I/O Case
纳入 `fuse-request-amplification-contract.json`。固定结论：

- `stat` 为 1 lookup + 1 getattr，0 DataCore/Meta/Peer。
- 本地热读为 0 Meta/Peer；4 KiB 只进入 1 次 DataCore，1 MiB 进入 2 次。
- 跨节点首读只允许 1 次 Meta Lookup，并按缺失 Block 数 Pull；后续热读均为 0。
- 200 个目录项在当前内核回复缓冲下稳定产生 4 次 readdir callback，但不进入 DataCore、Meta 或 Peer。

> 本节记录 2026-09-15 的历史冻结基线，不能直接充当 M1.7 release 证据。M1.7 最终门禁使用
> 四轮对称交替顺序，每轮 220 个文件（其中 30 个 1 MiB 文件），并要求每个关键 case 至少
> 100 个样本；性能比例阈值没有放宽。
- workload 初始化的 resolve/mkdir 会产生固定 root getattr，必须位于 Metrics 快照之外。

机器入口：

```bash
DMS_FUSE_AMPLIFICATION_RESULT=/path/to/result.json \
  bash scripts/performance/validate_fuse_request_amplification.sh
```

不要重复尝试：

- 不要给进程内文件入口增加 Worker RPC。
- 不要让同节点热读重新访问 Meta。
- 不要为追加或中段覆盖读回并重写完整旧文件；只新增 patch/tail Block。
- 不要把未持久化对照与 local WAL 结果混在同一性能门禁。公平的可靠性性能比较应让
  两边都同步持久化。
- 不要把 FUSE EOF 探测 callback 当成一个完整 payload segment；以 I/O bytes、
  Meta commit 和 Peer Pull 联合判断真实分段。

## 复现与判定

```bash
bash scripts/performance/validate_native_filesystem_performance.sh
```

三 VM 重新采样使用 `scripts/performance/run_native_filesystem_vs_glue_3vm.py`，结果由
`assemble_native_filesystem_result.py` 汇总，再交给 `evaluate_native_filesystem.py`。
不得手工编辑测量结果来取得 PASS。
