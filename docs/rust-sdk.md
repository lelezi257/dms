# Rust SDK 编程

## 先运行，再接入自己的工程

先按[单 VM 教程](local-single-vm-manual.md)启动 Meta 和 Node。在 **Linux VM** 内、源码目录执行：

```bash
source scripts/env.sh
export DMS_ENDPOINT=http://127.0.0.1:25200
cargo run --locked -p dms-client --example tutorial
```

成功输出 `DMS tutorial passed`，同时逐项断言 SET/GET、版本读、随机写、批量、Hash Merge/Replace、分页和删除。示例会覆盖 `tutorial/` 前缀下列出的演示 key，末尾删除它们，请勿在这些 key 保存业务数据；删除并不保证立刻回收所有内存。完整代码见 [tutorial.rs](../sdk/rust/dms-client/examples/tutorial.rs)。

接入自己的程序有两种入口：候选包消费者使用 [候选仓库安装](release-installation.md)，共同开发 SDK 时使用下面的源码依赖。**当前没有公共仓库发布版本。** 以下路径需换成实际源码位置：

```toml
[dependencies]
dms-client = { path = "/你的源码目录/sdk/rust/dms-client" }
```

路径依赖允许 Cargo 同时解析工作区内部依赖；不能只复制 SDK 源码目录作为发布包。候选 `.crate` 已将私有实现和预生成协议纳入一个包，消费者只声明 `dms-client`。两种方式的公共 API 相同，应用不直接依赖 generated protobuf API。

## 连接与线程

```rust
let client = DmsClient::connect("http://127.0.0.1:25200", ClientOptions::default())?;
// 或由 DMS_ENDPOINT 与 ClientOptions 共同决定地址：
let client = DmsClient::connect_with_options(ClientOptions::default())?;
```

当前 API 是同步阻塞接口，内部持有 Tokio runtime，不要直接在 Tokio 异步任务中调用；异步宿主应在专用同步线程或 `spawn_blocking` 中创建、使用和释放 Client。`DmsClient::clone()` 共享同一会话和 Region 映射管理，不会创建一个新进程，也不维护私有 value 缓存。

本地地址使用 `unix:///绝对路径/worker.sock`，远程使用 `http://IP:port`。当前公开 `ClientTlsOptions` 仅有 `Disabled`；不要因为连接器识别 `https://` 字符串，就假设 SDK 已有可用 TLS/mTLS 配置入口。参数优先级为默认值 < 环境变量 < API，详见[配置](configuration.md)。

## 1. 基础 KV：成功、缺失、失败是三回事

```rust
client.set("k", b"hello")?;
match client.get("k") {
    Ok(Some(bytes)) => println!("读到 {} 字节", bytes.len()),
    Ok(None) => println!("key 不存在"),
    Err(error) => eprintln!("code={:?} kind={:?}: {}", error.code(), error.kind(), error),
}
let result = client.del("k")?;
println!("本次是否删除已有值：{}", result.deleted);
```

`?` 遇到 `Err` 立即向上返回；不是把失败包装成 `Ok`。`DmsError` 是原生错误结构：数字 `ErrorCode` 用于精确匹配，`ErrorKind` 表示通用类别，message 解释这次失败。未知服务端错误码仍保留原始数字。不要解析 message 决定业务分支，也不要对超时写无条件生成新请求重试：超时不一定代表服务端未提交。

以下函数原型省略泛型细节，`key` 可传 `&str`、`String` 或 bytes；精确定义见 [client.rs](../sdk/rust/dms-client/src/client.rs)。

```rust
set(key, value: &[u8]) -> Result<SetResult, DmsError>
get(key) -> Result<Option<Vec<u8>>, DmsError>
del(key) -> Result<DeleteResult, DmsError>
set_with_options(key, value: &[u8], options: SetOptions) -> Result<SetResult, DmsError>
get_with_options(key, options: GetOptions) -> Result<Option<GetResult>, DmsError>
```

`SetResult` 返回 `version` 和逻辑长度 `len`；`GetResult` 返回选中版本和 bytes。`SetOptions.condition` 可选 `Any / IfAbsent / IfPresent / IfVersion(version)`；条件检查在 Meta 原子完成。可靠性当前只接受 `LocalMemory`，其它枚举值不是已实现承诺。

## 2. 范围与版本

```rust
let original = client.set("k", b"abcdefghij")?;
client.set_range("k", 4, b"X")?;
assert_eq!(client.get("k")?.as_deref(), Some(&b"abcdXfghij"[..]));
let old = client.get_with_options("k", GetOptions {
    version: ReadVersion::Exact(original.version),
    range: Some(ByteRange { offset: 3, len: 4 }),
})?.ok_or("历史版本不存在")?;
assert_eq!(old.bytes, b"defg");
```

范围是 `[offset, offset + len)`。历史版本可能受保留策略影响，不承诺永久可读。`set_range` 要求 key 存在且修改区间不超出当前长度；只提交 patch 与新布局，不复制整个 base value。并发修改可能返回版本冲突。

```rust
set_range(key, offset: u64, data: &[u8]) -> Result<SetResult, DmsError>
set_range_with_options(key, offset: u64, data: &[u8], options: RangeWriteOptions)
    -> Result<SetResult, DmsError>
```

`RangeWriteOptions.expected_version` 可显式锁定 base version；省略时由 Node 绑定当次 Current，仍以该版本提交 CAS。

## 3. 多 key 与 Hash/KKV

```rust
mset(entries: &[KvEntry], options: MSetOptions) -> Result<MSetResult, DmsError>
mget(keys) -> Result<Vec<Option<GetResult>>, DmsError>
hset(key, entries: &[HashEntry], options: HashWriteOptions) -> Result<HashSetResult, DmsError>
hget(key, field) -> Result<Option<HashValue>, DmsError>
```

MSET 是独立 key 的原子发布；MGET 按输入顺序返回，缺失项为 `None`，但不是跨 key 快照。

KKV 是 `主 key → field → bytes`。例如 `job → model → A` 与 `job → config → B` 共用一个主 key。Hash 表示这种字段集合，不是按哈希值查询：

```rust
client.hset("job", &[
    HashEntry::new("model", b"A".to_vec())?,
    HashEntry::new("config", b"B".to_vec())?,
], HashWriteOptions::default())?; // 默认 Merge

client.hset("job", &[HashEntry::new("model", b"C".to_vec())?],
    HashWriteOptions { mode: HashWriteMode::Replace, ..Default::default() })?;
assert!(client.hget("job", "config")?.is_none()); // 未指定字段被删除
```

同一次 HSET 的字段一起发布为一个 HashVersion。`expected_version` 对整个字段表做 CAS。不要对普通 bytes key 使用 Hash 方法，也不要把 Hash 编码当成普通 value 修改。

| 方法 | 用途 / 主要返回值 |
| --- | --- |
| `hget_with_options(key, field, HashGetOptions)` | 读取指定字段表版本。 |
| `hmget(key, fields, HashGetOptions)` | 一个字段表版本的多个字段，按请求顺序返回 `values`。 |
| `hgetall(key, HashGetOptions)` | 一次读取有界字段集合，返回 `entries`。 |
| `hdel(key, fields, HashDeleteOptions)` | 删除指定字段并发布新字段表版本。 |
| `hscan(key, ScanCursor, HashScanOptions)` | 按页返回 `entries/version/next_cursor`，cursor 为 0 表示完成。 |
| `hwrite_at(key, field, offset, data, HashRangeWriteOptions)` | 修改已有字段 value 内部一段 bytes；返回新 Hash/value 版本。 |

HSCAN 首次传 `ScanCursor(0)`，后续传返回的 cursor；要跨页保持一致，应把首个返回版本放入后续 `HashScanOptions.version = HashReadVersion::Exact(version)`，而不是每页重新读 Current。可运行示例展示完整循环。`set_range` 修改普通 key 的 value，`hwrite_at` 修改主 key 下一个 field 的 value；后者当前仍重编码字段表，不能推断性能等同。

## 4. 共享写 Buffer 与只读 View

普通 `set/get` 也能通过 UDS 工作；小 SET 仍可内联，普通 GET 仍返回复制后的 `Vec`。只有想显式减少复制时，才使用下列接口，且 ClientOptions 需 `shared_memory: Some(true)`：

```rust
let mut buffer = client.allocate_write("shm/k", 5)?;
buffer.as_mut_slice()?.copy_from_slice(b"hello");
client.commit_shared(buffer)?; // 消耗 buffer，之后不能再拿它改写

let view = client.get_view("shm/k")?.ok_or("missing")?;
assert_eq!(view.as_slice()?, b"hello");
drop(view); // 结束本次只读借用，不等于保证物理 Block 已回收
```

SDK 内部取得 Node Region 的 FD，mmap 后按 offset/length 定位；应用不操作 FD。View 持有映射生命周期，不能把它当成永久指针。当前只支持单个 SHM segment；TCP 连接或多 segment 布局会明确报不支持，不会假装零拷贝返回 Vec。

运行 [sdk_shared_memory.rs](../sdk/rust/dms-client/examples/sdk_shared_memory.rs) 前，把 `DMS_ENDPOINT` 改为本次 Node 的 UDS 地址；详见[单 VM 教程](local-single-vm-manual.md)。关于隔离容量和回收限制，必须先读[产品边界](product.md)。

## 观测与下一步

SDK 默认不创建日志文件、不监听 `/metrics`、不安装全局 Trace Subscriber。可注入宿主 `MetricsRegistry`；日志交给宿主 `log` 后端；Trace 交给宿主 Subscriber。完整接线见[观测](observability.md)，不要为此给 SET/GET 用户参数添加 trace ID。
