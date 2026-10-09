# 数据模型

AFS 把文件内容、命名空间和副本信息拆开管理。Meta 保存权威元数据；Node 保存或搬运文件字节。

## 核心对象

| 对象 | 含义 |
| --- | --- |
| inode | 文件或目录的稳定身份、属性和当前版本指针 |
| dentry | 父目录到名称再到 inode 的映射 |
| `FileVersion` | 一次已提交文件内容的不可变版本 |
| `LayoutRoot` | 文件版本到 chunk 范围的布局根 |
| chunk | 已提交数据的不可变字节对象 |
| copy record | 某个 chunk 在节点、本地盘、验证缓存或外部 spill 上的可用副本 |

## 稀疏文件

布局可以表达洞和数据范围。读洞时返回零；写入洞会产生新的 dirty 范围，并在提交时进入新版本布局。truncate、punch hole 和覆盖写必须更新布局视图，不能让旧 chunk 被错误解释为新内容。

## 版本关系

普通写不会立刻产生 `FileVersion`。sync、同步写、close-time flush 或后台提交成功后，Meta 通过 CAS 推进 inode 的 `head_version`。读取固定版本时必须使用同一个布局和长度视图。

## 当前状态

该页描述目标模型。哪些字段已经实现、哪些场景已验收，以 [当前计划](../development/plan.md) 为准。
