# 能力与限制

## 当前解决什么问题

应用把中间结果、模型数据或检查点内容写入计算节点内存，其它应用在本节点或其它节点读取。DMS 提供字节对象与版本语义，不提供文件挂载、目录、inode 或 POSIX 接口。

默认先写接入 Node 的内存。Meta 记录 key 的版本、组成数据块和所在 Node；Reader 不需要自己维护位置表。当前部署是一台 Meta 配若干 Node，单 VM 可承载多个独立进程。

## 已实现的入口

| 能力 | 用户可观察到的行为 |
| --- | --- |
| SET / GET / DEL | 发布完整 value，读取当前版本，以删除版本隐藏当前值；重复删除可安全返回未删除。 |
| STAT / SCAN | 查询对象长度、版本、稳定修改时间；按前缀分页列举当前有效 key。 |
| 条件写与历史读 | 指定不存在/已存在/版本条件；可读仍被保留的历史版本。 |
| 范围读 / SET_RANGE | 读取一段 bytes，或修改已有 value 内的一段；随机写不能扩展长度。 |
| MSET / MGET | MSET 在 Meta 中原子发布多个 key；MGET 保持输入顺序，但不承诺跨 key 同一快照。 |
| Hash / KKV | 一个主 key 下多个 field；Merge 保留未指定字段，Replace 删除未指定字段。 |
| 本地共享内存 | UDS 协商、FD 传递、mmap；普通接口可用，显式 buffer/view 接口可减少复制。 |
| 跨 Node 读取 | 接入 Node 根据位置从 Peer 拉取缺失 Block；无需把用户数据经 Meta 中转。 |
| 元数据恢复 | 内存 Journal 或本地 WAL/snapshot；后者恢复元数据，不等于恢复内存 value。 |
| 观测 | 服务日志、Prometheus 指标、可选分布式 Trace；SDK 遵循宿主观测配置。 |

key 与 field 是二进制安全的 1～1024 字节；普通方法直接接收字符串或字节，不必先构造 Key。普通 SET 支持空 value；GET 命中的空字节与 key 不存在是不同结果。Hash 操作适合有界字段集，当前字段表编码包含字段值，不能当成超大数据集的低成本索引。

Go SDK 当前提供 Connect/Set/Get/Del/Stat/Scan 及必要选项，不等于已实现 Rust SDK 的全部高级方法。文件系统接入仅通过 JuiceFS 对象后端适配器，目录、inode、文件布局仍由文件系统管理；详见 [接入与部署](juicefs.md)。

## 一致性与缓存

SET 成功表示新逻辑版本已经提交，并完成相关旧 Current 缓存的失效协调；此后发起的 GET 不需要 `sleep` 才能看到更新。并发读写仍可能按先后顺序读到旧版或新版，但一次读取绑定一个版本，不拼接两个版本的 bytes。

SDK 不维护跨请求的 value 缓存，TCP/SHM 普通 GET 都请求 Node。TCP 接收 bytes；SHM 复用已映射 Region，再复制成用户自己的 `Vec<u8>`。普通 GET 复制完成后结束本次共享读借用；显式 `get_view` 才保护数据直到用户释放 View。mmap 复用不是一份隐藏的 value 缓存。旧 `current_cache_bytes` 配置仍可解析，但不影响运行行为。

Node 有默认 8 MiB 的 Current 元数据缓存，保存布局与同次解析的位置提示，不复制 value。授权和 Watch 有效时，所需 Block 本地命中便直接读取；缺块可按提示拉取并保存在 Node 中，供其它 Client 复用，位置失效则按固定版本刷新。默认 TTL 上限1秒，从解析请求发起时算起，不从下载结束重新计时；失效立即撤销资格，不是允许读旧值1秒。可配置关闭，旧 Meta 未授予资格时不启用。

写入仍需完成 Node Current 缓存失效协调；兼容旧 SDK 时也不能绕过已经授予的缓存租约义务。新薄 SDK 不再申请 value 缓存租约，但保留 Session 心跳和活动 View 保护。因此故障和并发情况下尾延迟仍可能增加；不要把单 VM 中位延迟当成上限。

## 必须接受的开发预览边界

- **数据可靠性仅支持 `LocalMemory`。** `MemoryCopies`、`LocalDisk`、`ObjectStore` 虽在类型中预留，当前写请求会明确拒绝，不会自动降级。Node 重启或唯一副本丢失可能使 value 无法读取。
- **不是 HA 集群。** 多 Meta 复制/选主没有完成；本地 Meta WAL 不能替代数据副本，也不能保证所有介质故障可恢复。
- **回收不是删除后立即释放。** 当前候选已实现旧版本裁剪、排空在途读、回收通知和物理 allocation 释放；完整文件系统 GC 与故障验收仍以候选报告为准。活对象、保留版本及未归还的读写借用继续占用容量。旧 SDK 或丢失写权归还的 SHM allocation 保持隔离，不能因 TTL/断连复用；这类未归还资源仍可能耗尽容量。
- **共享内存只适用于受信任本地应用。** 把 Region 的 FD 交给进程，不是只授权其中一个 offset 的强安全沙箱。当前不承诺恶意租户隔离。
- **只有 Host memory、gRPC/TCP 与本地 SHM 实现。** RDMA/UB、设备内存、磁盘/对象存储分层、Python/C++ SDK 是扩展方向，不是当前可运行功能。
- **候选制品不等于正式发布。** 公开源码与开发候选包不代表已发布 GitHub Release / crates.io 包，未承诺稳定 ABI、跨版本滚动升级或千节点性能。候选安装入口见 [release-installation.md](release-installation.md)。

操作说明见[单 VM 教程](local-single-vm-manual.md)，实现分工见[架构](architecture.md)。
