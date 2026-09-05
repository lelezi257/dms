# 项目开发 Skills

这些是随源码维护的工作方法，不是运行时依赖，也不会安装到 SDK 用户的进程中。它们把当前阶段的输入、产物和人工 review 重点定下来；实现事实仍以代码及 [产品边界](../docs/product.md) 为准。

| 入口 | 何时使用 | 解决什么问题 |
| --- | --- | --- |
| [dms-delivery](dms-delivery/SKILL.md) | 需求、设计、穿刺、开发、测试或发布准备 | 只加载当前阶段，交付可验证的小批次，避免漏掉环境与验收 |
| [dms-review-artifact](dms-review-artifact/SKILL.md) | 输出供人工审阅的 Markdown/HTML，或修改已审阅方案 | 同源生成、链接可追溯、小轮次突出修改，不维护两份内容 |

## 怎么让 AI 使用

源码根 [AGENTS.md](../AGENTS.md) 指向这里。可以明确要求：“先读 `skills/dms-delivery/SKILL.md`，按设计阶段整理这次变更，再用 `skills/dms-review-artifact/SKILL.md` 生成 HTML。”

**任意 `skills/` 目录不保证被所有 AI 工具自动发现。** 工具支持安装项目 skill 时可按其机制显式配置；不支持时直接读文件即可。这里不要求全局安装、不依赖特定模型或外层研究目录。

## 当前状态与修改原则

本版由本项目已执行的设计、穿刺、重构和基础组件交付过程精简而来；S3 提供工具测试与独立使用检查，仍等待本阶段人工验收，不宣称已适配所有工具。

只因实际遗漏或 review 意见修改模板；一次意见先修对应规则，不把全部历史对话复制进来。阶段报告记录本轮改了什么方法，以及是否已验证。新增步骤必须说明它减少了哪类漏项，否则不添加。

日常小修不需要走五份长文。详细约束只维护在 [AGENTS.md](../AGENTS.md)、[开发指南](../docs/contributing.md) 与 [产物合同](dms-review-artifact/references/artifact-contract.md)。
