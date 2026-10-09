# 当前计划

更新日期：2026-10-09
当前 Issue：[#44 迁移前快照](https://github.com/lelezi257/dms/issues/44)

本计划是当前唯一维护的项目计划入口。历史过程、逐轮证据、checkpoint、失败原始材料和旧计划保存在源码仓上一级 `local-archive/`，不再作为产品树内容维护。

## 当前快照目标

在导入 Agent DX 之前，DMS 自身先形成一个清洁快照：

- 可编译、可安装、可运行的有限版本仍清楚可追溯。
- 正式文档以中文描述当前能力、边界和后续任务。
- 维护中的验收工具和测试保留在源码树内；历史过程资产移出源码树。
- 解除私有 `fuser` 补丁依赖作为正式导入前置项；本次锁能力明确后置，必要正确性仍不足时停止受影响迁移并记录具体缺口。

本轮不执行 Agent DX 目标仓迁移，不追求性能优化，不重开 G1 历史 8/8。

## 本轮依赖决策与有限执行顺序

2026-10-09 新决策替代此前“锁/取消必须先补齐”的迁移前置条件：本次不承诺跨节点 fcntl/flock、阻塞等待取消或 bind/native↔FUSE 锁域一致性。既有后端实现、测试与历史结论保留，后续按统一 E2E 清单评估官方 fuser、fuse3、fuse-backend-rs，最后评估自研；本次不换 FFI、不自研协议层、不向第三方外发。

1. 收口工具测试小项，冻结 `6969a069`、原包及其证据身份。
2. 按完整 vendor 差异清单分类，迁移固定官方 `fuser =0.18.0` 的公开 API；仅调整自有适配和必要清位兼容。
3. 先在既有 Linux 验证受影响编译/单测，再验证 suid/sgid、权限、direct-I/O mmap、bind ON＋远端新鲜度与正常卸载/排空。依赖切换后的候选不继承旧运行结论。
4. 必要正确性未通过时停受影响迁移，记录具体接口和 case；验证通过后再移除可恢复的 vendor，更新当前文档与同一 Issue/PR。

## 阶段账本

| 阶段 | 状态 | 出口 |
| --- | --- | --- |
| G1 可试用版本 | 历史关闭，8/8 | 同事可拿到 OwnerFs/DFS 试用包；memory 演示、local-file Meta 正常重启恢复、OwnerFs 本地/远端基本操作、DFS 基本跨节点读写和有序停止在原版本范围内成立。 |
| G2 核心试用与性能 | 进行中 | 优先 OwnerFs workspace bind ON 当前场景，其次远端访问协同和性能，再 DFS 一写多读，最后普通本地 FUSE 优化。 |
| G3 复杂可靠性与后端 | 后置 | 大规模、长时间、复杂故障、多 Meta、etcd 资源专题和 Redis 后端。 |

## G2 优先级

1. **G2.12 OwnerFs workspace bind ON 功能闭环与试用交付**：真实 Home 底层目录 bind 到 OwnerFs FUSE 根下对应一级 workspace；默认 OFF；提供明确 ON 配置、支持范围和已知限制。
2. **G2.13 bind 核心性能**：选定八个小规模核心 case 继续使用 `>=0.90 native ext4` 判据；已有历史通过证据仅在未受影响范围内复用。
3. **G2.14-G2.16 OwnerFs 远端读写删除**：读写吞吐 `>=1.2x` 同条件 MooseFS，独立操作时延 `<=0.8x` 同条件 MooseFS；删除要求正确性和性能报告，不新增硬比例。
4. **G2.21 DFS 一写多读**：同 POSIX/FUSE 接口、三同步持久副本条件下与 3FS 持平；性能待验时不得冒称达标。
5. **G2.09-G2.11 普通本地 OwnerFs**：保留同 MooseFS 双目标，但优先级低于 bind 当前场景、远端协同和 DFS。

## 当前已知结论

- a103 试用包已发布并可安装运行：真实 Home bind ON、UID501/UID502、Owner64KiB/DFS64MiB local-file Meta 正常全停重启恢复在限定范围内通过。
- G1 历史 8/8 保持关闭；当前快照不自动继承历史完整验收。
- 普通 OwnerFs 本地读已记录 FAIL：吞吐和时延均未达到 MooseFS 双目标。
- 远端读历史记录为吞吐 FAIL、p95 PASS；仍需按固定条件补齐当前候选验收。
- DFS 一写多读已有功能证据；3FS 性能对照仍待验。
- 官方 `fuser =0.18.0` 已完成限定迁移验收，无私有 vendor；受影响 Linux 构建、权限、mmap、正常卸载和 bind ON＋远端核心场景通过。锁/取消 API 缺口按本次范围调整退出迁移前置；新鲜度、close-to-open、权限、错误传播、持久化、direct-I/O mmap 及正常卸载/排空保持，限定证据见 [依赖决策与迁移验收](fuser-official-blocker.md)。
- FUSE 单挂载内核本地回退锁与 bind 路径本机 ext4 锁是不同锁域，均不是分布式锁；不保证所有锁 syscall 进入用户态。进入官方 getlk/setlk 的请求必须明确拒绝，不能假成功。

## 迁移前整改项

| ID | 项目 | 状态 | 出口 |
| --- | --- | --- | --- |
| S1 | 本地归档 | 完成（本地恢复已验证） | `local-archive/` 有完整快照、manifest、校验和恢复记录；源码树移出过程证据和历史切片。 |
| S2 | 测试边界 | 完成（限定工具回归） | 维护中的验收驱动在 `tests/acceptance/`，测试探针在 `tests/support/`，普通构建不依赖测试程序。 |
| S3 | 中文正式文档 | 完成（链接与独立复核通过） | `docs/architecture`、`docs/development`、`docs/testing`、`docs/deployment` 成为正式入口；保留可操作安装/配置/运维步骤，不保留历史流水账。 |
| S4 | 官方 fuser | 完成（限定正确性范围） | 固定 `=0.18.0`，自有适配，无私有 patch/path/vendor 依赖；锁能力后置，通过本轮受影响正确性验证；不代表完整标准或性能。 |
| S5 | 候选验证 | Linux 限定通过，PR CI 单独登记 | Linux 受影响构建、权限/清位、mmap、新鲜度、bind ON＋远端核心场景和正常卸载/排空；PR 给出新候选 SHA、结果与限制。 |

## 不做项

- 不重写 Git 历史，不 force push。
- 不把原始证据、VM 镜像、大型备份或运行日志提交到源码仓。
- 不把 fuser vendor 目录直接删除后宣布完成。
- 不用文档清理、工具检查或提交数量充当产品目标完成。

## 本次快照的可追溯边界

- 归档基线：`6b75d78a1c9350553770394b13384e01a9b292c0`。本地入口为源码仓上一级 `local-archive/README.md`，原件目录 `2026-10-09-6b75d78a/`；构建和维护中的测试不依赖此目录。
- 一次性原源码快照 SHA256：`a5f6b8127d6aeb516113c0a48d6af65d546d9486f40d758b9bb6f8602d357b43`。manifest 与 Linux 实际恢复记录核验了 20,066 个文件及 9 个符号链接的内容、模式和目标；历史 Git 不改写。
- 移出当前树的原路径 19,659 个（约 93.1 MiB）。必要回归输入在 `tests/acceptance/fixtures/` 保留 185 个小输入，因此原路径归档数不是重复文件数；本轮另移出 vendor 51 文件/497,529 字节并新增必要测试，净统计以 PR 冻结树为准。
- 维护中的 Python：`tests/acceptance/` 126 个 runner、driver、probe 和回归模块；其它 `scripts/` 与 `tests/` 共 11 个。另有 22 个冻结 Python 输入只用于验证历史观测的 hash 绑定，不作为维护工具或当前通过证据。5 个旧 VM 固定路径专用启动/重启脚本已归档。
- `6969a069` 收口此前工具与文档小项；该历史快照沿用 `f540c242` 的产品构建输入，不修改当时的 Cargo 或第三方源码。现在进行的官方依赖迁移会修改 Cargo 及自有 FUSE/必要兼容代码，不能继续套用“产品行为未改”或旧候选通过结论。

## 当前官方依赖候选验证

固定官方版本、完整私有差异分类、测试结果与本地证据 SHA 见 [依赖决策与验收](fuser-official-blocker.md)。新增关闭 S4：发布包 85 文件逐字节一致、123 个构建输入与本地树一致；产品 bins、fmt、Clippy 通过，真实 FUSE 合约 10 PASS、权限 1 PASS、bind 排空 2 PASS、非 root 卸载 2 PASS。两个 Node 经 mTLS TCP 的 bind ON＋远端读写/清位/删除及 local-file 正常全停恢复通过（64KiB，7 次实际 wait0）。这是同 VM 功能验证，不宣称跨主机、完整 POSIX、复杂可靠性或性能达标。

依赖切换前历史 PASS/FAIL 保留，下表不用于替代当前结果。S5 的完整 PR CI 待新提交结果，正式 Agent DX 导入未开始。

## 历史快照证据

下表 PASS 属于官方依赖切换前的快照，限定原候选范围；`6969a069` 的工具收口、`f540c242` 的运行包和 a103 Release 各保留原身份。当前候选不自动继承下表结论。继续复用既有 `afs-build` Linux ARM64 和 `afs-g2-micro`，不重建环境；原命令、日志、失败和校验和保存在 `local-archive/`。

| 范围 | 旧快照结果 / 当前待验 | 解释 |
| --- | --- | --- |
| 格式、产品 bins、测试 examples、release bins 构建 | PASS | Linux Rust 1.95；产品构建保留两个既有非 RDMA fallback 警告。测试探针仍是显式 example，不属于产品 bin。 |
| Clippy workspace/all-features/all-targets | PASS | `-D warnings`；不等于运行验收。 |
| 配置合约 | 10 PASS | `config_contract`；不能代替完整 POSIX。 |
| 受影响工具回归 | 固定 f6a6acdf 主工具 535 PASS / 4 SKIP；后续 probes 定向 21 PASS | CI 主组发现 539 项，无失败；4 SKIP 为可选历史夹具及三个限定平台 coordinator。probes 的四个前置平台隔离错误保留，修复后既有 Linux ARM64 两模块 21 PASS；模拟 x86_64 为 20 PASS / 1 SKIP，不能冒充真实 x86 内核证据。其它未变项复用，未重跑本地主组。 |
| 打包可复现性、进程控制、试用配置和自检引用 | PASS | 说明文件新路径正确、manifest 一致、测试探针不入普通包；不证明 bind 功能或性能。 |
| 历史 f540c242 bind ON 安装运行 | 限定 PASS | 原 release 包在无编译器 VM 全新安装；真实 Home bind、UID501/502、64KiB 读写删除、FUSE 正反对照及 4 次实际 wait0。未运行远端、恢复、性能或完整 POSIX。 |


历史有限运行候选源码为 [`f540c242`](https://github.com/lelezi257/dms/commit/f540c242f26f9815d7ff5f3cd3f1f1babf82af1b)，包 `afs-snapshot-f540c242-linux-aarch64.tar.gz` 仅保留本地归档，未替换 a103 GitHub Release。

- 包 SHA256：`8fb6d5f052f662a93ef76836fc99f573fde16db5268fbc89eac3fb20236d8f30`。
- Meta ELF SHA256：`d5c4f0cdcd9d944cb075a8721c49b61ac5292a65b9d03c06e6650ff31a5f0631`。
- Node ELF SHA256：`3f4a9826a6f40cd7225625dae903c628ca71c0a369195dfafc1b1e66fc8a1640`。
- 本地运行证据归档 `host-bind-f540c242-evidence.tar.gz` SHA256：`d17f198567ac98cbae1ef6dc00ee8809815b707eb83b8034397ec94ee368a7c6`。

这些检查关闭的是本次快照的受影响工程子项，不新增 G1/G2 产品完成项。S4 已按新范围完成，S5 完整 CI 独立登记；正式 Agent DX 导入仍未开始。

固定 `f6a6acdf` 的 [CI 37923964098](https://github.com/lelezi257/dms/actions/runs/37923964098) 已结束为 FAIL：工程构建/检查和打包 PASS，主工具 535 PASS/4 SKIP；probes 158 PASS/2 FAIL/2 ERROR/1 SKIP，vendor ABI 7.40 `reply_create` 断言/SIGABRT 失败。`6969a069` 仅收口 probes 单元测试前置隔离及真实拒绝哨兵，ARM64 定向 21 PASS、模拟 x86_64 20 PASS/1 SKIP，不改写整轮 CI 结果。[更早 CI 37921408568](https://github.com/lelezi257/dms/actions/runs/37921408568) 及全部失败原件保留归档。上述属于旧私有依赖候选；当前 S4 按新锁边界完成官方迁移和限定验证，不外发上游草稿，Agent DX 正式导入仍未开始。

`6969a069` 的 [CI 37926197719](https://github.com/lelezi257/dms/actions/runs/37926197719) 已结束 FAIL，仅 Rust Test 步骤失败（vendor ABI）；工具、打包及其余步骤通过。该旧版失败保留，不作为当前官方候选结论。
