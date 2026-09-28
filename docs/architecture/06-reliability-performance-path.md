# 专题六：可靠性与高性能数据路径

状态：Research
实现状态：Not Implemented
专题入口：[架构设计专题](design-topics.md)

## 目标

定义不会绕过授权、版本和校验合同的高性能数据路径，并覆盖 RPC 重试、进程重启、端到端校验、多源读取、容量 spill 和降级传输。

## 可靠性主题

- OperationId、channel/sequence 和持久 chunk version 的分工；
- 响应丢失、Node 重启和 Meta 重启后的去重；
- routing/layout/placement epoch fencing；
- client、每个 replica、cache seed 和 external copy 的 checksum；
- scrub、repair、rebalance、drain 和校验失败隔离；
- durable replica、verified cache、P2P seed 和 external committed 的状态转换；
- spill、逐出、recall 和外部存储不可用时的行为。

## 数据路径原则

```text
Small payload:
control message + inline bytes

Large payload:
control message + PayloadDescriptor
  ├── Shared Memory / memfd
  ├── Registered Buffer / RDMA
  ├── Local Buffer
  └── Streamed Network Body
```

- 同节点优先共享内存或本地注册 buffer；
- 跨节点按能力选择 RDMA 或普通网络；
- transport adapter 不改变文件身份、提交语义和 checksum；
- Native SDK、FUSE 和 Block Adapter 在 Chunk 层汇合；
- 固定 FileVersion 的数据可从属于该版本的 durable replica、verified cache 和 seed 并行读取；
- 活动文件读取先固定一致性协议确认的 FileVersion，再选择合格来源。

## 预期设计产物

- 幂等与版本 fencing 合同；
- PayloadDescriptor 和 completion 接口；
- inline/SHM/RDMA/streaming 选择规则；
- 多源读取、seed 晋升和校验失败状态机；
- spill/recall/逐出门禁；
- 功能、故障和性能验收矩阵；
- 对应 Draft RFC。
