# 专题三：单副本与多副本写入状态机

状态：Research
实现状态：Not Implemented
专题入口：[架构设计专题](design-topics.md)

## 目标

以单副本本地写为基础，定义可扩展到同步多副本、异步副本和重配置的统一 Chunk 写入协议。多副本不建立独立存储引擎。

## 已确认方向

```text
R=1:
prepare local update → local commit → return

R=N:
prepare local update → replicate → commit according to policy → return
```

- R=1 优先写当前计算节点的本地磁盘；本地亲和需要成为 placement 合同。
- R=N 需要明确 write-all、quorum 或其他确认规则；读取来源必须与提交规则匹配。
- pending 数据不能计入已提交可靠性，也不能成为普通读取或 P2P seed。
- chain/version 变更必须隔离旧请求和新配置。

## 必须分析的状态与故障

- head、middle、tail 或任意 replica 在 update/ACK/commit 前后退出；
- 响应丢失后的安全重试；
- 已提交版本与 pending 版本并存；
- 故障副本移出、SYNCING 副本加入和 repair；
- R=1 节点故障后的可用性边界；
- Local+AsyncReplica 中本地提交与异步任务登记的原子性；
- 跨多个 Chunk 的 POSIX write 部分成功；
- placement epoch、chain version 和 chunk version 的 fencing。

## 参考模型

3FS 提供 per-chunk head→tail update、tail→head commit、版本化 chain、重试幂等和 SYNCING 副本 materialize 的白盒参考。AFS 需要验证该模型与 R=1 本地优先、普通网络、P2P seed 和 spill 的适配关系，不能直接复制实现。

## 预期设计产物

- R=1/R=N/异步副本状态图；
- 写入、读取和 repair 的版本规则；
- 节点退出和 chain 重配置故障矩阵；
- OperationId、版本和持久去重模型；
- 对应 Draft RFC 与故障注入计划。
