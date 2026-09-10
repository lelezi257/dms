# 开发指南

先看 [产品边界](product.md) 和 [架构](architecture.md)。本项目当前按开发预览维护：正常删除与旧版本物理回收已有实现，但不承诺永久失联 Node 的完整回收、生产 HA 或已发布的 SDK 包。

## 1. 环境与日常检查

按 [单 VM 手册](local-single-vm-manual.md) 进入 Linux 并加载 `scripts/env.sh`。在源码根执行：

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --all-targets --locked
cargo test -p dms-shm --doc --locked
```

可先测目标 crate；交付涉及多个模块时再跑 workspace。`CARGO_INCREMENTAL=0` 可降低反复构建的缓存占用，不是功能配置。不要删 runtime journal 来解决编译空间问题。

用户 API 文档可在 VM 生成：

```bash
cargo doc -p dms-client --no-deps --locked
```

结果在 `$CARGO_TARGET_DIR/doc/dms_client/index.html`。这是当前源码文档，不代表中心仓已经存在该版本。

## 2. 一个改动放在哪里

| 要改什么 | 主入口 | 不应该顺带做什么 |
| --- | --- | --- |
| 用户 SET/GET 等 API | `sdk/rust/dms-client/src/client.rs`，类型见 `types.rs`，导出见 `lib.rs` | 让调用者依赖 protobuf DTO |
| 节点对象流程与物理内存 | `server/src/node/` | 把外部 RPC 等待堵在唯一状态 owner 内 |
| 版本/位置、提交和恢复 | `server/src/meta/` | 为一次 CAS 新建通用业务框架 |
| wire 消息与服务 | `protocol/proto/dms/v1/` | 重用删掉的字段号，或手改生成代码 |
| TCP/共享内存/错误/观测机制 | `common/` 对应 crate | 把 key 业务规则放进公共机制 |
| 编译部署/验收工具 | `scripts/` | 假造成功、停止别人已有服务 |

公开协议变更需同时检查双方 adapter 和未知值兼容；不同传输路径应通过同样的内容/错误行为测试。示例位于 SDK `examples/`，不要给 SDK library 添加业务 `main`。

## 3. 写代码时直接遵守的基础约定

**错误**：子系统出口映射为原生 `DmsError { code, kind, message }`，内部不要求每个函数都使用它。程序比较 `ErrorCode` 常量；日志显示数值与说明；不要重包装每一层来丢掉根因。数字目录与原生定义见 `common/error/src/`；wire 结构见 `protocol/proto/dms/v1/types.proto`，沿服务 adapter 查转换。

**日志**：服务代码用 `dms_logging::info!/warn!/error!`，SDK 用 `log::...!`。日志表示具体事件，不代替错误返回；避免每层都重复记录同一次失败。初始化、滚动、保留、采集见 [观测指南](observability.md)。不要在热路径预先构造一段昂贵字符串再交给关闭的日志宏。

**Metrics**：在业务 `metrics.rs` 注册有限标签的 typed handles。请求起点 `start_*` 创建生命周期 guard，结束统一记耗时/结果并释放 inflight；已经完成的传输用 `record_*` 一次更新请求数、bytes、耗时；状态快照用 `set_*`。这三种语义不是三套框架。记录点要说明“谁的耗时，从哪里到哪里”。

**Trace**：同步作用域与异步 Instrument 都可以，不必为打点包一层无意义的 async。创建有界具体操作 Span；跨任务显式带上下文，跨进程注入/提取 metadata。`trace_id` 自动关联日志和 Exemplar，不能加入业务 API 或 value。SDK 不初始化宿主全局 Subscriber。

这些约定的强约束在 [AGENTS.md](../AGENTS.md)，机制事实在现有源码；不要另写一份几乎相同的“基础框架设计”。

## 4. 什么证据算完成

- 单测断言输入→状态→输出，包括失败路径；结构存在、进程 ready、没有报错都不是业务正确性证明。
- 网络验证要用真实 Client/Node/Meta；SHM 验证 range bytes、FD/mmap 生命周期、旧句柄不能误用。故障发生要有证据。
- 重构前固定基线；性能比较同时写 p50/p99 和未测项，不能只报有利指标。内存预算不等于进程 RSS，timeout 不等于物理共享页已可复用。
- 不确定提交先保留数据并查结果；测试不能通过重复写、sleep 或轮询遮蔽写后第一次读错误。
- 原始失败输出保留；修复后新证据不能覆盖第一次失败。不同实验的 warmup、样本、传输能力不能混写。

GitHub 的 [DMS CI](https://github.com/lelezi257/dms/actions/workflows/ci.yml) 在 main 推送、PR 和手工触发时运行：Ubuntu 24.04 + `rust-toolchain.toml` 指定的工具链，执行同一个 `scripts/release/check.sh`（fmt、Clippy、workspace tests/doctests、Python 工具测试）。工作流只有源码读取权限，不上传制品、创建 tag 或发布版本；本地也可在 Linux 源码根执行该脚本。

CI 的源码门禁不能替代候选包隔离安装、三节点或性能验收。当前尚未发布 GitHub Release / crates.io 包，发布验收另行推进。

## 5. 人工与 AI 的接力

 [项目 skills](../skills/README.md) 提供需求、设计、穿刺、开发和验收模板；只读取当前阶段。新增抽象先从用户操作推导，明确状态 owner、接口方向和一条异常路径；图、名字、原型保持一致。

使用哪个模型、是否并行由任务决定，不把特定工具或模型名称写成项目必需依赖。阶段报告给少量 review 决策，不让 reviewer 重读所有历史材料。
