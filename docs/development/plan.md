# 当前计划

更新日期：2026-10-09
当前 Issue：[#44 迁移前快照](https://github.com/lelezi257/dms/issues/44)

本计划是当前唯一维护的项目计划入口。历史过程、逐轮证据、checkpoint、失败原始材料和旧计划保存在源码仓上一级 `local-archive/`，不再作为产品树内容维护。

## 当前快照目标

在导入 Agent DX 之前，DMS 自身先形成一个清洁快照：

- 可编译、可安装、可运行的有限版本仍清楚可追溯。
- 正式文档以中文描述当前能力、边界和后续任务。
- 维护中的验收工具和测试保留在源码树内；历史过程资产移出源码树。
- 解除私有 `fuser` 补丁依赖作为正式导入前置项；如官方 API 不足，则明确阻塞而不削弱行为。

本轮不执行 Agent DX 目标仓迁移，不追求性能优化，不重开 G1 历史 8/8。

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
- 官方 `fuser` 0.18.0 和固定 master 缺少当前实现需要的 lock flags 与 interrupt 回调入口；无损迁移当前阻塞，详见 [fuser 官方版迁移阻塞说明](fuser-official-blocker.md)。

## 迁移前整改项

| ID | 项目 | 状态 | 出口 |
| --- | --- | --- | --- |
| S1 | 本地归档 | 完成（本地恢复已验证） | `local-archive/` 有完整快照、manifest、校验和恢复记录；源码树移出过程证据和历史切片。 |
| S2 | 测试边界 | 完成（限定工具回归） | 维护中的验收驱动在 `tests/acceptance/`，测试探针在 `tests/support/`，普通构建不依赖测试程序。 |
| S3 | 中文正式文档 | 完成（链接与独立复核通过） | `docs/architecture`、`docs/development`、`docs/testing`、`docs/deployment` 成为正式入口；保留可操作安装/配置/运维步骤，不保留历史流水账。 |
| S4 | 官方 fuser | 阻塞 | 在不降低锁、取消、权限、freshness、close-to-open、错误传播和 direct-I/O mmap 协商的前提下迁移到官方发布版；当前公开 API 不足。 |
| S5 | 候选验证 | Linux 受影响验证完成；CI 依赖声明修复待复核 | Linux 上完成受影响构建/测试；PR 给出候选 SHA、验证和剩余限制。 |

## 不做项

- 不重写 Git 历史，不 force push。
- 不把原始证据、VM 镜像、大型备份或运行日志提交到源码仓。
- 不把 fuser vendor 目录直接删除后宣布完成。
- 不用文档清理、工具检查或提交数量充当产品目标完成。

## 本次快照的可追溯边界

- 归档基线：`6b75d78a1c9350553770394b13384e01a9b292c0`。本地入口为源码仓上一级 `local-archive/README.md`，原件目录 `2026-10-09-6b75d78a/`；构建和维护中的测试不依赖此目录。
- 一次性原源码快照 SHA256：`a5f6b8127d6aeb516113c0a48d6af65d546d9486f40d758b9bb6f8602d357b43`。manifest 与 Linux 实际恢复记录核验了 20,066 个文件及 9 个符号链接的内容、模式和目标；历史 Git 不改写。
- 移出当前树的原路径 19,659 个（约 93.1 MiB）。必要回归输入在 `tests/acceptance/fixtures/` 保留 185 个小输入，因此原路径归档数不是重复文件数；净 tracked 文件减少 19,470 个，当前树约 9.6 MiB。
- 维护中的 Python：`tests/acceptance/` 126 个 runner、driver、probe 和回归模块；其它 `scripts/` 与 `tests/` 共 11 个。另有 22 个冻结 Python 输入只用于验证历史观测的 hash 绑定，不作为维护工具或当前通过证据。5 个旧 VM 固定路径专用启动/重启脚本已归档。
- 当前只迁移工具边界、修复陈旧 helper SHA 绑定、独立 clone 的命令构造回归和工程文档入口。Cargo 与第三方源码未变；Rust 仅更新忽略测试说明中的文档路径，不改变产品行为。

## 当前快照验证

验证环境为既有 `afs-build` Linux ARM64 VM；运行验证使用既有 `afs-g2-micro`，不重建环境。原始命令、日志、失败和文件校验和留在 `local-archive/`。

| 范围 | 当前结果 | 解释 |
| --- | --- | --- |
| 格式、产品 bins、测试 examples、release bins 构建 | PASS | Linux Rust 1.95；产品构建保留两个既有非 RDMA fallback 警告。测试探针仍是显式 example，不属于产品 bin。 |
| Clippy workspace/all-features/all-targets | PASS | `-D warnings`；不等于运行验收。 |
| 配置合约 | 10 PASS | `config_contract`；不能代替完整 POSIX。 |
| 受影响工具回归 | 整组 538 核对；probes 163 PASS | 整组 534 PASS、3 FAIL、1 SKIP；3 个失败均因 root 看不到已有 Hypothesis，在显式绑定同版本依赖后对应 11 项定向回归全部 PASS，不重复无影响项。1 SKIP 为已有可选 STD-01 历史夹具缺失，不计通过。首轮用户/校验和/路径失败均保留。 |
| 打包可复现性、进程控制、试用配置和自检引用 | PASS | 说明文件新路径正确、manifest 一致、测试探针不入普通包；不证明 bind 功能或性能。 |
| 新候选 bind ON 安装运行 | 限定 PASS | `f540c242` 的新 release 包在无编译器 VM 上全新安装；真实 Home bind、UID501/502、64KiB 读写删除、FUSE 正反对照及 4 次实际 wait0。未运行远端、恢复、性能或完整 POSIX。 |
| 官方 fuser 无补丁迁移 | BLOCKED | 锁类型和 interrupt 公开 API 缺口尚未解决；未向上游提交 Issue/PR。 |

新有限运行候选源码为 [`f540c242`](https://github.com/lelezi257/dms/commit/f540c242f26f9815d7ff5f3cd3f1f1babf82af1b)，包 `afs-snapshot-f540c242-linux-aarch64.tar.gz` 仅保留本地归档，未替换 a103 GitHub Release。

- 包 SHA256：`8fb6d5f052f662a93ef76836fc99f573fde16db5268fbc89eac3fb20236d8f30`。
- Meta ELF SHA256：`d5c4f0cdcd9d944cb075a8721c49b61ac5292a65b9d03c06e6650ff31a5f0631`。
- Node ELF SHA256：`3f4a9826a6f40cd7225625dae903c628ca71c0a369195dfafc1b1e66fc8a1640`。
- 本地运行证据归档 `host-bind-f540c242-evidence.tar.gz` SHA256：`d17f198567ac98cbae1ef6dc00ee8809815b707eb83b8034397ec94ee368a7c6`。

这些检查关闭的是本次快照的受影响工程子项，不新增 G1/G2 产品完成项。官方依赖整改 S4 未完成，正式 Agent DX 导入仍未开始。

远端 CI 首轮 [37913786563](https://github.com/lelezi257/dms/actions/runs/37913786563) 在 Check 阶段因工作流未安装 libfuse3 开发包失败，后续步骤未运行；失败日志留在本地。修复只补工作流依赖声明和前置检查，不更改 fuser 源码、编译 feature 或验收要求。远端 CI 通过状态待复核。
