# OwnerFs 根创建故障恢复闭环（2026-09-27）

## 结论

- **事实：** Node 启动在挂载前取得本机 catalog 排他锁，读取 Meta 的同一线性化 active/pending 快照，并核对本机记录与物理目录。active 根必须有匹配 catalog 和目录才能恢复授权；pending 根先用原 reservation 做 Meta CAS 撤销，再清理空目录和本机记录。Meta 不可达、身份不符或目录含意外文件时拒绝启动或拒绝清理。
- **事实：** `AbortRoot` 要求 epoch、session、intent、prepare token 全部匹配，并在事务里比较完整 reservation 值。旧 Abort 不能删掉同名新预留。`ActivateRoot` 同样以完整 reservation 作条件。prepare token 含 Reserve 请求 ID，避免同 session 同 intent 重复预留时出现 ABA。
- **事实：** Activate 已提交但回复丢失时，Node 保留已同步的 catalog 记录，不再盲目 Abort/删除。重启时 Meta active 与本机记录匹配后恢复 Home 新会话。未激活的预留清理中再次崩溃，下次启动继续收尾。
- **推断：** 本次变更只增加启动对账和创建失败处理，不改变已授权根内文件热路径；未重跑 W1/W2，不能据此报告新的性能数值。

## 故障切点与证据

| 切点 | 处理 | 验证 |
| --- | --- | --- |
| Reserve 提交后、建目录前 | 启动发现 pending，CAS 撤销 | 确定性 RootManager 测试 |
| 建目录后、父目录同步前 | 撤销 pending，删除空目录 | 同上 |
| 父目录同步后、catalog 前 | 同上 | 同上 |
| catalog 同步后、Activate 前 | 核对 prepare 身份后撤销，清目录与 catalog | 同上 |
| Activate 提交、回复丢失 | 保留 catalog，重启恢复 active 根 | 确定性 RootManager 测试 |
| Abort 提交、清本机记录前 | 下次启动继续清理 | 确定性 RootManager 测试 |
| 旧 Abort 与新 Reserve 竞争 | 完整 reservation CAS 拒绝旧 Abort | Meta 合同测试 + 真实 etcd CAS 测试 |

异常防线另测：Meta active 但本机 catalog 缺失会拒绝挂载；pending 目录含意外文件时不会删除数据。Activate 回复已到但尚未写入运行态缓存，与“提交后回复丢失”拥有相同持久状态，由同一重启恢复测试覆盖；没有逐点执行真实进程 `SIGKILL` 注入。

Linux 验证：`cargo test --workspace --all-features --exclude fuser` 通过；`cargo clippy --workspace --all-features --all-targets -- -D warnings`、`cargo fmt --all --check`、release binaries 构建通过。配置 `AFS_TEST_ETCD_ENDPOINT=http://127.0.0.1:2379` 的真实 etcd CAS 测试通过。新 release 二进制在 A/B/C 三 VM 的 OwnerFs 验收 **15/15**，无 gap 或清理错误；[原始 JSON](../../../experiments/results/2026-09-27-afs-ownerfs-p0/acceptance.json)。本轮未重跑 W1/W2 或 VM 掉电、长稳。

## 边界

本轮只闭合根创建 P0；根删除重建、全局根列举、`chmod/chown/atime/mtime`、完整 POSIX、BlobFs 和 OwnerFiles RDMA 内容路径仍在后续范围。已确认的普通文件数据不会因 pending 清理被删除：非空目录清理失败并保留文件，需人工诊断。Meta active 却缺本机 catalog 的既有损坏状态也拒绝自动重建，避免把未知物理目录误认为已授权根。未 push、merge 或 release。
