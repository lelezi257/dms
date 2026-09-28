# 专题一：File、Blob、Chunk 统一数据模型

状态：Research  
实现状态：Not Implemented  
专题入口：[架构设计专题](design-topics.md)

## 目标

定义 Mutable File 与 Immutable Blob 如何共享同一 Chunk Store，并确定 Blob 是否直接暴露 API。模型需要同时支持通用 POSIX、多副本可变文件、镜像和 Snapshot 多源读取、本地缓存及外部 spill。

## 已确认约束

- File 是 POSIX Namespace 中的可变对象，支持 overwrite、append 和 truncate。
- Blob 是 Chunk 之上的不可变辅助抽象，不建立第二套数据引擎。
- Blob 在 `seal` 前可构建，`seal` 后内容和长度不可原地修改。
- File 与 Blob 可以共享未修改 Chunk；共享关系必须进入引用和 GC 账本。
- Chunk 是复制、校验、放置和本地存储的基本逻辑单位。
- Stripe 决定一个文件的连续 Chunk 分配到哪一个 replica chain；chain 决定同一 Chunk 的副本集合与写入顺序。
- 一个 chain 可以承载多个文件的多个 Chunk，不与某一个 Chunk 一一绑定。

## 候选层次

```text
POSIX Namespace
├── Mutable File
│   └── FileId + Generation + Logical Range → ChunkRef
└── Published Version
    └── Manifest → BlobRef → ordered ChunkRef

ChunkRef
└── Replica Group / Chain
    └── Storage Target → Physical Position
```

## 核心未决问题

1. Blob 是否提供公开的 `create/append/seal/publish` API，还是只由 Snapshot/镜像集成层使用？
2. 是否同时保留 POSIX 临时文件加原子发布和显式 Blob API？
3. Blob 的默认粒度是整个 layer、大文件、固定 segment，还是允许调用者选择？
4. Mutable File 使用固定 ChunkIndex、Extent Map，还是二者组合？
5. ChunkId 是否包含 FileId/Generation，Published Chunk 是否转为内容寻址身份？
6. Manifest 如何表达稀疏范围、压缩、加密、校验和外部位置？
7. Stripe/placement 如何表达 R=1 本地亲和、R=N 多副本和外部 spill？
8. Chunk、Blob、PublishedVersion 的引用计数与租户配额如何计算？

## 需要比较的 API 方案

| 方案 | 优点 | 主要代价 |
| --- | --- | --- |
| 只提供 POSIX | 应用接入简单 | `seal/publish` 语义需要路径约定或 Runtime API |
| 公开 Blob API | 不可变语义、批量写和校验清晰 | 应用需要适配专用接口 |
| POSIX + Blob API | 兼容性和性能上限兼得 | 两个入口必须共享身份、授权和数据事实源 |

## 预期设计产物

- 对象关系图和持久字段表；
- Blob API 选择及兼容性说明；
- File/Blob/Chunk 身份规则；
- Stripe、chain、replica 和 physical position 术语表；
- 引用、pin、GC 和 spill 的所有权边界；
- 对应 Draft RFC。
