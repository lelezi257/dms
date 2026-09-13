# DMS 0.1.0 发布说明

0.1.0 是首个公开功能验证版本。它提供 Rust/Go SDK、Linux Node/Meta 服务，以及可复现的安装、单机和三节点验收入口。

## 下载与版本

- 源码与发布页：`https://github.com/lelezi257/dms/releases/tag/v0.1.0`
- Rust SDK：发布页资产 `dms-client-0.1.0.crate`
- Go SDK：`go get github.com/lelezi257/dms/sdk/go@v0.1.0`
- 服务包：发布页中与机器架构匹配的 `dms-server-0.1.0-linux-<arch>.tar.gz`
- 所有二进制资产必须先用同名 `.sha256` 或发布页 `SHA256SUMS` 校验。

Rust SDK 当前作为 GitHub Release 资产交付，尚未发布到 crates.io。需要 Cargo 在线依赖时，可以固定 Git tag：

```toml
[dependencies]
dms-client = { git = "https://github.com/lelezi257/dms.git", tag = "v0.1.0" }
```

## 已验证能力

- KV/Object：完整读写删除、批量、范围读写、分页扫描。
- Hash/KKV：字段读写、批量读取、分页、字段内随机写。
- 本地 UDS + 共享内存，以及 TCP/gRPC 跨进程、跨 Node 读取。
- Meta 版本与位置管理、失效通知、WAL/snapshot 恢复。
- 数字错误码、结构化日志、Prometheus Metrics、可选分布式 Trace。
- 独立 SDK 消费者、单 VM、三 VM、恢复和性能回归门禁。

## 重要限制

- 数据只保存在 Node 内存，Node 重启可能丢失 value；Meta WAL 不能恢复这些 bytes。
- 单 Meta，不提供 Meta 高可用。
- 只发布并承诺发布页列出的 Linux 架构；没有 Windows/macOS 服务包。
- RDMA、UB、L2、设备内存、Python/C++ SDK 未实现。
- 共享内存只面向同机可信进程，不等价于多租户安全隔离。
- 0.1.0 不承诺稳定 ABI、生产 SLA 或跨版本滚动升级。

安装、启动、校验、升级和回滚步骤见[发布包安装与验收](release-installation.md)。
