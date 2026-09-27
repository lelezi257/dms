# AFS 当前状态

状态：Implemented Capability Index
更新时间：2026-09-27
详细实验与阶段记录：[status.md](status.md)

## 能力矩阵

| 能力 | 状态 | 当前事实 | 主要缺口 |
| --- | --- | --- | --- |
| `afs-meta` / `afs-node` 进程 | Experimental | CLI/TOML、REST、gRPC、观测和退出已接入 | 生产部署、滚动升级和多 Meta 选主 |
| MetaStore | Experimental | etcd、local-file、memory 共用 Store 提交入口；后端 ACK 后发布可见状态 | 全量状态规模、多活动 Meta、跨进程故障注入 |
| FUSE/VFS | Experimental | Linux 真 FUSE 和 OwnerFs 路径已接通 | 完整 POSIX 兼容矩阵 |
| OwnerFs 本地路径 | Experimental | Home 本地普通文件、根授权和文件操作已接通 | chmod/chown/时间戳、根删除和全局列举等缺口 |
| OwnerFs P2P | Experimental | 三 VM 15/15 功能验收；远端访问同一 Home 文件 | 更广负载、掉电、长稳、异步 RELEASE 回收 |
| OwnerFs 性能 | Experimental | 固定 200×4 KiB、8 worker、12 轮完整 W2 p50 为 MooseFS 的 0.780 | 跨运行稳定性和不同负载不可外推 |
| UDS + SHM SDK 基础 | Experimental | 本地 API、memfd 和 FD passing 已接线 | 正式文件批量异步 API |
| RDMA transport | Experimental | RXE 握手、READ/WRITE 和诊断链通过 | OwnerFs/BlobFs 文件内容路径和硬件吞吐 |
| Distributed BlobFs | Planned | VFS 骨架和 Meta service 位置已存在 | layout、Storage Service、复制和数据路径 |
| Mutable Profile | Planned | 产品和架构合同已定义 | chunk version、写一致性、分布式 POSIX |
| Published Immutable Profile | Planned | 产品和架构合同已定义 | stable cut、COW、manifest、digest、publish |
| 多源 P2P | Research | 独立教学实验验证 consumer-to-seed 过程 | AFS tracker、source selection、限流和产品 E2E |
| 对象存储 spill | Research | 独立 OBS-compatible L2 实验存在 | AFS copy state、外部提交、逐出和 recall |

## 已验证边界

### OwnerFs

- 一个 WorkspaceRoot 具有稳定 Home 和授权身份。
- 本地操作在 Home 的普通文件上执行。
- 远端 Node 通过 P2P 操作同一份 Home 文件。
- close-to-open、旧文件描述符身份、同名重建和跨根 `EXDEV` 已进入三节点功能验收。
- Meta/Home 重启后通过重新打开恢复；运行中的旧 FD 不承诺无感续接。

证据入口：[OwnerFs P2P 并发优化](plans/2026-09-27-ownerfs-p2p-concurrency.md)、[根恢复审视](reviews/ownerfs-v16-root-recovery.md)。

### MetaStore

- 业务命令在私有状态中完成检查和合并。
- 后端确认前不发布新可见状态。
- etcd 使用条件事务，local-file 使用 WAL、同步和排他锁，memory 只用于可丢弃测试。

证据入口：[MetaStore 提交边界](plans/2026-09-27-meta-store.md)。

## 未实现能力

以下能力属于目标架构，不属于当前代码能力：

- 通用 Distributed BlobFs 数据路径；
- 可变 chunk 多副本协议；
- Snapshot 稳定切点与 COW；
- immutable manifest 和发布门禁；
- AFS 内置多源 P2P；
- 自动 spill、逐出和 recall；
- 文件内容 Native Async SDK；
- MicroVM 块设备正式适配；
- 完整 POSIX；
- Meta HA 和生产长稳。

## 代码入口

| 领域 | 入口 |
| --- | --- |
| 进程 | `src/bin/afs-meta.rs`、`src/bin/afs-node.rs` |
| Meta | `src/meta.rs`、`src/meta/store.rs`、`src/meta/owner_roots.rs` |
| Node | `src/node.rs`、`src/node/fuse.rs`、`src/node/vfs.rs` |
| OwnerFs | `src/node/vfs/ownerfs.rs`、`src/node/vfs/ownerfs/` |
| BlobFs 骨架 | `src/node/vfs/blobfs.rs` |
| P2P | `src/node/rpc/peer.rs`、`src/node/rpc/data/owner.rs` |
| SDK | `client/src/`、`src/node/api/local.rs` |
| Transport | `common/transport/` |
| Protocol | `common/protocol/` |
