# 能力与限制

## 当前解决什么问题

应用把中间结果、模型数据或检查点内容写入计算节点内存，其它应用在本节点或其它节点读取。DMS 提供字节对象与版本语义，不提供文件挂载、目录、inode 或 POSIX 接口。

默认先写接入 Node 的内存。Meta 记录 key 的版本、组成数据块和所在 Node；Reader 不需要自己维护位置表。当前部署是一台 Meta 配若干 Node，单 VM 可承载多个独立进程。

## 已实现的入口

| 能力 | 用户可观察到的行为 |
| --- | --- |
| SET / GET / DEL | 发布完整 value，读取当前版本，以删除版本隐藏当前值；重复删除可安全返回未删除。 |
| 条件写与历史读 | 指定不存在/已存在/版本条件；可读仍被保留的历史版本。 |
| 范围读 / SET_RANGE | 读取一段 bytes，或修改已有 value 内的一段；随机写不能扩展长度。 |
| MSET / MGET | MSET 在 Meta 中原子发布多个 key；MGET 保持输入顺序，但不承诺跨 key 同一快照。 |
| Hash / KKV | 一个主 key 下多个 field；Merge 保留未指定字段，Replace 删除未指定字段。 |
| 本地共享内存 | UDS 协商、FD 传递、mmap；普通接口可用，显式 buffer/view 接口可减少复制。 |
| 跨 Node 读取 | 接入 Node 根据位置从 Peer 拉取缺失 Block；无需把用户数据经 Meta 中转。 |
| 元数据恢复 | 内存 Journal 或本地 WAL/snapshot；后者恢复元数据，不等于恢复内存 value。 |
| 观测 | 服务日志、Prometheus 指标、可选分布式 Trace；SDK 遵循宿主观测配置。 |

key 与 field 是二进制安全的 1～1024 字节；普通方法直接接收字符串或字节，不必先构造 Key。普通 SET 不接受空 value。Hash 操作适合有界字段集，当前字段表编码包含字段值，不能当成超大数据集的低成本索引。

## 一致性与缓存

SET 成功表示新逻辑版本已经提交，并完成相关旧 Current 缓存的失效协调；此后发起的 GET 不需要 `sleep` 才能看到更新。并发读写仍可能按先后顺序读到旧版或新版，但一次读取绑定一个版本，不拼接两个版本的 bytes。

TCP Client 可缓存自有 bytes。命中必须同时满足缓存代次、版本与租约条件；失效通知、断流或租约过期会禁止旧缓存继续命中。默认缓存预算 64 MiB，不是整个进程 RSS 上限。SHM 模式不再维护一份重复的 owned bytes 缓存，而复用已映射 Region。

Node 另有默认 8 MiB 的 Current 布局缓存，只保存元数据，不复制 value；有效授权、Watch 和本地 Block 完整性都满足时，GET 可省掉 Meta 查询。默认 TTL 上限1秒；失效立即撤销资格，不是允许读旧值1秒。可配置关闭，旧 Meta 未授予资格时不启用。

为保证失联 Reader 不能永久使用旧值，写入可能等待 ACK 或租约到期。因此故障和并发情况下尾延迟会增加；不要把单 VM 中位延迟当成上限。

## 必须接受的开发预览边界

- **数据可靠性仅支持 `LocalMemory`。** `MemoryCopies`、`LocalDisk`、`ObjectStore` 虽在类型中预留，当前写请求会明确拒绝，不会自动降级。Node 重启或唯一副本丢失可能使 value 无法读取。
- **不是 HA 集群。** 多 Meta 复制/选主没有完成；本地 Meta WAL 不能替代数据副本，也不能保证所有介质故障可恢复。
- **完整长期内存回收未完成。** 已导出的 SHM allocation 会隔离并继续计入容量，不因 TTL 过期就复用。ViewEpoch 已记录生命周期，但尚未驱动完整旧 Block 回收；持续写入可能耗尽容量。
- **共享内存只适用于受信任本地应用。** 把 Region 的 FD 交给进程，不是只授权其中一个 offset 的强安全沙箱。当前不承诺恶意租户隔离。
- **只有 Host memory、gRPC/TCP 与本地 SHM 实现。** RDMA/UB、设备内存、磁盘/对象存储分层、多语言 SDK 是扩展方向，不是当前可运行功能。
- **候选制品不等于正式发布。** 提供源码与本地候选包，未上传公共仓库，未承诺稳定 ABI、跨版本滚动升级或千节点性能。候选安装入口见 [release-installation.md](release-installation.md)。

操作说明见[单 VM 教程](local-single-vm-manual.md)，实现分工见[架构](architecture.md)。
