# AFS 交付开发接手指南

日期：2026-09-30。此文件是跨电脑接手入口；验收目标以 [acceptance.md](acceptance.md) 为准，能力状态集中在 [status.md](status.md)。当前是可编译、可回归的开发检查点，尚未达到交付验收。

## 1. 目标与约束

**决策：** 完成交付水平的 OwnerFs 和 DistributedFs（DFS），提供 Linux 一键安装和进程部署，支持 memory 开发后端及 etcd/Redis 持久后端。Redis 替换 Meta 持久后端。第一阶段不包含 systemd、Kubernetes、DFS SDK、产品数据缓存、spill 和 Meta 选主/HA。

- OwnerFs 面向 1～4 节点 workspace；本地访问正常本地文件，远端访问 Home。DFS 是通用多读多写文件系统，主要优化镜像和大文件加载。
- OwnerFs/DFS 都属于 `afs-node` 的 VFS 子模块，共用 FUSE，使用独立 mount。应用访问 Node；Node 访问集群内的 Meta。文件内容不经 Meta。
- RPC 边界保持 `rpc/meta.rs`、`control.rs`、`data.rs`、`peer.rs`。控制面和数据面职责分开，gRPC/RDMA 是传输选择，不建立两套业务状态机。
- DFS 基座是不可变 Chunk，文件可变性由 dirty overlay 和新的 FileVersion/ExtentMap 表达。无需 Blob 专用发布接口。
- 一致性对齐 JuiceFS 默认的同 mount 可见性及 close-to-open；不能用“所有远端已打开句柄实时看到 dirty overlay”替代合同。成功 writable close 的 flush 承担提交并报告错误，release 只做清理。fdatasync 提交恢复数据所必需的布局、长度和版本，fsync 同步完整 inode 属性；父目录需独立 fsync。
- 同 inode 的 write/resize/sync 由 owner 串行处理。Meta ACK 丢失时保留原 OperationId、精确请求、FileVersion、LayoutRoot 和 receipts，阻塞该 inode 后续修改并以同一身份确认；应用超时不清除待确认状态。
- 副本 N 和同步最小数 M 是初始化配置。R1 本地写；RN 在 Chunk 层完成副本策略，上层布局不分叉。RDMA 必须实际交付，RXE 验证功能并尽力优化，没有 RDMA 速度门槛。
- 本地主功能及性能开发先用 memory Meta，etcd/Redis 留到核心功能/性能开发后独立验证。memory 不证明 Meta 重启持久性，也不能充当 durable 性能基线。

**性能门槛：** OwnerFs 本地读/写任务耗时分别 ≤ 同条件 MooseFS 的 0.5，远端分别 ≤ 0.8；DFS 各适用任务耗时 ≤ 同条件 3FS。固定副本、持久化、接口、资源和数据集，不允许折算或放宽。完整门槛和 TODO 见 acceptance。

## 2. 总体计划与当前断点

| 阶段 | 要交付的结果 | 当前事实与下一出口 |
| --- | --- | --- |
| P0 环境/测试/基线 | 四 VM、上游参考测试、明确适用性、真实比较系统、可信 runner | 已准备大量脚本和参考短测；环境锁仍 PREPARING，MooseFS 强耐久匹配及 3FS 参考资格未完成 |
| P1 安装纵切 | 无编译工具的安装、独立 mount、创建写读关闭重开 | 隔离安装/重装保留/进程短测已有证据；完整 DEP 矩阵未运行 |
| P2 文件语义 | POSIX、同 mount/close-to-open、权限/namespace、远端 owner、锁 | 当前 v25 源码回归通过；新锁/权限修复尚需部署后的跨 mount 及完整上游复验 |
| P3 副本/传输 | RN、P2P、多源读取、授权、持久化修复任务、真实 RDMA | gRPC/RXE 短测存在；完整多 VM 故障、安全和修复矩阵未完成 |
| P4 存储/运维 | 有界内存、崩溃恢复/GC、容量/损坏、REST/指标/诊断 | 部分路径和短测存在；资源、故障、恢复门禁待完成 |
| P5 性能 | 合格基线上的配对结果及优化 | 尚无可以声明满足上述比例的结果；先关闭基线资格问题 |
| P6 最终交付 | 全功能矩阵、8 GiB、FSx、随机测试、8h、完整部署和最终 review | 尚未执行，不能以短测或单元测试替代 |

详细依赖和用例对应见 [development/plan.md](../development/plan.md)，规则见 [implementation.md](../development/implementation.md) 和 [validation.md](../development/validation.md)。小范围实现选择自主完成；核心架构/接口的必要调整保留原因和证据，最后统一评审。

## 3. 已验证的检查点

### 当前源码候选 v25

**事实：** 223 文件的 Linux 构建快照稳定捕获，基于历史 Git HEAD `6ee3f177a43ee85cc6b79666502330095d445fdb` 的未提交修改。它不是当前 Git HEAD；本检查点代码随后统一提交。快照 tar SHA256：

```text
09d08b944c81e6f3141efd6fa83dd046f432f8e2efab764e0403b82e2a0e1992
```

Linux Ubuntu 24.04.4 ARM64、内核 6.8.0-142、Rust 1.95.0：fmt、workspace 全 targets/features 严格 Clippy、库回归 **258 PASS / 0 FAIL / 2 显式环境 probe ignore**，以及全 features Node/Meta 构建通过；另有 53 项接口、4 项公共错误、5 项真实 FUSE 测试和 none/ownerfs/dfs feature 编译矩阵通过。两项忽略分别需要独立 durable Redis 和显式 RXE fixture，不是后端/RDMA 验收通过。迁移输入的 77 项 Linux 自检和 Skill 格式验证通过。附加接口与真实 FUSE 结果见 [检查点证据](../development/evidence/20260930-checkpoint/README.md)。

```text
afs-node  00fc42564f8c45f4de29862d1bc42ff76c9f0df53a74ecb599d308b7e24c30ab
afs-meta  7f6fc7fc857e73d87679ea2c4ac6454e06812515d3054730a94d66dcb7ad30bc
```

**事实：** 当前运行的 memory 实验仍是较早 v11，并非 v25。v25 尚未部署到 A/B 产品双 mount。文档、可迁移测试输入和 Skill 在快照后加入；它们不改变已编译 Rust 文件。Git 内保留源码哈希和原始短测日志，二进制需在新 Linux 环境重建，不保证跨机器产生相同二进制摘要。

### 已有运行证据的范围

- v11 实际 DFS 的完整 pjdfstest：236 文件、8819 检查、0 unexpected、28 TODO。OwnerFs 同套完整检查有 **2 unexpected**：`ftruncate/00.t` 和 `unlink/14.t`。源码已修复，但新候选实际 mount 复验仍待执行；不增加排除。
- 实际 A/B RXE 产品 FUSE：R2 同步写、R1 远端读各 4194321 B 内容正确、gRPC 文件载荷为 0，B 副本重启后可读。这是独立较早候选短测，不覆盖当前 v25 的完整故障/性能门禁。
- ext4 LTP 参考：657 命令、605 PASS、50 TCONF、2 TBROK，无普通 FAIL/TIMEOUT；仍需冻结 event-level 适用性，全部命令保留。参考结果不是 AFS 通过。
- FSx seed1/1000 操作和真实 Hypothesis prefix replay 有短测。完整 3 seeds×900 秒 FSx、10 seeds×10000 随机操作、8 GiB、8 小时长稳均未运行。
- 3FS ARM64 scratch Folly 指针兼容 patch 后实际 RXE/FUSE 32 MiB 短写读通过；此为 patched 参考，不能声明 vanilla 或完整基线已资格化。MooseFS stock fsync 的物理持久性合同缺口未关闭。

**待验证：** 全部 69 个正式 release case 仍为 NOT_RUN，ENV 仍 PREPARING。上述开发证据不自动改变正式结果。

## 4. 换电脑接手

### 获取完整输入

```sh
mkdir -p afs-work
cd afs-work
git clone https://github.com/lelezi257/dms.git source
cd source
git rev-parse HEAD
git status --short
mkdir -p ../experiments
cp -a development/acceptance ../experiments/afs-acceptance
```

已有目的目录先比较，不覆盖独立修改。`development/acceptance` 是 versioned 输入，运行布局使用它复制出的 `<work>/experiments/afs-acceptance`，不要直接在 bundle 内跑那些依赖 parents[2] 的脚本。详见 [PORTABILITY](../development/acceptance/PORTABILITY.md)。仓库包含 [.codex/skills/afs-acceptance/SKILL.md](../.codex/skills/afs-acceptance/SKILL.md)；AI 可直接读取，不依赖原电脑个人 Skill。

原始完整 VM 日志、磁盘内容、TLS 私钥、工具二进制和 build cache 不在 Git 内。重建环境、重新生成 TLS、重新观察身份；不要把旧机器 PID/IP/SHA 填进新证据。大批历史原始日志在原研究区 `evidence/afs-delivery/`；关键当前源码短测已随本检查点保存。

### 严格保持环境

| VM | CPU/RAM | 磁盘 |
| --- | --- | --- |
| ctl | 2 vCPU / 4 GiB | root 24 GiB + state 8 GiB |
| A/B/C 各自 | 2 vCPU / 6 GiB | root 24 GiB + data 32 GiB |

ARM64 Ubuntu 24.04 LTS、同镜像/内核、guest ext4、真实 RXE；完整细则见 acceptance §3。源码可共享，测试数据不可放 macOS share/virtiofs/tmpfs。构建用独立 Linux VM，不与性能测试争资源；原机器 `afs-build` 是 4 vCPU/8 GiB/root64 GiB。x86 新电脑不能直接换掉既定 ARM64 基线或拿模拟成绩混比，应保持 ARM64 远端验收机器并记录实际条件。

VM YAML 中原 host 路径、guest 用户、镜像缓存 URL 是迁移参数，按新机器调整并验证 pinned image SHA。网络重新分配和观察，保留隔离/MTU/RXE 约束。`acceptance.lock.json` 和 `network-leases.json` 仍是准备输入，不能作为新机器已冻结证明。

### Linux 快速验证

在 Linux 可写源码副本执行，按 rust-toolchain 安装 Rust 1.95.0、rustfmt、clippy 和 build 前置依赖（[prepare-build.sh](../development/acceptance/prepare-build.sh)）。只读共享目录不能直接 cargo fmt。

```sh
export PATH="$HOME/.cargo/bin:$PATH"
export CARGO_BUILD_JOBS=2 CARGO_INCREMENTAL=0
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test -p afs --lib --all-features
cargo test -p afs-error
cargo test -p afs --all-features --test error_contract --test meta_contract --test ownerfs_peer_contract --test rest_contract --test config_contract --test vfs_contract --test fuse_contract
cargo build -p afs --all-features --bins
# Linux /dev/fuse + fusermount3；必要时 sudo env 传入 PATH/target。
cargo test -p afs --all-features --test fuse_contract -- --ignored --test-threads=1
```

每条用 timeout 包裹，保留 exit code/raw log/source SHA；真实 FUSE 测试确保权限可用。新机器重建二进制后记录新摘要，运行前验证 `/proc/<pid>/exe` 和实际 mount/backend。`--all-features` 不代替无默认 feature/组合矩阵。

迁移后的 runner 自检从 `<work>/experiments/afs-acceptance` 在 Linux 执行 `python3 -m unittest discover -p 'test_*.py'`，安装真实 Hypothesis 等前置依赖。先看 runner/probe 的 `--help`；全部 TODO driver 必须保持 NOT_RUN，不能因为 selftest 成功改为 READY。

## 5. 第一项接续任务

1. 保存旧 memory lane 的日志/fixture，核对运行身份，再用 **旧 v11 二进制身份** 停止它；不要复制新二进制覆盖 live executable。新环境没有旧 lane 时无需执行历史 stop。
2. 部署已校验的 v25 或接手后重建候选，A 的 isolated memory lane 信任 A/B；B 启动真实独立 Node 和 mount。`p2-memory-lane.sh` 有端口/mount/live-PID 预检，默认历史 `.local/p3a-binaries` 路径须显式覆盖为本次候选。`start-memory-peer.sh` 在 B 内 root 执行，要求 EXPECTED_NODE_SHA。
3. 使用 `probes/locks_cross.py` 验证同一个 inode 的两个 mount：fcntl/flock 冲突、阻塞超过 5 秒、unlock 后继续、精确 cancel、close/session cleanup。严格 errno 判断，ENOLCK/ENOSYS/EOPNOTSUPP 不能冒充锁冲突。ext4 同路径自检的 `cross_mount_qualified=false` 不算产品双 mount 通过。
4. 新候选复验 OwnerFs 两个 pjdfstest 失败文件，再跑完整 236 文件，不增加排除；同时核对 namespace 权限/SGID/sticky 和远端 DFS owner 文件操作。
5. 按 P2→P3→P4 依赖继续功能、故障和资源验证，完善正式 drivers。基线合格后进入 P5；持久后端 parity/recovery 随后完成，完整长测最后执行。

这些是依赖顺序，不是评审停点。小范围问题自主修复、写失败回归、重新跑 Linux 证据。硬门槛仍缺失时继续实施，不宣称总 goal 完成。

## 6. 原机器断点与待评审项

**事实（仅原机器）：** ctl/A/B Running，C 因资源预算暂时 Stopped；build 独立 Running。ctl/A/B/C 地址观察值 `.11/.12/.13/.14`，CIDR `192.168.109.0/24`。新机器必须重新观察。

- memory v11 在 A：Meta TLS/REST 17800/17801，Node 17802/17803，lane `/mnt/lima-afsadata/afs-delivery/p2-memory-lane`；Meta PID169264 SHA8d4af9c9…，Node PID169294 SHAbd25c7e3…。
- B memory peer 计划 17804/17805，`/mnt/lima-afsbdata/afs-delivery/p2-memory-peer`，尚未启动。历史 174xx/175xx etcd lanes 保留，不能当作当前功能主 lane。
- build 工作副本 `/home/lzc.guest/afs-build/work/root-merged-v25`，target `/home/lzc.guest/afs-build/target`。ctl 的 LTP pinned 源码已移到 `/home/lzc.guest/afs-tools/ltp-src`，原路径保留 symlink。

**待评审/待验证：** 本次检查点保持核心目录/RPC 分工，新增 `src/node/vfs/locks.rs` 公共锁协调、`src/meta/store/redis.rs` 后端、`scripts/deploy/` 部署及测试；扩展现有 proto/FUSE vendor 以携带精确锁身份、中断、权限和平台 ABI。错误码增加 Interrupted/WouldBlock/Deadlock/NoLocks。远端锁权威在 Home/inode owner，不新增 Meta 锁表。

锁退休身份采用保守有界集合（默认 4096 terminal/retired），不会为了新 admission 清除已有活锁；长活会话可能耗尽资源并返回 ENOLCK。已做容量/乱序/cancel 回归，8 小时门禁和稳定资源策略仍需验证。远端清理失败保留目标和身份，维护 tick 重试；其多 VM 故障覆盖仍待执行。Meta 全快照的序列化/空间成本、etcd 64 MiB 过渡配置、pack/GC/compaction、完整 repair 和多副本崩溃矩阵仍需处理；不为快照大小问题长期阻塞功能主线。

## 7. 给下一个 AI 的任务入口

```text
读取 AGENTS.md、PRINCIPLES.md、docs/handoff.md、docs/acceptance.md、docs/status.md 和 development/{implementation,validation,plan}.md，应用仓库内 afs-acceptance Skill。目标是完整交付 OwnerFs/DFS，保持既定目录/RPC和验收门槛。先重建或核对 ARM64 Linux 环境及当前源码/二进制身份，从 handoff 第5节的真实双 mount 锁与 OwnerFs POSIX 复验继续；memory Meta 为核心开发主 lane，etcd/Redis 后续验证。自主完成小任务，保存失败/成功原始证据，不以历史结果代替新候选，不以短测代替完整验收。实现和证据提交 GitHub，维护 status 与本接手断点，最终统一输出必要的架构/接口变更及未完成门禁。
```

新接手者不必恢复原对话、原个人 Skill 或原 `.local`。保留完整目标，依据真实证据推进；更新本断点时只保留当前有效入口，历史原因由 Git/原始日志追溯。
