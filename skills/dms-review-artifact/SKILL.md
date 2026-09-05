---
name: dms-review-artifact
description: 为 DMS 阶段方案、代码导读或验收报告生成可人工 review 的 HTML 时使用；也用于修改已审阅文档、保证 Markdown/HTML 一一对应、显示每轮修改。维护一个 Markdown 内容源，用项目内工具生成并校验 HTML，不手写第二份正文。
---

# 同源审阅产物

先完整读取 [artifact-contract.md](references/artifact-contract.md)，然后再改文档。它定义正文结构、生成语法、差异与验收规则。

## 执行

1. 找到本轮唯一 Markdown、对应 HTML，以及有没有人工已确认的 Markdown baseline。没有 baseline 时使用全量视图，不伪造“已确认”。
2. 写清当前阶段最重要的内容。用用户输入解释原理，图、状态、API 名称对应；避免所有阶段都面面俱到。
3. 在 Linux 环境的源码根调用下面命令生成并检查。输出可以在本次约定的仓外研究目录，但脚本不得依赖那里。

```bash
python3 scripts/docs/render_review.py PLAN.md PLAN.html
python3 scripts/docs/render_review.py PLAN.md PLAN.html --check
```

小轮次修改，且 baseline 已获人工认可时：

```bash
python3 scripts/docs/render_review.py PLAN.md PLAN.html --baseline accepted.md --baseline-confirmed --round-label R2
python3 scripts/docs/render_review.py PLAN.md PLAN.html --baseline accepted.md --baseline-confirmed --round-label R2 --check
```

`PLAN.md` 等是占位路径，实际命令使用本轮文件。大规模重写或用户明确无需差异时不传 baseline。不能自动把刚生成的 HTML 或未审阅 Markdown 提升成 baseline。

最后检查链接和图文对应；能在浏览器查看时再检查布局与差异开关。仅静态校验就如实声明，不声称已完成视觉验收。修改只回到 Markdown 或通用 renderer；禁止手改生成 HTML 正文。
