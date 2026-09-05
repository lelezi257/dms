# 阶段产物合同

## 一份内容，两种阅读方式

Markdown 是唯一可编辑正文；HTML 是确定性阅读视图。图的参与者、箭头、解释、API、状态和结论都应来自 Markdown，不允许 HTML 加一套独有设计。

脚本 `scripts/docs/render_review.py` 仅依赖 Python 标准库。HTML 内嵌源 Markdown 与 SHA-256、可选 baseline 原文与哈希；`--check` 比较完整生成结果，源变化、baseline 变化或手改 HTML 都应失败。

这不是密码学签名或人工批准证明；`--baseline-confirmed` 只是调用者对“该独立 MD 快照已经人工认可”的显式声明。

## 当前支持的格式

使用标题、段落、单层有序/无序列表、表格、代码块、引用、普通链接和行内代码/加粗。表格需要标准分隔行，列表与段落留空行。raw HTML 只显示为文字，不执行；不支持完整 CommonMark、Mermaid、嵌套列表或历史专用图语法。不要依赖未支持格式偷偷承载语义。

通用时序图使用下例；HTML 同时保留可展开原文：

```sequence
participant App as Application
participant Node as dms-node
participant Meta as dms-meta
App ->> Node: SET 输入 key/value
Node ->> Meta: 提交布局和位置
Meta -->> Node: 返回版本
Node -->> App: 返回成功
```

示例只演示表达格式，不替代业务设计。需要进程套模块等更丰富图时，先扩展通用、可测试的渲染规则，并让 MD 中有完整原图/模型；不要只修改某个 HTML。不得以图更漂亮为由改变设计内容。

链接以源 Markdown 所在目录解析，生成器会按 HTML 输出位置重定位；同文链接指向 HTML 自身。其它 Markdown 不会被擅自替换为未生成的 HTML。源码正式文档只能链接源码内必要内容；阶段研究报告可以链接本次证据，但不能让用户安装教程依赖个人路径。

## 每轮修改看得见

小改以最后一次**人工已确认**的独立 Markdown 为 baseline：新增使用绿色与“新增”文字，修改使用蓝色与“修改”文字；删除/修改前内容折叠展示。列表项和表格行分别标识，提供“只看变化”，不因为一句话改变就把整个章节染色。颜色不是唯一信号。

首轮、巨大重写或用户免差异要求：全量呈现，不为了 diff 强行保留旧叙述。baseline 与当前源不能是同一文件；不能使用 HTML 作为 baseline；未 review 的轮次不会自动成为新基线。

## 内容与验收

先给本轮结论和少量 review 决策，再展开当前阶段相关细节。状态标识区分已实现、设计、推断、待验证；不要把接口罗列冒充原理，也不要把历史证据当成本轮新测试。

生成后运行 `--check`，检查每个本地链接、图中名称与正文一致、无敏感信息。工具验证入口是 `python3 -m unittest discover -s scripts/docs -p 'test_*.py'`，同样在 Linux 执行。静态通过只证明格式和同源性；视觉布局、人是否理解仍需如实记录检查范围。
