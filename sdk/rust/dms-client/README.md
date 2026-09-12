# dms-client

DMS 的 Rust 同步 SDK。应用可连接本地 UDS 或远端 TCP Node，使用普通字节 KV、批量、Hash/KKV、范围读写及显式共享内存 API。

**当前没有公开发布的 SDK 版本。** 不应直接用 `dms-client = "0.1"` 假设公共仓库已有包。源码开发仍使用工作区内部依赖；正式构包脚本会把私有实现与预生成协议纳入单个候选 `.crate`，消费者只声明 `dms-client`。候选包已完成独立安装和真实 Node 调用验证，安装方式见[候选仓库安装](../../../docs/release-installation.md)；这不代表整个产品已完成验收或获准公开发布。

```rust
use dms_client::{ClientOptions, DmsClient};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = DmsClient::connect("http://127.0.0.1:25200", ClientOptions::default())?;
    client.set("example/key", b"hello")?;
    assert_eq!(client.get("example/key")?.as_deref(), Some(&b"hello"[..]));
    Ok(())
}
```

- [Rust 编程教程](../../../docs/rust-sdk.md)：源码依赖方式、函数分类与可运行示例。
- [用户接口定义](src/client.rs)：`DmsClient`、Options、返回结构、Buffer/View。
- [公开值类型](src/types.rs)：版本、条件、范围、批量字段。
- [完整业务教程示例](examples/tutorial.rs)：SET、条件写、随机写、批量、Merge/Replace、分页与删除。
- [共享内存示例](examples/sdk_shared_memory.rs)：显式分配、提交和只读 View。

`get` 的缺失值是 `Ok(None)`，失败是 `Err(DmsError)`。同步写成功后无需 sleep。当前同步可靠性仅支持本地内存，不能把预留的多副本/落盘策略当成可用功能；详见[能力与限制](../../../docs/product.md)。

SDK 不自行安装宿主的日志后端、Prometheus HTTP listener 或全局 Trace Subscriber。观测由宿主应用决定，见[观测](../../../docs/observability.md)。
