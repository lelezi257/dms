# AFS 基础框架阶段交付

日期：2026-09-25。分支：`feat/agent-workloads-foundation`。本次交付可运行、可继续扩展的正式底座，不合并 main，不发布产品版本。

## 已实现

- `afs-meta` / `afs-node` 进程、CLI/TOML 配置、OwnerFs/BlobFs 独立编译和运行开关。
- 一套 FUSE/VFS 入口、两个隔离 namespace；创建请求已接到对应后端，当前明确返回 `ENOSYS`。
- 本机 SDK：UDS gRPC 控制 + SHM 数据；Node→Meta 与节点控制 gRPC；节点数据使用共同 API 下的 gRPC / 单边 RDMA adapter。
- RDMA 握手参考 3FS：一次 `NegotiateData` 交换参数，再通过真实 `SEND_WITH_IMM` 探测验证通道，取消独立 `ReadyData` RPC。握手版本 1 需要两端同时升级。
- REST、日志、错误、metrics、跨服务 OTLP Trace，以及配置、资源生命周期和诊断读写测试。
- Linux CI、运行示例、中文代码导读及架构文档。

## 验证结果

Linux ARM64 / Rust 1.95：最新常规测试 57 项通过；RXE 底层探测 3 项、节点会话 4 项通过；gRPC 和 RDMA 独立进程 E2E 各 22 项通过。基础框架阶段另已执行真实 FUSE 专项 3 项，全部通过。

fmt、严格 Clippy、编译 feature 矩阵、bins/examples 构建和严格 Rustdoc 通过。提交前重新执行当前工作树的 fmt、workspace all-features 测试和 Clippy；设备和 E2E 结果来自同阶段专项验收。GitHub CI 状态以该分支 Actions 页面为准，不能用本地结果代替。

复现命令、依赖和启动步骤见[运行指南](foundation-running.md)，测试入口见[tests/README.md](../tests/README.md)。原始日志保存在开发研究区的 `evidence/afs-foundation-20260925/` 与 `evidence/afs-rdma-probe-20260925/`，不随源码提交；本页是仓库内可独立阅读的验收摘要。

## 尚未提供

完整 OwnerFs/BlobFs 文件业务、根授权、共享缓存一致性、镜像发布/副本/GC、持久恢复及生产鉴权尚未实现。诊断存储不能当成已授权业务接口。SHM/RDMA 暂存区仍有拷贝；RXE 证明机制，不证明硬件性能或跨机器收益。

下一阶段按[下一步](next.md)实现业务，不重新引入旧 NFS/KV 路径。
