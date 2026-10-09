# AFS 源码仓库协作规则

修改本仓库前先读：

1. `README.md`
2. `PRINCIPLES.md`
3. `docs/README.md`
4. `docs/development/plan.md`
5. 与本次改动直接相关的架构、部署或测试页面

## 文档规则

- 正式文档只描述当前接受的目标、能力边界和验收方式，使用中文；命令、配置项、API、许可证和外部名称按原文保留。
- 当前阶段、任务 ID、优先级、有限完成项和待验项统一写入 `docs/development/plan.md`。
- 机制说明放在 `docs/architecture/`；测试和验收放在 `docs/testing/`；部署和试用放在 `docs/deployment/`。
- 过程日志、历史证据、checkpoint、逐轮诊断和大体积运行产物不写入产品源码树；保留在仓库上一级 `local-archive/`，必要时由 README 说明本地入口。
- 不新增指向已归档 `development/`、旧 `docs/guides/`、旧 RFC 或旧交接页的正式链接。

## 代码规则

- 文档清理任务不得改变运行时行为。
- OwnerFs 和 DFS 是不同挂载、不同后端状态机；不要把一个后端的结论自动套到另一个后端。
- SDK 只属于 DFS，除非后续接受的新设计明确改变。
- Rust 构建、FUSE、文件系统和性能验收只在 Linux 环境声明结果；macOS 只用于编辑、编排和只读检查。
- 不修改第三方源码来隐藏适配问题；`third_party/fuser` 的私有补丁依赖必须作为迁移前阻塞项处理。

## 交付规则

- 用户要求 Issue/PR 流程时，使用短分支、关联 Issue 和 PR；PR 说明必须区分产品改动、文档整理、验证证据和未验证边界。
- `main` 是长期集成入口。禁止重写历史、force push 或丢弃历史失败记录。
- 提交信息遵守 Lore 协议。
- 每次报告只说明本轮关闭的子项、真实产品/文档改动、验证结果、阻塞和下一项；不要把工具就绪、文档整理或检查数量当作产品目标完成。
