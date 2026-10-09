# Meta 与事务边界

Meta 是文件系统可恢复状态的权威，不代理稳定数据流。它负责命名空间、inode、写租约、文件版本、布局、放置、副本目录和幂等提交结果。

## 职责

| 范围 | 内容 |
| --- | --- |
| 命名空间 | dentry、inode 身份、属性 |
| 写权威 | 写租约、owner epoch、fence |
| 版本权威 | `FileVersion`、`LayoutRoot`、inode `head_version` CAS |
| 放置 | 放置快照、设备 epoch、故障域 |
| 副本目录 | 持久副本、验证缓存、外部已提交副本和生命周期 |
| 幂等 | 重试提交时保留精确操作结果 |

## 提交模型

Meta 先用已提交状态校验请求，再准备候选状态；只有持久后端接受该状态后才发布。若后端写结果未知，请求身份就是恢复边界，不能把未知写冒充成功。

当前基础设计允许 Meta 在 store 接口之上保存完整 committed snapshot 并批量更新。底层后端第一版不必暴露原生多记录事务；记录级存储只是后续实现细节。

## 后端顺序

当前主线只要求中心 `local-file` Meta 能重启恢复；`memory` 是一次性演示后端，重启后不保留状态。etcd 和 Redis 后端都后置：etcd 是资源和可靠性专题，Redis 优先级最低。

Meta 多实例选主、跨实例 fence 和 HA 需要独立协议，不是当前收尾前置条件。

## 读视图

复合读可以通过 `MetaReadView` 固定一个已提交视图，再解析文件版本、布局、inode 和 chunk 来源。查询过程中 live state 即使推进，也不能混用多个 revision。该一致性是 Meta 语义，不要求底层 store 一定提供原生多记录读事务。

当前能力和验收状态见 [当前计划](../development/plan.md) 与 [状态摘要](../status.md)。
