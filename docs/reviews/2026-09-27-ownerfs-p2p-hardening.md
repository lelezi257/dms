# OwnerFs P2P 句柄边界修复与最终复验

## 结论

**事实：** 在原有 VFS → OwnerFs → P2P、Meta 粗粒度根授权、Home 普通文件的分层内，已补齐三处局部边界：带 `fh` 的 `getattr/setattr` 与文件 I/O 按句柄保序；Home 将打开句柄绑定到实际根、peer 会话和授权；异步 RELEASE 短暂失败会有界重试，peer 会话过期后 Home 后台回收遗留句柄及授权缓存。没有新增核心模块、Proto 或改变 VFS/Storage 接口。

**事实：** 最终 Linux Release 二进制 SHA-256：`afs-node` 为 `cc3d46d800f60996…`，`afs-meta` 为 `d5330f44ca8a981c…`，A/B/C 三 VM 一致。全特性测试、严格 Clippy、格式与 diff 检查通过；三 VM 功能验收 **15/15**，无失败、缺口或清理错误。B 持有远端 FD 时 `SIGKILL` B daemon，Home 后续记录 `ownerfs.peer_handles_reaped count=1`。

| 同一最终二进制、Linux 三 VM | OwnerFs / MooseFS |
| --- | ---: |
| W1，本机私有根，两份 6 轮 | **0.482 / 0.451** |
| 完整 W2，200×4 KiB、8 worker，两份 12 轮 | **0.768 / 0.780** |
| 顺序 W2，200×4 KiB、1 worker，两份 6 轮 | **1.106 / 1.080** |

W1 同场本机 Native FS 为 147/144 ms、薄 FUSE 为 234/235 ms、OwnerFs 为 342/316 ms、MooseFS 为 709/701 ms。**推断：** 固定并发 W2 的总量优势仍主要来自 A 的 Home 本机回读；B 的重读和覆盖写仍慢于 MooseFS。顺序 W2 一次略高于原 1.10 参考线、一次低于，不能宣称顺序路径稳定领先或稳定满足 1.10。

## 修复边界

- `src/node/fuse.rs`：带打开句柄的属性操作进入现有 per-fh dispatcher；确定性队列测试与真实 Linux FUSE write→truncate 测试通过。
- `src/node/vfs/ownerfs.rs`、`ownerfs/files.rs`、`ownerfs/root.rs`：Home 对每个实际打开句柄核对根、peer 与授权；过期 peer 会话被围栏，缓存 grant 与打开 FD 均可清理。Meta 不可达时保留句柄，不把未知状态误判为过期。
- `src/node/rpc/peer.rs`：异步 RELEASE 有界排队、短暂失败重试，重复释放按幂等处理；`src/node.rs` 每 10 秒从热路径外执行回收；Meta `LookupNode` 将过期会话视为不存在。

**限制：** 回收是 Meta lease 到期后的后台动作，不承诺 B 崩溃瞬间撤销或旧 FD 透明续用。B 仍需重挂载、重新打开。当前证据覆盖一次真实 B daemon 故障和定向单元/集成测试；未覆盖 VM 掉电、长稳、大规模句柄数或完整 POSIX。Meta 单活动围栏/选主、Root 生命周期的其他缺口仍按既有计划处理。

原始证据保存在研究工作区的 `experiments/results/2026-09-27-afs-p2p-hardening/`：`acceptance.json`、`w1/result.json`、`w2-8-r1/result.json`、`w2-8-r2/result.json`、两份顺序 W2、`profile-receipt.json` 与 `fault-reap-home.log`。这些本地运行数据不纳入 Git；固定负载之外的性能仍需扩面。本阶段只交付 `feat/agent-workloads-foundation` 分支，未合并或发布。
