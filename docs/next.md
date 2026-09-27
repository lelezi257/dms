# 下一步

本阶段的 OwnerFs 句柄保序、Home 授权绑定、异步 RELEASE 与故障回收已完成复验。当前 Linux 三 VM 功能验收 15/15；固定 200×4 KiB、8 worker 完整 W2 两份 12 轮为 MooseFS 的 0.768/0.780；顺序 W2 两份为 1.106/1.080，未证明稳定达到 1.10。详见[修复与复验](reviews/2026-09-27-ownerfs-p2p-hardening.md)及[当前状态](status.md)。

1. **扩面性能：** 增加文件数、大小、并发与远端读比例，用现有分段指标定位 B 重读/覆盖写及顺序 W2 波动；有白盒证据后再优化 P2P/FUSE，复验本地 W1。
2. **补强故障与控制面：** 验证 Meta 临时不可达、多个遗留 FD、进程重启后重新打开及根创建的真实中断切点；另行设计单活动 Meta 围栏/选主。`memory` Store 仅用于可丢弃测试集群，不能承担跨重启授权。
3. **补齐产品边界：** 恢复 Agent 常用 `chmod/chown/atime/mtime`；单独设计根删除、同名重建与跨节点根列举。完整 POSIX、VM 掉电与长稳尚未验收。

保持已确认的 VFS/Storage/Proto 核心合同；BlobFs 与 S6 不在此阶段。阶段历史、已关闭的问题和当时的下一步保留在 [OwnerFs 审视记录](reviews/)与 Git 历史；代码入口见[目录架构](code-layout.md)。
