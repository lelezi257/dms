# 贡献说明

开始改动前先读 [当前计划](docs/development/plan.md)，确认本次属于哪个任务 ID、验收出口和验证范围。

## 设计与文档

- 已接受的机制写入 `docs/architecture/` 下的正式页面。
- 当前目标、优先级、性能判据、迁移前阻塞和剩余事项写入 `docs/development/plan.md`。
- 测试入口和验收规则写入 `docs/testing/`。
- 部署、配置、试用包和运维说明写入 `docs/deployment/`。
- 过程性材料、原始证据和 checkpoint 放在仓库外 `local-archive/`，不要让源码树承担流水账。

## 验证

改动完成后按 [验证说明](docs/testing/validation.md) 选择最小充分检查。PR 或提交报告必须写清：

- 变更了什么行为或文档入口；
- 影响哪些契约；
- 哪些检查已经通过；
- 哪些边界没有验证。

Rust 构建、FUSE、文件系统和性能结论只在 Linux 环境声明。macOS 可以做编辑、静态搜索和 VM 编排。

## Issue/PR

用户要求 Issue/PR 流程时，每个阶段性收尾使用一个 Issue 承载目标和边界，并用 PR 交付可审查 diff。不要为单点工具检查反复开新结论，也不要把未完成项写成已完成。

## Lore 提交协议

首行写明为什么需要这次变更；正文只记录影响决定的背景。按实际需要添加 Git trailers，例如：

```text
Keep the standalone trial usable after documentation moves

Constraint: Historical evidence retains its original version and result
Rejected: Copying historical scripts into each candidate | Increases repository burden
Confidence: high
Scope-risk: narrow
Directive: Keep runtime output outside the product tree
Tested: Linux package and installer regressions
Not-tested: Full POSIX and performance acceptance
```

示例不是本次验证结论；`Tested` 和 `Not-tested` 必须填写本提交的真实范围。
