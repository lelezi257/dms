# Contributing to AFS

AFS 接受文档、测试、工具、协议、存储和文件语义方面的贡献。所有修改都需要明确目标能力、当前状态和验证证据。

## 阅读顺序

1. [README](README.md)
2. [架构原则](PRINCIPLES.md)
3. [产品定位](docs/product-positioning.md)
4. [架构总览](docs/architecture/overview.md)
5. [当前状态](docs/current-status.md)
6. [路线图](ROADMAP.md)
7. [代码地图](docs/code-layout.md)

## 选择任务

任务按领域和难度标记：

- `area/meta`、`area/blobfs`、`area/ownerfs`、`area/storage`、`area/p2p`、`area/sdk`、`area/fuse`、`area/docs`
- `kind/rfc`、`kind/implementation`、`kind/test`、`kind/benchmark`、`kind/documentation`
- `difficulty/good-first-issue`、`difficulty/intermediate`、`difficulty/advanced`
- `status/ready`、`status/blocked-by-rfc`、`status/needs-evidence`

适合首次贡献的任务应当具备明确文件范围、输入输出、验收命令和预期结果。复制协议、Snapshot 原子性、GC 和故障恢复需要先有 Accepted RFC。

## 设计修改

以下修改需要 RFC：

- 外部 POSIX 语义；
- inode、dentry、file layout 和版本身份；
- 副本、缓存、发布和 spill 状态机；
- wire protocol 和持久格式；
- 错误码兼容性；
- 跨模块依赖方向；
- 可靠性承诺。

RFC 模板见 [docs/rfcs/0000-template.md](docs/rfcs/0000-template.md)。RFC 合并表示设计被接受，不表示功能已经实现。

## 实现要求

- `ref/` 中的上游源码保持只读。
- Meta、Node、Storage、OwnerFs 和 BlobFs 的职责边界保持清晰。
- Meta 不进入文件内容数据路径。
- 未完成副本不可读、不可计入可靠性、不可成为 seed。
- 优化不得改变公开一致性和持久化语义。
- 不支持的操作返回明确错误。
- 新能力更新对应状态、架构和验证文档。

## 验证

修改需要使用能够证明目标声明的最小验证集合。文件语义修改优先验证对应 POSIX Case，再验证故障边界和性能。性能结果必须说明介质、耐久策略、缓存状态、节点数、负载和对照组。

Linux 是权威构建和测试环境。具体命令见[运行指南](docs/foundation-running.md)。

## Pull Request 描述

PR 应说明：

- 解决的用户 Case；
- 改变的外部或内部合同；
- 涉及的模块；
- 验证结果；
- 未覆盖的边界；
- 关联 RFC 和 Issue。
