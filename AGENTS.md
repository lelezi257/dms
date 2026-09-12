# DMS 开发协作规则

本文件适用于整个源码 workspace。先读 [README](README.md)、[产品边界](docs/product.md)，再按任务读取 [开发指南](docs/contributing.md) 和目标模块；不要每轮加载所有历史设计。

## 产品与目录

- 用户产品面是 Rust/Go SDK 与 Node/Meta 组件；当前是未发布的开发预览。Go 提供已列明的薄接口子集；正常删除与旧版本回收已有实现，不等于永久失联 Node 的回收问题已解决。Python/C++ SDK、HA、RDMA/UB/L2 不因目录或类型存在就算实现。
- `sdk/` 放语言 SDK；`server/` 放业务进程；`protocol/` 是 wire schema；`common/` 仅放已共享的错误、传输、SHM、观测基础。不要新增万能 runtime/controller/sink。
- `docs/` 是用户/开发文档；`skills/` 是随源码交付的方法。所有可执行工具入口放 `scripts/`；示例放 crate 的 `examples/`，不把 SDK 改成可执行进程。
- 若源码处于完整研究工作区，中间设计/原始证据继续放源码外层；独立检出时先在任务计划中约定产物目录。不要把个人路径、临时日志或凭据放进发布文件。
- README 只用于独立入口、包说明或模块独有的不变量；不为每层目录复制教程。已约定的多语言规划目录可保留简短状态说明，废弃模块的空目录应清理。
- 日志、WAL、PID 等写入显式运行目录，不混入源码；本地 `target/`、`artifacts/`、`.local/` 是产物而非发布输入。清理前检查活跃进程，保留可恢复备份，不自动删除用户运行数据。`.gitignore` 不约束 tar/Cargo 打包，发布仍需核对实际文件清单。

## 不能默默改变的边界

- `NodeState`、`MetaState` 各自是业务状态唯一写 owner。外部等待可离开 actor；状态更新回到 owner；在途工作有界，ACK/心跳不能被提交等待阻塞。
- SDK 的公开输入保持简单；不要把 protobuf/tonic 类型、socket/FD、内部生命周期字段扩散到用户 API。跨进程序列化在 adapter 完成。
- 各语言 SDK 的业务名称与结果语义对齐；只作语言风格、context 和资源关闭差异。Go 的 found 对应 Rust Option，空值不等于缺失。文件系统 adapter 转换范围参数和错误，不改变 SDK 的业务合同。
- Client 保持薄：不跨请求缓存 owned value，普通 GET 请求 Node，由 Node 共享数据/布局。SDK 可复用连接和 Region mmap；普通复制 GET 完成或失败即释放已收到的共享读保护，显式 View 到用户释放才归还。不得把 mmap 索引与 value cache 混为一谈，也不得让旧兼容缓存配置重新启用缓存。
- 用户版本、Extent 映射、Block 身份、Region/Allocation 物理内存分层；具体概念见 [架构](docs/architecture.md)。新增抽象必须用一个实际用户 Case 证明价值。
- 重试同一逻辑写保留 operation id；未知结果不等于未提交。禁止收到网络/Journal 错误就删除已可能被权威版本引用的 bytes。
- Current cache 的回填与命中受 generation/版本/租约约束；断流不等于失效 ACK。不要用 sleep、轮询或提前清除义务伪造写后可见性。
- Node 缓存只保存授权期内的 Current 布局及同次解析的位置提示，截止时间从 resolve 请求开始计算，心跳不延长旧条目。失效先清 Node 再 ACK；本机写同样清理。只检查请求范围所需 Block；缺块可按有效提示拉取，位置失效时按本次固定版本有界回 Meta，不能混拼新旧版本。位置不代表存活证明，完整性校验不能省；布局与位置共同计入预算。
- SHM 只用于可信本地进程。导出过的可写 allocation 不能仅因 TTL 或断连就复用；只有已协商安全归还的新 SDK 明确归还写权后才解除隔离。普通读与显式 View 的连续完成水位必须保护旧 Block，不能跳过尚未结束的较早读。
- 旧 Block 回收先确认无保留版本引用，再通过 Meta 的 Prepare→排空 ACK→Final→释放 ACK 收口。普通 Watch 游标不是回收完成证明；不得因重试超限或会话失效丢弃尚未完成的安全义务。回收元数据、迟到提交与旧 receipt 的 fence 也须有界且可恢复。
- WAL 的兼容记录/字段号不得随清理删掉；恢复、追加、截断失败要有确定状态，不能只改内存而忽略物理残尾。

## 错误与观测

- 子系统 API 返回原生 `DmsError`；内部可有局部错误，在所属子系统出口映射一次。透传根因 code/kind，message 给人看，不用于程序匹配。数字码不重用，未知码原样保留。
- 服务日志用 `dms_logging`；SDK 用宿主 `log` facade，不安装宿主全局后端。保留异步 DropAndReport 和本地日志能力；集中采集失败不能改变业务结果。
- Metrics：公共 Registry + 业务 `metrics.rs` typed handles。`start_*` 管完整生命周期；`record_*` 记录已结束动作；`set_*` 更新快照。标签用有限枚举，不放 key/请求ID/traceID。共享 Registry 的多 Client 不重复注册。
- Trace：SDK 不抢宿主 Subscriber；Node/Meta 在入口初始化。上下文经 gRPC metadata、mailbox envelope 传播，不写入用户 bytes。异步用 `Instrument`/`#[instrument]`，不要持有 enter guard 跨 await。
- Trace off 的成功热路径不产生 Span 日志回退；健康周期操作默认只记 Metrics。日志 level gate 在字段计算前；指标耗时必须能说清测量起止。不要在每个请求扫描全量状态或格式化昂贵字段。
- 日志与 Trace 只记录必要的有界诊断字段，不输出 value、令牌、私钥；trace ID 用关联字段/Exemplar，不作普通指标 label。

## 环境、注释、验证

- macOS 只编辑和阅读；构建、测试、脚本运行与性能实验在 Linux VM/容器。环境先准备，再在其中执行短命令；教程以 [单 VM 手册](docs/local-single-vm-manual.md) 为准。
- 保留 toolchain/Cargo.lock；不混用 Host 与 Linux target 目录。不为文档阶段升级依赖或改变业务语义。
- 默认配置<环境变量<SDK API；Node/Meta 默认<文件<CLI。在线能力按真实接口声明，不能只有内部类型就宣称外部可改。
- 当前中文注释解释职责、所有权/生命周期、关键算法和异常保证。公开函数给出参数、返回值、错误与必要示例，不逐句翻译语法。
- 先有失败回归，再改正确性/重构；至少目标测试、fmt、Clippy。真实进程路径要断言实际 bytes，写成功后第一次读就验，不以探活替代业务 E2E。
- 性能结论给相同输入、环境、样本、p50/p99、开销范围；仿真不证明真硬件性能。未测试就写未测试，不把 package --list 当真正构包。
- SDK 构包由 `scripts/package_sdk.py` 从工作区机械生成；不得手改 staging 包、引入包独有公开 API/feature 或升级锁定依赖。消费者只依赖 `dms-client`；用 `scripts/release/accept.sh` 在无项目源码、无 protoc 的容器验证真实候选仓下载和 TCP/SHM 读写。内部 common 不独立发布给用户。

## 阶段与交接

按 [skills 导航](skills/README.md) 选择轻量方法；普通小修不强制走全套阶段。已明确授权范围内连续执行，已约定的人工门禁仍需停下提交结果。

`main` 是唯一长期开发分支。改动从 `main` 拉短期分支，经 PR 合入 `main` 后删除短期分支；不要再建立长期 `integration/*` 或保留已合入的 `review/*`、`perf/*` 分支。实验身份用 commit SHA 和证据账本固定，不用长期分支充当版本归档。

阶段正文只有一份 Markdown，HTML 自动生成并验证，规则见 [产物契约](skills/dms-review-artifact/references/artifact-contract.md)。不新增 review.md 复制真实设计。完成后记录变更、证据、残余风险、下一入口；发布和不可逆操作必须有单独授权。
