# fuser 官方依赖决策与迁移验收

更新日期：2026-10-09。当前执行状态见 [计划 S4/S5](plan.md)。沿用原路径以保留链接；旧版本阻塞结论保留原版本和判据。

## 本次决策

本次迁移前快照采用固定官方发布版 `fuser =0.18.0`，必要迁移放在自有 FUSE 适配代码，目标是不将 `third_party/fuser` 带入目标仓库。不引入 libfuse FFI、不自研 FUSE 协议层，也不为立即补齐锁能力换库。本轮必要正确性验证后已移除私有目录（51 文件，497,529 字节，恢复清单留本地）；改依赖或编译成功不等于功能验收通过。

本次不承诺 FUSE 跨节点 `fcntl/flock`、阻塞锁等待取消，以及 OwnerFs workspace bind/native 与远端 FUSE 的锁一致性。这替代此前“先获得上游 lock flags/interrupt API”的迁移前置条件。既有后端锁实现、针对性测试与历史通过/失败保留可恢复记录；G1 历史 8/8 不重开，新候选不继承旧锁能力结论。

必要正确性不放宽：读写新鲜度、close-to-open、权限、错误传播、持久化、direct-I/O mmap 能力协商、正常卸载和受管引用排空必须保留。上游草稿仅保留本地，本轮不向第三方社区发布 Issue/PR。

## 固定官方依赖

| 身份 | 固定值 |
| --- | --- |
| 发布版 | `fuser 0.18.0`，精确版本，不用私有 path/git patch |
| 发布包 SHA256 | `b82b6597d216503555ead6b358f341ef748869bf5c6fbae6a0cb9dd231baecfd` |
| 发布包 VCS commit | `9c957f74efe715112049298cdf1d601781829c8d` |
| 历史对照 master | `c0420fc49d3f1ce09603beb127f392eb2726c2a1`，不是本次依赖 |

官方 0.18.0 未暴露旧私有 `LockOptions/lk_flags` 和 `Filesystem::interrupt`；锁后置后不再要求这些接口进入本次交付。公开 `Filesystem` 使用 `INodeNo`、`FileHandle`、`LockOwner`、`Errno`、`InitFlags`、`FopenFlags` 等类型，已在自有适配中迁移签名及错误映射。

## 全部私有差异及处理

原 vendor 基于官方 0.16.0。完整对照为 21 路径：14 修改、6 官方文件省略、1 vendor 专用说明。逐文件 SHA、完整 diff 和原结论在源码仓上一级 `local-archive/2026-10-09-6b75d78a/tree/development/evidence/20261006-fuser-upstream-review/`，由 `local-archive/README.md` 导航；构建不依赖归档，原 Git 版本保持可追溯。

“公开 API 替代”是迁移方案，**仅在下列有限 Linux 范围通过，不代表完整验收**。调用点使用自有模块/符号，避免依赖迁移中变化的行号。

| 差异路径 | 用途及产品调用点 | 分类与处置 |
| --- | --- | --- |
| `.cargo_vcs_info.json` | 官方 provenance 被省略；无产品回调 | 官方注册表与 `Cargo.lock` 固定身份 |
| `.cirrus.yml` | 上游 CI 被省略；无产品调用 | 退出本次交付 |
| `.dockerignore` | 上游容器元数据被省略；无产品调用 | 退出本次交付 |
| `.github/workflows/ci.yml` | 上游 CI 被省略；无产品调用 | 退出本次交付 |
| `.gitignore` | 上游规则被省略；无产品调用 | 退出本次交付 |
| `Cargo.lock` | 官方示例 lock 被省略；工作区 lock 控制产品依赖 | 官方版本由产品 `Cargo.lock` 固定 |
| `AFS-PATCH.md` | 私有说明未覆盖完整改动 | 归档保留，不能证明原版一致性 |
| `Cargo.toml` | 私加 ABI 7.33/7.36；产品 `Cargo.toml` path 依赖 | 官方 features/API 替代，核对实际解析来源 |
| `deny.toml` | lint/依赖策略；无产品调用 | 退出，不修改官方源码压制 lint |
| `examples/notify_inval_inode.rs` | 私有 open 签名适配；非产品调用 | 退出本次交付 |
| `examples/passthrough.rs` | 私有 open 签名适配；非产品调用 | 退出本次交付 |
| `examples/poll.rs` | 私有 open 签名适配；非产品调用 | 退出本次交付 |
| `examples/simple.rs` | 私有 open/create/setattr killpriv 参数适配；非产品调用 | 示例退出；产品清位另验 |
| `src/channel.rs` | 私有停止 socket/receive 唤醒；`node::fuse::MountedFuse` 生命周期 | 正常退出必须保留；自有正常卸载＋官方 join；真实排空/退出通过 |
| `src/lib.rs` | 私有锁 options/interrupt、killpriv 参数及 lint；`AfsFuse` 回调 | 锁接口退出；typed 公开回调替代；killpriv 正确性必须保留、legacy 方案在限定 Linux 用例通过；lint 补丁退出 |
| `src/ll/fuse_abi.rs` | killpriv-v2、mmap/FLOCK 常量和布局；`AfsFuse::init/open/create/write/setattr` | mmap 用公开 `InitFlags/FopenFlags`；锁标志退出；不协商未实现的 killpriv-v2；官方布局须回归 |
| `src/ll/request.rs` | 私有锁/interrupt/killpriv 解析和 Linux rename 布局测试 | 锁/interrupt 退出；killpriv legacy 限定通过；保留 rename 正确性要求 |
| `src/mnt/fuse_pure.rs` | 私有 EBUSY 卸载 fallback；`MountedFuse` | 自有正常卸载、mount ID 核验；busy 保留挂载和报错通过 |
| `src/reply.rs` | separate TTL；`AfsFuse` lookup/cache policy 回复 | 官方 `entry_with_ttls(attr_ttl, entry_ttl, …)` 替代；独立 TTL 优化可后置，保守 TTL=0 不降低新鲜度 |
| `src/request.rs` | 私有锁/interrupt/killpriv 分派、解析错误；`AfsFuse` | 锁/interrupt 分派退出；公开回调、清位与错误传播限定通过 |
| `src/session.rs` | 私有停止/receive-loop 唤醒；`MountedFuse` join | 自有正常 `fusermount3 -u` 后官方 `BackgroundSession::join()`；通过正常/忙引用回归 |

旧 vendor 全 workspace/all-features CI 曾在 `reply::test::reply_create` 的 ABI 7.40 参数/`FOPEN_PASSTHROUGH` 断言处失败并 SIGABRT。原失败保留；采用官方依赖不改写旧结论，不修改第三方测试、隐藏 feature 或忽略失败来证明成功。

## 自有适配与必要正确性

- **锁边界**：不协商 `FUSE_POSIX_LOCKS/FUSE_FLOCK_LOCKS`。实际进入官方 `getlk/setlk` 的请求明确返回 `EOPNOTSUPP`，不调用后端锁操作、不猜 `lk_flags`。未协商时 Linux 可以在本挂载内本地回退，不能保证所有锁请求进入用户态或全部报不支持。bind 的本机 ext4 锁与 FUSE 本地回退锁是不同锁域，均不宣传为分布式锁。
- **suid/sgid**：0.18.0 不完整暴露旧 killpriv-v2 原因；本次不协商 `FUSE_HANDLE_KILLPRIV(_V2)`，保留 Linux legacy 清位、`DefaultPermissions` 与后端授权。自有适配只识别常规文件精确特权清位：清 suid，以及具有 group-execute 时的 sgid，不把一般 chmod 或 uid/gid 变化当可信原因。Home 再用当前权威属性复核；并发 chmod 不匹配必须报错，不能写回旧 rwx mode。识别/规范化不免除 size、时间、属主或权限检查，非 executable sgid 必须保留。direct-I/O WRITE 的公开 `WriteFlags::FUSE_WRITE_KILL_SUIDGID` 仍须透传；Linux 实测 UID501 正向写清位、root/CAP_FSETID 保留、non-executable sgid 保留。
- **mmap 与新鲜度**：使用公开 `InitFlags::FUSE_DIRECT_IO_ALLOW_MMAP`，只在内核宣告支持时请求；OwnerFs 远端读写继续使用 `FopenFlags::FOPEN_DIRECT_IO`，不因迁移开启 writeback/KEEP_CACHE 或延长未经验证的 TTL。shared mmap 需真实挂载回归；不扩大已有 FD/mmap 即时刷新/撤权保证。
- **持久化与卸载**：flush/fsync/父目录屏障、既有失败状态和 errno 传播保持原约束；release 不是持久提交。官方 root 路径为正常 umount，但 EPERM 回退可用 lazy unmount，因此自有适配核对 mount ID，显式 `fusermount3 -u`（无 `-z`），挂载消失后才 join。正常卸载或身份核验失败保留会话，Node 等既有 watchdog 失败退出，不继续关闭后端；回调与受管引用排空、实际进程退出和挂载消失分别验证，不以 drop 或命令返回代替。

## 本次验收出口

本轮限定迁移验收通过，原始证据在源码仓上一级 `local-archive/fuser-investigation/official-validation-evidence.tar.gz`，SHA256 `089b94ea04014247d241eff6c9b58864519bee30b8988a13e19542b579a9585a`；入口及恢复清单见 `local-archive/README.md`。发布依赖 85 文件与 Cargo 实际缓存逐字节一致，123 个 Rust/proto/Cargo 输入与当前树一致。

| 范围 | Linux ARM64 结果 | 限制 |
| --- | --- | --- |
| 产品 bins、测试编译、fmt、workspace/all-features/all-targets Clippy | PASS | 普通非 RDMA build 保留两个既有 fallback 警告 |
| 调度 / legacy 清位 / killpriv / config / FUSE 非 ignored 合约 | 20 / 2 / 12 / 10 / 4 PASS | killpriv 另 1 ignored；不等于全 POSIX |
| 真实 FUSE 合约 | 10 PASS | 含 mmap、FIFO、errno、statfs、busy 不 detach |
| 真实 suid/sgid / 权限 | 1 PASS | 含 held FD 后 chmod、非 owner chmod/utime 拒绝；lockf/flock 本地成功仅记录回退 |
| 真实 bind / FD+mmap 排空 | 2 PASS | 物理 ext4 覆盖、身份和 busy 约束 |
| 非 root 正常卸载 / busy 不 detach | 2 PASS | UID501，专用 mount-owner 用例 |
| bind ON＋远端＋local-file 全停恢复 | 限定 PASS | 同一 VM 两个独立 Node，经 mTLS TCP；64KiB，50 检查、7 次实际 wait0；不是跨主机或性能对照 |

首轮迁移曾出现 Request 类型/读缓冲借用编译错误，以及自有适配 mutex 临时值延长造成 cached-read 自锁；均已修复，失败、超时及仅针对失败测试连接的 abort 清理记录保留，不改写为正常退出。新测试中 DFS 纯清位走 metadata-sync 的断言错误及 runner 的多余 dfs_mount 配置错误也保留。非 root 首次使用 root-only MockBackend 写用例的 EACCES 保留，改用明确支持 mount owner 的卸载用例完成实际验证。

当前没有已知的必要官方接口阻塞；跨主机、完整 POSIX、复杂失败退出组合和性能仍待各自验收。旧后端锁源码及测试保留；被移出的旧 FUSE 锁/interrupt 适配原件、完整 vendor、历史通过/失败在本地快照与原 Git 版本可恢复，未向上游外发。

## 后置锁专题

1. FUSE↔FUSE 跨节点字节范围锁和 flock。
2. 冲突、等待、取消、释放及进程/文件描述符生命周期。
3. bind/native↔FUSE 锁域协同。
4. 按统一 E2E 行为清单评估 fuser 上游、fuse3、fuse-backend-rs，最后才评估自研协议层。

Curvine 自有 Rust FUSE 协议、会话、请求处理和锁协调说明自研可行，不能推导为本次应重写或已完整覆盖 fuser。fuse3、fuse-backend-rs 是后续候选，尚无完整直接替换验证。fuser 使用规模不等于所有能力更好；未找到当初 fuser/fuse3 完整选型记录，不补写历史理由。

官方参考：[0.18.0 API](https://docs.rs/fuser/0.18.0/fuser/trait.Filesystem.html)、[发布源码身份](https://docs.rs/crate/fuser/0.18.0/source/.cargo_vcs_info.json)、[KernelConfig](https://docs.rs/fuser/0.18.0/fuser/struct.KernelConfig.html)、[BackgroundSession](https://docs.rs/fuser/0.18.0/fuser/struct.BackgroundSession.html)。
