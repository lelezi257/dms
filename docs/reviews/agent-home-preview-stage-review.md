# Agent home 文件系统阶段验收

2026-09-24，测试代码版本 `4eceb8f38145e9b37bfe694df59ce269cdf1c6cd`。这是面向同事测试的候选分支验收，不是正式发布或完整通用 POSIX 验收。

## 结论与边界

**事实。** 本地独享目录的 W1 在两份独立、各六轮采样中，DMS home p50 分别为 328/332 ms，MooseFS 默认配置为 665/668 ms，耗时比为 0.494/0.497，达到“各自不高于 MooseFS 的 0.80”的目标。同场薄 FUSE 为 211/209 ms，Native FS 为 122/121 ms；后两者只是定位本地路径开销的参照，不是本版验收门槛。

**事实。** P2P 远端混合 W2 六轮 p50 为 340 ms，MooseFS 为 345 ms，耗时比 0.985。这个混合用例整体约持平，不能据此声称远端热读胜出：200 文件的重复读阶段 DMS 约 94 ms，MooseFS 约 41 ms。NFS 后端已通过功能验收，尚未做本轮性能优化。

**推断。** 目录在 home 节点时，本机普通文件加薄 FUSE 避开了中心逐文件提交，因此这个工作负载有明确收益。当前 DMS 仍比薄 FUSE 慢约 1.6 倍，绝对性能还有实现空间；真实 Agent 工作区能否普遍命中 home，需要在调度器/同事环境继续验证。

## 可审阅的采样

同一 Linux 三 VM 实验室、相同 200 文件输入、交错执行顺序。下表是整轮耗时，单位 ms；每份 W1 各有一轮预热，预热不计入六轮。p50/p95/p99 均从六轮样本得出，不能解读为长稳分布。完整逐操作收据由仓库内 `scripts/homefs/bench_with_s5_harness.py`、`scripts/homefs/bench_remote_with_s5_harness.py` 生成；下表保留了每一轮的整轮计时。

| W1 场次 | DMS 六轮 | MooseFS 六轮 | 薄 FUSE 六轮 | Native FS 六轮 | p50 DMS/MooseFS |
| --- | --- | --- | --- | --- | ---: |
| 1 | 334.66, 328.25, 322.03, 326.87, 362.68, 346.84 | 682.42, 642.17, 666.19, 659.60, 677.37, 664.74 | 204.36, 241.93, 210.05, 210.72, 267.72, 215.76 | 124.02, 122.41, 119.85, 122.14, 123.78, 114.58 | 0.494 |
| 2 | 369.33, 323.32, 331.83, 336.63, 352.01, 316.86 | 648.76, 670.35, 672.45, 671.82, 667.88, 636.61 | 211.70, 205.40, 208.83, 199.90, 228.88, 245.38 | 128.55, 120.63, 161.77, 113.51, 121.28, 116.49 | 0.497 |

| W2 P2P 远端混合 | 六轮整轮耗时，ms | p50，ms |
| --- | --- | ---: |
| DMS home | 337.57, 353.48, 351.50, 339.88, 344.93, 339.74 | 339.88 |
| MooseFS | 2310.99, 348.86, 345.11, 364.09, 331.78, 341.71 | 345.11 |

W2 第一轮 MooseFS 出现 2310.99 ms，未剔除；交错顺序为奇数轮 DMS 先、偶数轮 MooseFS 先。200 个文件每个 4 KiB，不人为 `drop_caches` 或插入可见性 sleep。对照默认 MooseFS ACK 与 DMS home 本机保存承诺不等物理耐久，因此这里只比较当前用例延迟，不宣称等耐久优势。

测试二进制 SHA256：`fc1a18fe978c18b9c219aaa438b875e81f5afe67c9115621add564cf50e8df6d`。W1 runner SHA256：`13adac69e5948d6edf538f36b5618f5037afd3b5fe8e0c9451cedf251d759b9a`；workload SHA256：`e157ced771cf6297f20c3a0b6efd7f9e5c67904fec57c61afe2da21a2fd783c6`。W2 harness SHA256：`764c80dab0f73133349ccb6136ea876a84a7dab2d18ee0ec74b7e297868a085c`；脚本 SHA256：`748282680f349caa5e58223f5712b1391ef879b35c82b392489d6133eec294a6`。原始完整 JSON 收据仍保存在项目工作区；W1 SHA256：`a2d0a8bd7a0b9f8b1b27141844eaaf8c98be3fa7ef9192f5dc7bd2ecf7699c5a`，W2 SHA256：`8fb73350310091bdeba36b5af47b058aa2776742be729d692dd845e849ae9841`。

## 功能验收与已知缺口

**事实。** Linux 上 `dms-home` 的 34 项单测、格式、Clippy、release 构建及本机两节点真实 FUSE 验收通过。最终安装包在 A/B/C 三 VM 校验 `SHA256SUMS` 后，P2P 与 NFS 两种后端分别通过安装启动、关闭后远端重开可见、远端目录操作、位置查询、中心重启保留归属、home 进程重启从普通文件恢复等步骤。P2P 额外做了无 sleep 的立即 `stat`/重开与删除重建测试，以及 300 次交替长度探针。完整验收 JSON 在项目工作区；P2P SHA256：`ed87a0cee37744c2822e43423f6ca2e69985884565d1b50c869aa2060bda48c8`，NFS SHA256：`6a413bf6ca92ae69ec5b83eb8d44bda45d9b9665e1caf2194c2802674222bd10`。

**事实。** 本地 FUSE 路径的文件执行 `chmod 000` 后，再经 FUSE 恢复 `0644` 会返回 `EACCES`；这是扩大同事测试前应修复的已知权限问题，不影响上述 W1/W2 字节验收。P2P wire 已升到 v3，节点必须同版部署；远端新打开在 home 失联后失败，失联前已打开的小文件只读 FD 可能读出打开时得到的内容。首版删除一级目录保留 tombstone，不支持复用同名目录。

**待验证。** 四节点、完整通用 POSIX、长稳、中心 HA、自动 home 接管、等物理耐久性能对照，以及真实 Agent workload/home 命中率均不在本次结论内。源码构包与安装步骤见[安装说明](../agent-home-preview-installation.md)，架构与语义见[设计](../agent-home-preview-design.md)。

## 2026-09-24 文件身份与失败边界复验

上文是 `4eceb8f` 的历史阶段记录；最新验收以 `6ea3203` 为准。P2P wire v5 把预期底层文件身份随远端 `OPEN` 送到 Home，Home 在 `O_TRUNC` 前校验；NFS 也在截断前检查已打开文件。旧 FD 在改名/删除与同名重建后仍指向原文件，远端丢失修改回复不自动重放。Linux 39 项单测、Clippy、release 构建通过；从最终安装包部署的三 VM P2P/NFS 各 10/10 步通过。

同场 W1 两份六轮 DMS/MooseFS p50 比值 0.386/0.358，P2P W2 六轮为 0.972。逐轮样本与包 SHA 位于项目工作区 `evidence/2026-09-24-agent-home-semantic-closure/`，短结论见 `outputs/reports/2026-09-24-agent-home-semantic-closure.md`。这些数值不代表等物理耐久，也不证明四节点、全 POSIX、掉电切点、旧 FD 跨进程重启透明续接或自动 Home 接管。

## 2026-09-24 故障切点与 inode 复用复验

最新安装包来自 `ecb5b9b`；上文数据作为历史记录保留。本次 P2P wire v6 使用 Linux 不透明 file handle 区分同一底层 inode 号快速复用；底层不支持该能力时远端操作显式失败。一级目录创建、删除在 Home 数据根目录 `fsync` 后才推进中心状态；重启先对账，再提供 FUSE 服务。

Linux 安装包三 VM P2P/NFS 各 10/10 步通过。另将六个根目录置于预约、删除中、tombstone 和活跃的中间点，`SIGKILL` 中心及 Home 后重启均按盘上实际状态收敛；仅预约而目录未落盘会撤销预约，重试 `mkdir 0710` 保留权限；活跃目录若物理消失则拒绝 Home 启动。VZ `--force` 停 A 后 B 无法接管，原 VM 回来后文件可由 A/B 重开。该实验先显式同步本地数据，不等于物理存储控制器断电证明。

本地 W1 两份独立六轮 DMS/MooseFS p50 比值为 **0.409/0.400**；同场 DMS/薄 FUSE/Native/MooseFS p50 分别为 281/218/130/686 ms 和 263/218/122/657 ms。远端 W2 六轮 P2P 为 **0.969**（332/343 ms），NFS 为 **7.337**（2563/349 ms）；NFS 只保留后端功能，性能优化后置。完整逐轮值、构包 SHA、脚本及清理收据见工作区 `evidence/2026-09-24-agent-home-fault-closure/`。首次穿刺包的 Linux 42 项单测、Clippy 和 release 构建通过；随后增强 `OPEN(O_TRUNC)` 故障测试。默认 MooseFS 与本机单副本保存仍非等物理耐久对照。

上述是 `ecb5b9b` 的首次穿刺；最终安装包重新从 `c17bef8` 构建，包 SHA256 为 `f5fcdd8e81003611f9c1d995ecca173b25ea6a7403f539d512e9481f49fd452a`。该包重新跑过六个目录故障切点、活跃目录缺失负例、A 的 VZ 强制停/启、三 VM P2P/NFS 各 10/10 步。最终 W1 两份独立六轮 DMS/MooseFS p50 比值为 **0.402/0.405**，DMS/薄 FUSE/Native/MooseFS 为 264/210/119/657 ms 与 260/218/124/643 ms；W2 P2P **0.978**（330/337 ms），NFS **7.251**（2525/348 ms）。Linux 43 项单测、Clippy、release 构建与 GitHub x86 `source-check` 通过。最终包和逐轮收据在工作区 `artifacts/homefs/dms-home-fault-c17bef8.tar.gz` 与 `evidence/2026-09-24-agent-home-fault-closure/`。真实控制器断电与等物理耐久仍未证明。
