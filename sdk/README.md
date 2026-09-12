# SDK

当前可用实现包括 [Rust dms-client](rust/dms-client/README.md) 和[纯 Go SDK](go/README.md)。应用使用各语言的公开 API 和原生错误类型，不需要操作生成的 protobuf 类型。

| 目录 | 当前状态 |
| --- | --- |
| `rust/dms-client/` | Rust SDK；支持源码开发及独立候选包安装，已验证真实 Node 路径。 |
| `python/` | 预留纯 Python SDK，尚未实现。 |
| `cpp/` | 预留 C++ SDK 与头文件，尚未实现。 |
| `go/` | 纯 Go SDK；当前提供 Set、Get、Del、Stat、Scan 及相关 Options，支持 TCP 和 Linux 本机 UDS/SHM。 |

语言间共享的是 [protocol](../protocol/) 的进程通信约定，不要求用户依赖 Rust FFI。**当前没有公开发布的 SDK 版本**；Rust `.crate` 与 Go module 候选包已完成独立消费者安装及真实进程验证，但这不代表所有功能、生命周期和性能验收均已完成。候选包与源码内部依赖的布局不同，安装时使用交付方提供的候选仓库和版本，不要假设公共仓库已存在对应版本。

入门见 [Rust 编程教程](../docs/rust-sdk.md)、[Rust 候选包安装](../docs/release-installation.md)和 [Go SDK 教程](../docs/go-sdk.md)，部署见[单 VM 手册](../docs/local-single-vm-manual.md)。
