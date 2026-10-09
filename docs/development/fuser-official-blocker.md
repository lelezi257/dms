# fuser 官方版迁移阻塞说明

更新日期：2026-10-09

当前目标是迁入 Agent DX 前去除私有第三方补丁依赖，但不能牺牲数据新鲜度、close-to-open、权限、错误传播、锁语义、interrupt 取消语义或 direct-I/O mmap 能力协商。

## 对照结果

已对照官方 `fuser` 0.18.0 与固定 master：

- crates.io 最新非 yanked 发布：`fuser 0.18.0`。
- 发布包校验和：`b82b6597d216503555ead6b358f341ef748869bf5c6fbae6a0cb9dd231baecfd`。
- 发布包 VCS commit：`9c957f74efe715112049298cdf1d601781829c8d`。
- 对照 master commit：`c0420fc49d3f1ce09603beb127f392eb2726c2a1`。

官方发布版和当前 master 均不能无损承载当前 OwnerFs/DFS 使用方式：

1. `Filesystem` 回调没有暴露当前锁命名空间需要的 `lk_flags`。
2. `Interrupt` 仍缺少可用回调入口，当前实现依赖取消信号传播来避免长操作不可控。
3. 官方 0.18.0 已有 direct-I/O mmap 能力的公开接口，可在自有适配层迁移；它不是当前主要缺口，但必须保留协商和行为。
4. 当前 killpriv 请求原因参数在 0.18.0 也不完整，所核对 master 已补充；仅换到 master 仍不能解决前两个缺口。

## 现存 vendor 的边界

现存目录基于官方 0.16.0。完整逐文件对照有 21 处路径差异：14 个文件修改、6 个官方打包/仓库文件省略、1 个 vendor 专用说明；原 diff 和逐文件 SHA 保留在本地归档。修改涵盖 TTL、锁/killpriv/interrupt 请求传递、能力常量、卸载唤醒及对应示例/策略适配。`AFS-PATCH.md` 的 TTL/lint 描述没有覆盖现存完整差异，不能用它证明官方原版一致；本轮不再修改 vendor 来补说明。

远端全 workspace/all-features CI 还发现 vendor 内 `reply::test::reply_create` 在 ABI 7.40 下使用带 `FOPEN_PASSTHROUGH` 位的测试参数，与 `ReplyCreate::created` 的断言冲突并 SIGABRT。这是实际未通过项，未改 vendor、关闭 feature 或排除测试；不把产品构建成功当作全部依赖测试通过。

## 当前决策

- 本快照暂保留 `third_party/fuser`，并通过 `.gitattributes` 标记为 vendored。
- 不再继续向第三方源码追加无关接口、常量或 lint 修改。
- 迁移到官方发布版前，必须先有公开 API 或上游可接受变更覆盖上述语义。
- 不允许仅删除目录、改版本号或降低行为要求后宣布完成。

## 后续出口

1. 形成上游 issue/patch 或找到官方版本中等价 API。
2. 在自有 FUSE 适配层完成迁移，不把私有接口藏回产品逻辑。
3. Linux 上确认构建、锁/取消/权限/freshness/direct-I/O mmap 相关受影响测试通过。

官方固定源码：[锁接口](https://github.com/cberner/fuser/blob/c0420fc49d3f1ce09603beb127f392eb2726c2a1/src/lib.rs#L1167-L1216)、[Interrupt 分派](https://github.com/cberner/fuser/blob/c0420fc49d3f1ce09603beb127f392eb2726c2a1/src/request.rs#L146-L149)。产品调用位于 `src/node/fuse.rs` 的 `getlk_with_options`、`setlk_with_options` 和 `interrupt`，后端取消由 `src/node/vfs/locks.rs` 处理。独立 TTL 优化可以后置；公开 API 丢失的锁类型或请求取消信息不能在下游包装层猜测恢复。上游问题草稿仅保留在本地归档；当前决定暂不外发 Issue/PR，依赖整改仍未完成。
