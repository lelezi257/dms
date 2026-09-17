---
name: dms-performance
description: 在 DMS 项目复现性能基线、定位 Client/Node/Meta 或文件适配路径开销、实施性能优化和判断回归时使用。要求先冻结环境与调用图，再用组成下界和机器 evaluator 验收；不用于普通功能开发。
---

# DMS 性能分析与优化

先读源码根 `AGENTS.md`、[`docs/performance/whitebox-baseline.md`](../../docs/performance/whitebox-baseline.md) 和 [`benchmarks/whitebox/contract.json`](../../benchmarks/whitebox/contract.json)。只有需要重新归因时再读 [诊断账本](references/diagnostic-ledger.md)；分析 Native Filesystem 时再读 [文件路径基线](references/native-filesystem-baseline.md)。

## 工作合同

1. 固定 source SHA、环境 `profile.id`、节点拓扑、transport、对象大小、预热方式和样本数。环境不一致时禁止硬比较。
2. 先复现候选值，再拆成 Adapter/FUSE 外壳、Client→Node、Meta、Peer payload、本地交付和残差；不要先改代码。
3. 对照机器合同逐项审计前台 RPC、整段 payload copy 和 allocation。不能用当前实现反向修改“最小值”。
4. 只有证据指向某段非必要工作才修改。优先删除重复 RPC、复制、摘要或同步等待，不增加新的通用抽象。
5. 不修改公开 SDK、protobuf 或 Client/Node/Meta 核心职责，除非用户先批准独立设计变更。保持薄 Client；跨语言 SDK 不能各自发明语义。
6. 性能埋点必须可关闭或低基数。成功 heartbeat/watch 默认不产生 Trace；不要在热路径输出逐请求日志或 key/object label。
7. 修改后先跑正确性门禁，再生成与基线同 schema 的候选结果并执行 evaluator。失败就继续归因，不能只用均值或单次样本宣布提升。
8. 文件路径对比必须让双方使用相同可靠性合同。memory 与 local WAL 不能直接判性能输赢；write-through 的一个 callback 对应一次发布，除非先批准独立 writeback 设计。
9. `payload_copy_stages` 只表示单个数据段的完整字节复制路径深度。逻辑操作内的分段数量必须由 callback、Meta commit 和 Peer Pull 实测计数表达，不能把两者混成一个数字。
10. FUSE 优化不能只检查挂载参数。必须同时比较原始 callback、文件业务操作、DataCore、Meta、Peer 和边界字节；workload 自身的 resolve/mkdir 等动作必须放在 Metrics 快照之前，不能污染业务账本。

## 固定判定

- 所有受保护 Case 的 p50 不得比同环境基线回归超过 10%。
- `local_hot` 必须接近组成下界且优于同环境中心式对照。
- `advantaged` 必须兑现架构优势。
- `architecture_penalty` 可以慢于中心式对照，但必须在组成下界 `1.25x` 内，并量化后续热读收益。
- 未解释耗时不得超过端到端 p50 的 10%；超过就继续补测，不猜根因。

执行入口：

```bash
python3 scripts/performance/evaluate_whitebox.py /path/to/candidate.json \
  --output /tmp/dms-whitebox-evaluation.json
```

Native Filesystem 与外置 Glue 的固定入口：

```bash
bash scripts/performance/validate_native_filesystem_performance.sh
```

FUSE callback、DataCore 和 RPC 放大门禁：

```bash
DMS_FUSE_AMPLIFICATION_RESULT=/path/to/result.json \
  bash scripts/performance/validate_fuse_request_amplification.sh
```

## 交付

报告只保留：环境与 SHA、Case、旧/新 p50、调用图变化、分段证据、正确性结果、evaluator 结果和未测项。代码通过中文 Issue/PR 提交审阅，不自动合并；性能优化提交不得夹带架构重写。
