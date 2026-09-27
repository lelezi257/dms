# OwnerFs 实施与验收记录

状态：既定三节点功能与 W1 门槛已通过；W2 后续已恢复基本持平，见[优化复验](../reviews/ownerfs-v13-w2-optimization.md)。目标以 `2026-09-26-ownerfs-readiness.md` 为验收合同；原阶段数据见[阶段复验](../reviews/ownerfs-v12-stage-review.md)。本文件记录实施顺序和接口偏离，不改变合同。

## 必须闭合的路径

1. Meta 在持久存储中完成 Node 会话、根预留/激活/查询/授权/校验/恢复。失败时拒绝授予访问权。
2. Node 启动先注册会话、锁住并对账本机根，再挂载 FUSE。一级 `mkdir` 完成 Meta 预留、本机普通目录及身份记录同步、Meta 激活；根内本地操作直接到普通文件，不逐文件找 Meta。
3. B 通过 Meta 定位 Home 和取得授权；Home 只接受经过节点通道认证且经 Meta 首次验证的授权。B 的文件命令在 A 同一份普通文件上执行，A/B 可同时操作，B 加入不撤销 A。
4. FUSE 回调共享/远端路径保守使用 direct-io、无 writeback、TTL=0；未被远端访问的本地私有根启 1 秒缓存，首次远端访问前失效并退出私有缓存。进程内 inode/FD 表保持 rename/unlink 后旧 FD 身份。Home 重启后旧远端 FD 报 ESTALE，重新打开可恢复。
5. 在 Linux 重跑旧三 VM 10 步、完整 W1 两次独立会话各六轮，同场对照 MooseFS/薄 FUSE/Native；W2 报告分段。W1 每份 p50 比值均须不大于 0.80。

## 已识别的合同增量，交付前复审

- **VFS 构造**：既有 `Vfs::new` 只装配无依赖的 OwnerFs 骨架，生产需要注入已完成注册/恢复的 OwnerFs 实例；已增加 `Vfs::with_ownerfs`，保留原构造供框架测试。原因是不能把未接通的骨架当作挂载业务。
- **Meta 数据模型**：原 `RootRecord` 无 Pending/Active 与 prepare token，无法表达 reserve/activate；须在既有 `meta/store.rs` 增状态与持久字段。
- **节点发现**：`RootLocation` 只有 Home ID，没有 Home endpoint；2–4 Node P2P 需要通过 Meta 注册表解析地址，不能依赖单个静态 `peer_endpoint`。
- **认证**：原 Node gRPC 默认明文，不能把请求自报的 `holder_node_id` 当作认证身份；OwnerFiles 只能在可信通道身份绑定后启用。
- **Storage 属性操作**：旧验收的 fchmod/ftruncate 需要 `FileHandle`/`FileStore` 补必要方法，仍由 OwnerFs 决定文件语义。

上述合同增量已在当前分支实现；具体复验、仍未覆盖的边界与 W2 缺口以[阶段复验](../reviews/ownerfs-v12-stage-review.md)为准。功能/性能验收只使用新分支、新二进制和新运行日志；旧分支数字只作目标来源。
