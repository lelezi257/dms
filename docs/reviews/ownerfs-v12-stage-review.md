# OwnerFs 当前阶段复验（2026-09-26）

**结论：** 既定 OwnerFs 功能门槛与本地 W1 性能门槛通过。旧版 W2 的近 MooseFS 性能尚未恢复；当前 gRPC/mTLS 远端路径为 MooseFS 的 1.94 倍，应继续作为明确的性能缺口，而非同事试用版的已完成收益。范围只含 OwnerFs；所有构建、功能与性能实测在 Linux VM，没有 push、merge 或 release。

| 项目 | 当前结果 | 原始证据 |
| --- | --- | --- |
| A/B/C 三节点功能 | 隔离集群 15/15 步通过、0 清理错误；并发建根 A 成功、B 收到 `EEXIST`；Meta/Home 进程重启后重新打开恢复 | [result-v12-isolated.json](../../../../experiments/results/2026-09-26-afs-ownerfs-acceptance/result-v12-isolated.json) |
| W1 本地优势场景 | 新建且未被 B 访问的根，两份独立会话各六轮；OwnerFs/MooseFS p50 为 **0.3796 / 0.3794**，均低于合同 0.80 | [result-v12-private.json](../../../../experiments/results/2026-09-26-afs-ownerfs-w1/result-v12-private.json) |
| W2 跨节点诊断 | 六轮内容正确；OwnerFs 642.5 ms、MooseFS 330.5 ms，p50 比值 **1.944**；没有 W2 数值通过门槛 | [result-v11.json](../../../../experiments/results/2026-09-26-afs-ownerfs-w2/result-v11.json) |

W1 第一/第二会话 p50（ms）：OwnerFs **253.0/250.0**，MooseFS **666.5/659.0**，薄 FUSE **197.0/198.9**，Native FS **113.0/117.9**。同场报告 p95/p99 保留在原始 JSON；每会话只有六轮，p99 是最近秩最大样本，不宜当长尾稳定性结论。新 W1 使用预建根；历史旧版 0.375/0.381 使用每轮创建一级根，两个比值数字相近但**不是严格同一 workload**。历史旧版预建根诊断约 225 ms，是更接近的绝对耗时参照。

W2 各阶段 p50（OwnerFs/MooseFS，ms）：B 初读 **164/82**、B 重读 **161/37**、B 覆盖写 **289/147**、A 回读 **29/62**。每阶段 200 个 4 KiB 文件。已在原有 `OwnerFs`、`OwnerFiles`、FUSE 与 Node peer 边界内平移旧版小文件 `open` 预取、只读关闭不等待响应、已同步写不重复 flush、本地私有根 1 秒 FUSE 缓存及首次远端访问前失效；W2 从 v8 的 5.56 倍降到 1.94 倍。旧版远端目录项 1 秒 TTL 与裸 TCP 会话不能直接搬进当前 gRPC/mTLS 路径：前者在 A 删除重建后可能使 B 以旧 inode 打开并返回 `ESTALE`，后者改变已定传输与认证合同。后续先量化每文件 RPC/耗时，再设计可验证的失效或复合操作；不能以旧读换取 W2 数字。

本轮还修复了 catalog 临时文件名随根名和多份 session 拼接，17 字节根名即可超过 Linux 255 字节单组件限制的问题；`bench-private-v12` 真实建根和定向测试通过。OwnerFs 专用 RPC 编译条件现与 `ownerfs` feature 一致，BlobFs-only、OwnerFs-only、全功能编译通过。并发建根验收脚本现要求恰好一个成功者，避免已有根导致双方失败仍误判通过。

**接口/目录偏离清单：** 没有新增 VFS、Storage 或传输核心层，也没有改已确认的 VFS trait。`node_data.proto` 的 `OwnerOpenReply` 增加可选小文件预取字段；`fuse.rs` 增加生产挂载的 OwnerFs 缓存 hook；`rpc/peer.rs` 增加接收 Node Tokio runtime 的构造函数以支持只读异步关闭。这三处是旧快路径在现有边界上的合同增量，均以 Linux 三节点和编译矩阵验证。历史 v8 设计偏离见 [v8 阶段验收](ownerfs-v8-stage-review.md)。

仍未验收完整 POSIX、根删除重建、VM 掉电与长稳；`/ownerfs` 根列表不能主动列出别的 Node 刚创建的根；OwnerFiles 的 RDMA 数据路径尚未完成。以上不应写成已经可替代 MooseFS 的通用完整功能。
