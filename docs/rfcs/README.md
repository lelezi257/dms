# AFS RFCs

RFC 记录影响外部语义、持久格式、跨模块边界、可靠性合同和长期兼容性的设计决策。

## 状态

- `Draft`：内容可修改，不作为实现合同。
- `Accepted`：设计已确认，可以实施。
- `Implemented`：设计已经由代码和验证证据闭合。
- `Superseded`：由新的 RFC 替代。
- `Rejected`：不采用，保留理由供后续查阅。

## 编号

RFC 使用四位编号：`0001-title.md`。编号只表示身份，不表示优先级。

## 索引

| RFC | 标题 | 状态 |
| --- | --- | --- |
| [0001](0001-product-architecture.md) | AFS 产品架构 | Accepted |
| [0002](0002-file-version-chunk-model.md) | FileVersion、Extent 与 Chunk 数据模型 | Accepted |
| [0003](0003-write-visibility-durability.md) | 写入可见性、持久化与版本提交 | Accepted |
| [0004](0004-replication-state-machine.md) | Chunk 单副本与多副本状态机 | Accepted |
| [0005](0005-local-chunk-engine-cow.md) | 本地 ChunkEngine、COW 与崩溃恢复 | Accepted |
| [0006](0006-chunk-transfer-cache-spill.md) | 固定版本 Chunk 的传输、P2P、Cache 与 Spill | Accepted |

新 RFC 使用[模板](0000-template.md)。
