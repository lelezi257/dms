# 性能诊断账本

这个文件只在需要重新做白盒归因时读取。它固定“先看哪一段”，不保存临时实验日志。

## 读取路径

| 阶段 | 要回答的问题 | 证据 |
| --- | --- | --- |
| Adapter/FUSE 外壳 | 是否多了一次 Stat/Get/ReadAll，范围读拆分是否与对照一致 | Adapter 调用计数、文件层 p50 减对应 SDK p50 |
| Client→Node 控制 | 是否只有合同中的一次 Get、SetInline 或 Allocate+Set | gRPC metrics/trace 与 service 计数 |
| Meta | 热读是否仍 Resolve；写是否只有一次 Commit；Peer 首读是否只有一次 Resolve | Meta 方法计数与 p50 |
| Payload | SHM 是否没有 Upload RPC；Peer 是否只 Pull 一次；完整 Block 是否复用提交摘要 | provider 计数、bytes、摘要原语基准 |
| 本地交付 | 普通 GET 是否只有一次到用户 buffer 的必要复制 | `get_into`/SHM 复制原语 |
| 残差 | 端到端减去独立分段后还剩多少 | 同轮样本的算术残差；超过 10% 就继续测 |

## 先判断路径类型

- `local_hot`：数据和 Current 已在本地 Node；目标是一次 Client→Node 和一次必要交付复制。
- `advantaged`：架构应减少远端搬运或复制；必须比同环境对照快。
- `architecture_penalty`：首次建立本地位置/副本需要额外阶段；与组成下界比较，并同时报告下一次热读。

## 不重复探索的已确认结论

- Client 保持薄，不用 SDK 私有 bytes cache 掩盖 Node 路径；SHM mapping registry 不是对象缓存。
- Peer 首读的最小前台图是 Get + ResolveObject + PullBlock；ReportReplicas 在后台最终完成。
- 本地热读不访问 Meta；若出现 ResolveObject，优先判断 Current cache 失效、租约或实现回归。
- 1 MiB SHM SET 的 AllocateStaging 和 Set 是“取得 Slot”与“宣布写完”两个语义，不应仅为减少 RPC 强行合并。
- 完整不可变 Block 可复用提交摘要；区间 payload 必须重新校验。
- jemalloc 不替代 Region 内 Slot 管理；allocator 只有被分段证据证明为主成本时才进入优化优先级。
