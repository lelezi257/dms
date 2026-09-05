# SDK

当前可用实现为 [Rust dms-client](rust/dms-client/README.md)。应用声明一个直接依赖，使用其公开 API 和原生错误类型，不需要操作生成的 protobuf 类型。

| 目录 | 当前状态 |
| --- | --- |
| `rust/dms-client/` | 可在源码工作区构建、连接真实 Node 的 Rust SDK。 |
| `python/` | 预留纯 Python SDK，尚未实现。 |
| `cpp/` | 预留 C++ SDK 与头文件，尚未实现。 |
| `go/` | 预留 Go SDK，尚未实现。 |

语言间共享的是 [protocol](../protocol/) 的进程通信约定，不要求用户依赖 Rust FFI。**当前尚未发布 SDK 包**；源码内部依赖与最终发布制品布局不是同一件事，实际构包及独立安装将在发布阶段验证。

入门见 [Rust 编程教程](../docs/rust-sdk.md)，部署见[单 VM 手册](../docs/local-single-vm-manual.md)。
