# DMS：分布式近计算内存对象系统

DMS 把计算节点的一部分内存用于保存和共享数据。应用通过 Rust SDK 使用 `set/get/del`、批量操作、随机写和两级键操作；同节点可使用共享内存，跨节点通过网络获取数据。Meta 保存版本与位置，不转发用户 value。

**当前版本为 0.1.0 开发预览，源码托管在 [GitHub 私有仓库](https://github.com/lelezi257/dms)。** 访问需要仓库权限；尚未发布 GitHub Release 或 crates.io 包。适合开发与功能验证，不作为生产持久存储。当前写入保证仅为本地内存，Node 重启可能丢失 value；完整内存回收和多 Meta 高可用尚未完成。先看[能力与限制](docs/product.md)。

## 从这里开始

| 你要做什么 | 入口 |
| --- | --- |
| 准备 Linux 环境并构建 | [安装与构建](docs/installation.md) |
| 安装 SDK/服务候选包 | [候选制品安装与验收](docs/release-installation.md) |
| 手动启动 Meta、Node，验证 SET/GET | [单 VM 教程](docs/local-single-vm-manual.md) |
| 写自己的 Rust 应用 | [Rust SDK 编程](docs/rust-sdk.md) |
| 理解进程分工与数据流 | [架构与关键流程](docs/architecture.md) |
| 改参数、看观测、排查错误 | [配置](docs/configuration.md) · [观测](docs/observability.md) · [排障](docs/troubleshooting.md) |
| 参与开发或让 AI 接手 | [贡献指南](docs/contributing.md) · [AGENTS.md](AGENTS.md) · [项目 Skills](skills/README.md) |

## 最小应用

这是普通 Rust 同步程序，连接已启动的 Node。完整依赖设置、执行命令见 [SDK 教程](docs/rust-sdk.md)。

```rust
use dms_client::{ClientOptions, DmsClient};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = DmsClient::connect("http://127.0.0.1:25200", ClientOptions::default())?;
    client.set("example/key", b"hello")?;
    assert_eq!(client.get("example/key")?.as_deref(), Some(&b"hello"[..]));
    client.del("example/key")?;
    assert!(client.get("example/key")?.is_none());
    Ok(())
}
```

`Ok(None)` 表示 key 不存在；连接、超时、容量等失败是 `Err(DmsError)`，不能用空值代替。同步 SET 成功后立即 GET 不需要人为等待。

## 工程目录

| 目录 | 责任 |
| --- | --- |
| `sdk/rust/dms-client/` | 应用直接使用的 Rust SDK；其它语言目录目前只有规划说明。 |
| `server/` | `dms-node`、`dms-meta` 及健康检查工具。 |
| `protocol/` | 组件间 protobuf 源码，不是用户必须手写的接口。 |
| `common/` | 内部共用的错误、传输、共享内存、日志、Metrics、Tracing 库。 |
| `scripts/` | 构建、部署、验证与观测栈入口。 |
| `docs/` | 用户与开发者文档。 |
| `skills/` | 本项目的轻量开发流程和产物模板。 |

用户产品面是 **SDK + 服务组件**，`common` 不是第三个服务。源码工作区使用路径依赖；SDK 候选包内含私有共用实现与预生成协议，消费者只依赖 `dms-client`，不需要安装其它 DMS crate 或 protoc。候选包验收与正式仓库发布是两件事。

权威环境为 Linux，Rust 工具链由 [rust-toolchain.toml](rust-toolchain.toml) 固定，依赖由 [Cargo.lock](Cargo.lock) 固定。macOS 只用于编辑和阅读；编译、测试、示例及服务运行都进入 Linux VM/容器执行。
