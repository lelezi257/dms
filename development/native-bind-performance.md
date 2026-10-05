# OwnerFs native bind：架构判断与首轮实测

日期：2026-10-01。沿用 [RFC 0001](../docs/rfcs/0001-ownerfs-native-bind-mount.md)、[验证计划](native-bind-validation-plan.md)与既有 E1～E16，不重启方案。Issue [#42](https://github.com/lelezi257/dms/issues/42)。本文件是当前候选的实验结论，不是发布验收或已实现能力声明。

## 明确结论

1. **同路径 bind、同一 backing、已接受的文件对象/close-to-open 边界可行。** E11～E13 在真实 Linux VM、当前 Node/Meta/TLS/P2P 链路给出证据；E16 另证明 native Home 与 ordinary remote 的两个重开案例。完整挂载过渡、生产 READY 与生命周期集成仍有缺失，不能从这些局部证据推导全部通过。
2. **当前标准 FUSE/P2P + native 混合写入不能满足完整追加/偏移合同。** E15 的一次远端 2MiB write 被 native 写入分隔，顺序追加也返回错误 SEEK_CUR。FUSE 按分块请求处理、WRITE 回复仅返回字节数；Home inode 的追加位置/锁与远端 FUSE inode 不同。没有整次 syscall 边界与结束偏移，仅在 Home 加用户态逐请求锁不能完整修复。此结论针对当前边界，不是对所有 bind/定制客户端方案的不可行性证明。POSIX owner 的简单代理也被 E4 否定，完整替代路线未证明。旧 cwd/dirfd 的已接受边界不豁免这些要求。
3. **仅覆盖 workspace，当前零 TTL 策略没有达到完整绝对路径的 native 性能。** 以下实际数据关闭“是否只要 bind 成功就达到 native”的问题。进入 native cwd 后的数据路径接近 ext4；原路径绝对访问仍有稳定、大幅的祖先查询开销。不能要求使用者改名或只使用相对路径来宣告原目标实现。

因此，当前候选的完整架构与性能验收均未通过。bind 基础机制有价值，但还需要解决路径解析及混合访问仲裁；通用重试、排空工程不能代替这些架构决策。

## P1 本地元数据/小文件数据

在 A VM 运行同一 `gcc -O2` syscall 工作负载，10,000 个 4KiB 文件，按 create/write/close、stat、read/close、readdir、rename、unlink 分项计时。每个路径/并发组合先预热一次，再交替运行五组成对 ext4/native；MooseFS 同条件串行插入。计时不含 SSH、控制文件轮询、初始 mkdir/chdir 与结果序列化。读取逐文件校验完整内容，stat 校验长度，readdir 校验条目数；失败即中止该样本。

下表任务耗时为六个分项的合计中位数。**比值是逐轮成对比值的中位数，不是两列中位数相除。**

| 访问方式 | 并发 | ext4（ms） | native bind（ms） | native/ext4 成对比值 | MooseFS（ms） |
| --- | ---: | ---: | ---: | ---: | ---: |
| 绝对路径 | 1 | 332.75 | 6496.20 | **19.276** | 35924.32 |
| 绝对路径 | 8 | 353.55 | 1328.50 | **4.161** | 21255.03 |
| native cwd 相对路径 | 1 | 328.48 | 328.12 | **0.992** | 37461.68 |
| native cwd 相对路径 | 8 | 332.90 | 328.38 | **1.036** | 21705.58 |

绝对路径并发1的五个比值为 20.923、19.276、17.763、20.261、18.607；并发8为 3.523、4.176、4.161、4.267、3.430。相对路径并发1为 0.981、1.040、1.002、0.992、0.985；并发8为 1.036、1.062、0.937、1.255、0.986。前两组是稳定退化；后两组只支持“接近原生”，没有证明每个分项均满足既定 `median(T_native/T_ext4) ≤ 1.0`，不自行加入5%/10%宽限。

完整分项、每轮 wall/client CPU、操作 p50/p95/p99、配对来源与无效配置标记在 [机器可读数据](acceptance/results/native-bind-p1-20261001.json)。尾延迟是每轮实际操作分布，不把五个任务耗时当作可靠 p99；未提供跨轮合并的 p99。Root/epoch/Home/namespace、Node SHA、卷及资源前后记录保留在原始归档。

### 可比性与限制

- A：Ubuntu24.04、kernel6.8.0-142、2vCPU、约5925MiB RAM、无 swap；同一个 `/dev/sdb1` ext4 数据卷。ext4 与 native 两条路径访问同一 Home source inode，测试目录各自新建并清空；MooseFS 唯一 chunkserver 也在 A 同一卷，master 在 ctl。C 仅编译基线。没有同时运行其他 benchmark。
- DMS 使用新构建 `release [optimized]` 的 Linux lib-test Node driver 和优化版 Meta，沿用真实生产 bootstrap/TLS/P2P；生产 native admission 仍未接入。MooseFS 固定 commit `ac106b2ec8661ff00def725d042cb67d3ca2184d`，实际版本4.59.2-1/build2106，默认客户端缓存、nice0；专用 `native-bind-one` 类定义为单份 `*`，只有 A 注册 chunkserver。
- 这是新文件创建后依次 stat/read 的 warm 可见性工作负载，不是冷读、持久化写或普通 FUSE-only 产品基线。native 与 ext4 都是普通 close；MooseFS 使用 stock close/flush，`HDD_FSYNC_BEFORE_CLOSE=0`、`CHANGELOG_SAVE_MODE=0`。**B001 物理数据/CRC/元数据持久性对等性仍未资格化**，这些数据不证明 durable-write 比值门槛。
- 绝对路径的实际前缀长度/层数不同，报告源码与路径；后续优化后的严格微小差距需控制此因素。它不能解释更短 native 前缀反而出现的 4～19 倍退化。相对操作路径相同，初始导航成本另计。Hyper-V/Windows 宿主缓存未完全控制；本轮不宣称 cold。
- 第一个完整计时 run 的长时间采样超过语义 Actor 的五分钟寿命，收尾失败。72个计时任务已完成，原始 runner 仍为 FAIL；独立校验核对归档/实际回复/计时/二进制/退出，确认 Node/Meta 正常退出及正常 detach，仅资格化计时诊断。不能改写失败为全实验 PASS。
- 原完整 run 的绝对路径/并发1早期 MooseFS 仍为默认2CP；轮1、2不用于单副本比较。另一个短 run 在正确单副本配置下完成预热加两组成对补充，正常退出，补足这组五个 MooseFS 配对。数据明确记录跨 run 身份及被排除样本，不覆盖或静默替换历史记录。

## 原始输入与复核

- 完整计时：`network-probe-20261001T120749-90422701`，72样本，原 runner FAIL/Actor lifetime；独立计时核验通过，非阶段通过。
- 有限补充：`network-probe-20261001T122407-211d5bd3`，9样本，原 runner PASS/正常 teardown；只补绝对路径并发1，不替代全矩阵。
- release 构建记录：`/home/lzc/workspace/dms/evidence/ownerfs-native-bind/20261001/native-performance-build-20261001T115511/`，Node/Meta 构建均 terminal0。Node 原 SHA `89ae34954a8fadcb0217decf02d3be2ad3d9d508df27fc4924678691a9eec316`、执行 SHA `409363724228d3a1de051158d6ec9e627452c55af0b95d9f4607feb5c3eb5c72`；只移除 DWARF。Meta 执行 SHA `77b9435f23c198de3e53d3552979ce2beca1ad79f05e8fafdc19bdfab6796cfe`。
- benchmark SHA `a5a7c66a2c38e521d23d0f857b4bd732dd0e3a33f74c7b136c1fa879bcaf7ff3`；源 [ownerfs_native_benchmark.c](acceptance/probes/ownerfs_native_benchmark.c)，C VM严格编译及真实 ext4 行为自检通过。
- verifier SHA `c7e63f53b1a63dea3fbb5313d21c41ffc6ad8cd1f8548dd185a8eab42a07cc7e`。两个 run 的原始证据通过，坏归档SHA、伪造读回复、伪造计时均被拒绝。E13/E14/E15/E16重新离线复核保留各自原结论，包括E15语义FAIL。
- Windows 原始副本 `C:/workspace/code/dms/local/native-bind-vm/<run>/`；A/B VM `/mnt/afsdata/ownerfs-native-network/<run>/`，ctl `/mnt/afsstate/ownerfs-native-network/<run>/`。原始归档 SHA 与结果 SHA 收录机器数据；MFS stock 包 SHA `0999130ea116a024452755e9bd04cc803533c6a27c3d0079649d160645b7c9e7`。

## 核心容器 Agent 场景：C0/C1 与容器 P1

用户补充的核心流程是：管理面在 OwnerFs 根创建 workspace，确认 native 挂载后，只把该 workspace 挂入容器交给 Agent。此场景已经在 A 的真实 runc OCI 容器运行，独立于上面的宿主路径结果；没有要求 Agent 改目录名或仅使用相对路径。

`network-probe-20261001T125205-fc4c5860` 的流程与证据：

1. 沿用优化版 Node/Meta/TLS/P2P。FUSE mkdir → manager native activate，原 export 的 source 为 ext4 `device=2065,inode=1049463`。容器运行时先进入已核验的 Node mount namespace，再按**原 export 路径**进行 bind；native lane 未直接改用隐藏 backing 路径。
2. 容器内 workspace 是 `/ownerfs/agent1`，父目录是最小只读 rootfs 自身的 `/ownerfs`。native 与 ext4 对照最终都绑定同一个 source inode；容器的 mount/PID/network/IPC/UTS/cgroup namespace 与 Node 不同。native 容器内的实际 workspace mount 为 ext4、`statfs=0xef53`，其 mountinfo 无 FUSE mount。
3. 三个实际容器内 `/ownerfs` 仅列出 `agent1`；创建父级条目返回 EROFS。宿主上已存在的兄弟 workspace/外部 marker，通过 `..`、宿主绝对路径、backing 父路径、symlink 和 host PID/proc-root 均返回 ENOENT/EACCES/ENOTDIR。自身原文件内容读取正确，创建/写入的 marker 在实际 source 上核对成功。
4. Agent 实验进程为 UID/GID0，但全部 capabilities 为空、NoNewPrivs=1；只读 rootfs、独立 PID/proc 与 namespace，未挂宿主 root/proc 或整个 OwnerFs 根。这里证明的是上述配置的宿主**目录数据隔离**；Linux mountinfo 仍可显示 mount-root 路径字符串，没有承诺隐藏挂载元数据、证明任意恶意代码无逃逸，或已经支持任意 UID/user-namespace 映射。
5. benchmark 在容器内运行，runtime 启动/exec/SSH 均不计入 syscall 阶段时间。三方使用相同 rootfs/runtime/目标路径深度与长度/权限/数据/轮次；ext4/native 是同一个物理 source，不同等长后缀的新实验子目录。容器祖先本身为 native，因此绝对访问无需经过宿主 OwnerFs 根的 FUSE 查询。

容器同一 P1 工作负载（一次预热、五轮正式交替配对，三方共72任务）的结果如下。完整任务仍为六个分项合计；**比值取逐轮配对中位数**，与两列中位数相除不同。

| 容器内访问方式 | 并发 | ext4（ms） | native（ms） | native/ext4 成对比值 | MooseFS（ms） |
| --- | ---: | ---: | ---: | ---: | ---: |
| 绝对路径 | 1 | 333.08 | 330.03 | **1.050** | 36059.09 |
| 绝对路径 | 8 | 337.18 | 343.54 | **1.019** | 20866.04 |
| 相对路径 | 1 | 360.17 | 339.22 | **1.003** | 36287.03 |
| 相对路径 | 8 | 343.00 | 339.04 | **0.995** | 21560.62 |

这关闭“容器里是否仍有宿主 FUSE 祖先的数量级退化”的问题：本轮没有4～19倍差距，路径机制符合原生访问预期。但24个分项/路径/并发组合中16个配对中位比值大于1，且逐轮比例跨过1；不能把这轮波动数据判成全部严格 native 门槛通过，也不自行加入5%容差。它是近原生的实际证据，性能验收仍开放。MooseFS 单副本 stock 可见性对照延续 E17，B001强持久性仍未资格化。

原 runner PASS，三个容器都受控停止/delete 后才 detach，Meta/Node/短期 bootstrap Actors 全部正常退出；独立复核72个实际 runtime 回复、source、spec、capabilities、namespace、mountinfo、输入/逐文件/归档 SHA 和完整采样矩阵。伪造 timing/source/隔离结果、坏归档、伪造远端读取均被拒绝。

- 完整表格/全样本：[container P1 数据](acceptance/results/native-bind-container-p1-20261001.json)，SHA `193107e6b3ae4586d1e5915d92f8fef85673fb95a707b9e2da117fbdb4772de8`。
- runc `1.3.4-0ubuntu1~24.04.1`（OCI spec1.2.1），执行 SHA `bdce4d45b2dd217491db8a98c8484b161e225ce49c15ebc1ba42077fb7c07d50`。这是实际 OCI runtime 实验，未宣告 Docker/Podman daemon 已完成集成。[OCI root/mount 规范](https://github.com/opencontainers/runtime-spec/blob/main/config.md)、[runc 文档](https://github.com/opencontainers/runc/blob/main/README.md)。
- probe 执行 SHA `dcacaa08aff4edf5cdbd2c6e04ba3a3d2872a17c11209f58dc959328c3e8ff94`；冻结输入 `native-network-native-candidate-20261001T125157`，源码见 [容器驱动](acceptance/probes/ownerfs_native_container.py)和 [进程探针](acceptance/probes/ownerfs_native_container_probe.c)。
- 原始 A/B/ctl 归档分别为 `e39f2f77b90ac0b2b0049c22ede0b1d64666ce73246223c158cbe03e5017a566`、`437c62148f28459db18860bfd57899260d6719bd927ae5b225511bcbfe039d67`、`aee67cd89ea1f8b57d7ae1657986aea3522a02f038f5f4990a98be4ff1b58449`。完整身份/spec/原始回复已保留，未打包宿主数据或大工作集来替代验证。
- 首次 `network-probe-20261001T124743-f5c49050` 为观察器 FAIL、零计时样本：detached init 继承采集 pipe，造成 runtime 已退出但 communicate 等待；PID1 默认 TERM 又不能终止 sleep，改为专用带 TERM handler 的 init；归档跟随 `/dev/core` symlink 读取 `/proc/kcore`，改为只校验常规文件且不跟随 symlink。失败输入/人工核验的 PID/start/boot/exe SHA、收尾与恢复归档均保留，恢复 A 归档 SHA `aa3c90ff621d65b386fa16fd934ca23cea34677e99ea96e452873c4802a1ddce`。原 FAIL 不改写为 PASS。

生产管理面须把原 export 发布给实际 runtime 所在 namespace，核验最终 Agent 对象，再 READY/启动；宿主 daemon 仅拿到路径字符串并不满足这一条件。此处没有 Docker daemon 已完成发布的声明。

## 容器顺序、随机 IO 与跨 namespace 卸载：E19

`network-probe-20261001T131137-5e41ff06` 已完成实际 OCI 容器的完整本地 ext4/native 对照：30个负载/并发组合，每组一次预热、五个交替配对，共360任务。原 runner、独立归档/source/spec/runtime 回复复核和正常进程收尾通过。**这是架构尚有未决项时取得的有界诊断数据，不表示已跨过阶段一，也不代表完整性能阶段通过。**

计时为实际 `open`、线程启动/读写/回收、一次最终屏障及 `close`；读取逐块比对内容，写入在计时后重新 open 并重放所有请求偏移核对内容和长度。总 IO 不随并发增长；顺序块1MiB、文件/总IO均8GiB；随机块4/64KiB、总IO512MiB。随机冷读工作集8GiB，热读写工作集512MiB。`fdatasync/fsync` 是整个任务最后一次文件屏障，**不是每次小写后同步**；未测试创建/rename 的目录持久性或 VM 掉电。资源前后窗口还包含运行时及计时外校验，不把整个窗口当作 timed IO CPU/磁盘量。

每个 guest-cold 样本先记录全局 sync/drop_caches，然后在计时前对目标 fd 执行 fsync、DONTNEED、mincore 检查，最多三次准备，驻留量必须为0；hot 必须在计时前全量驻留。8GiB重复顺序读超过约5.9GiB guest RAM，明确标为 repeat，不能冒充全热读。Hyper-V/Windows 缓存仍未完全控制，guest-cold 不等于物理磁盘 cold。计时后驻留量也包含写入内容校验的读取。两个 lane 同 source/卷和等长可见路径，但各自数据文件的 extents/inode 不同，因此微小差异的原因不能单凭本轮归于 bind。

下表是五个正式配对的任务耗时比中位数；越小越快。全部实际 wall、MiB/s、client CPU、最终屏障时间及每轮操作 p50/p95/p99 见[完整360样本数据](acceptance/results/native-bind-container-io-20261001.json)，不把任务中位数或五轮样本当作稳定尾延迟。

| 负载 | 缓存 | 屏障 | native/ext4，并发1 | native/ext4，并发8 |
| --- | --- | --- | ---: | ---: |
| 顺序写 1MiB | guest-cold | close | 1.017 | 1.041 |
| 顺序写 1MiB | guest-cold | fdatasync | 1.089 | 0.908 |
| 顺序写 1MiB | guest-cold | fsync | 1.004 | 1.011 |
| 顺序读 1MiB | guest-cold | close | 1.032 | 0.998 |
| 顺序读 1MiB | repeat | close | 1.013 | 1.090 |
| 随机读 4KiB | guest-cold | close | 0.934 | 0.979 |
| 随机读 4KiB | hot | close | 1.053 | 1.023 |
| 随机写 4KiB | hot | close | 1.005 | 0.992 |
| 随机写 4KiB | hot | fdatasync | 1.018 | 0.988 |
| 随机写 4KiB | hot | fsync | 1.048 | 0.957 |
| 随机读 64KiB | guest-cold | close | 1.017 | 0.996 |
| 随机读 64KiB | hot | close | 1.135 | 1.055 |
| 随机写 64KiB | hot | close | 0.926 | 0.939 |
| 随机写 64KiB | hot | fdatasync | 1.036 | 0.834 |
| 随机写 64KiB | hot | fsync | 0.951 | 0.885 |

范围0.834～1.135，17/30个组合中位比值大于1。固定采用原严格 `median(T_native/T_ext4) ≤ 1.0` 标准时，本轮**未全部通过**；不自行扩大容差。没有宿主 FUSE 祖先的数量级退化，但偏向某 lane 的冷 IO、热64KiB读/写差异仍需归因。为排除不同文件布局和页缓存对象，预先限定四个同一文件 inode 的并发1补充案例：顺序 fsync 写、冷4KiB随机读、热64KiB随机读/close写；该补充不覆盖或替换完整矩阵。

### 四个同文件 inode 的有限归因补充

`network-probe-20261001T133647-d0e86248` 使用同一文件及同一可见绝对路径，逐样本核对两条lane的 `device/inode` 完全相同；每个负载重新达到相同guest冷/热条件，写入使用新的内容标记并校验。仍是一轮预热加五个交替配对，48任务；原runner/独立证据复核通过。补充没有替换前面的360任务。

| 同文件负载，并发1 | ext4（ms） | native（ms） | 成对中位比值 |
| --- | ---: | ---: | ---: |
| 顺序1MiB写8GiB、fsync、guest-cold | 4201.77 | 4202.96 | 0.990 |
| 随机4KiB读512MiB、8GiB工作集、guest-cold | 19197.21 | 19943.88 | 1.035 |
| 随机64KiB读512MiB、hot | 46.74 | 51.56 | 1.076 |
| 随机64KiB写512MiB、close、hot | 82.41 | 81.65 | 0.983 |

同文件条件下冷随机读的偏向从0.934变为1.035、热随机写从0.926变为0.983；不能把原不同文件的优势解释成稳定bind加速。热64KiB读仍有7.6%的配对中位差距，所以文件布局**不能单独解释所有差异**。现有样本无法区分剩余的虚拟化/调度/测量开销和具体路径因素；未证明是某个DMS热读bug或不可消除的bind固定成本。两条lane最终都访问同一ext4对象及native祖先，但机制相同不能代替性能通过；2/4补充组合仍大于1。此有限归因到此结束，不继续挑轮次或补跑直到符合阈值。

[全部48样本](acceptance/results/native-bind-container-io-shared-20261001.json)，SHA `94dff8c5ff7898c887b8b5aa4c2406c7057dc69f60d64628048b306d58e62d6f`；冻结输入 `native-network-native-candidate-20261001T133646`，IOv3执行 SHA `93a34610fa9fe7d08b2343a3a01889abcf40b0faa3c4eaa49b7fc7081e331942`。A/B/ctl原始SHA分别 `038db0aae3db4dd3505a6f9d63bb6933ef6c89da584632c250943a18302f66b4`、`1babe563c0a0058c421726f1d53755ba3c193536e316fa6169097b8f9e8399b3`、`2156d93eca2f0de365b7b6186956d590b22122b67f3dc3ecba42e197585e9a49`。同一verifier接受原归档/回复/缓存/文件身份，拒绝归档、读回复、计时、source、隔离、缓存前提及同文件身份七类篡改。补充也重复验证原export正常detach后容器仍读写 `device=2065,inode=1049833`，再停止/delete容器，Node/Meta退出0。

### 原 export 卸载成功不代表容器已排空

计时结束后，两个容器仍运行。Node manager 对原 export 的**正常卸载**返回 `Detached`，随后 native 容器仍读到原文件、创建并写入 marker；最终 workspace 仍是 ext4 `device=2065,inode=1049727`。再受控停止并 delete 两个容器，Node/Meta 正常退出。实际 manager 回复和容器 probe 回复均保留并独立核对。

这关闭了一个架构问题：跨 namespace 的容器 bind 是独立挂载对象，原挂载的 EBUSY/umount 结果不能代表克隆挂载状态。生产管理面必须跟踪最终 runtime/Agent mount identity，并在回收、切换、复用 backing **之前**停止/卸载这些使用者；否则容器仍能写入被当作已回收的旧 source。RootGrant 撤销也不能直接使 native 引用失效。这里没有实现完整生产排空/fencing；通用可靠性由另一位 AI 负责，但 bind 使用者身份和容器卸载的要求不可省略。

### 输入、失败与复核边界

- 完整 run 冻结输入 `native-network-native-candidate-20261001T131136`。执行 IO ELF v2 SHA `6e15d4733ec839a3ea27ff33df5c686bc3f9522c5fdbc948601bc445fd6abdcd`；Node/Meta/runc/probe 与 E18 相同。C 严格编译及36个正负自检通过；热条件不满足及错误内容都不能产生合格样本。
- 机器数据 SHA `7daf3b73eeef166a5b1de1b918059eeaf2495d4496ed13bb66250c224b6f6ff6`；原始 A/B/ctl 归档 SHA 分别 `1a04778c5ebe3e20d63547721e4ec786f82eb6eca166fd79a2cdc2fa905c50a6`、`215a9e9666def03113df6b692cbe776c0bd65546f0d0265ce5796c93d4b5c3f2`、`ff96328a6df7c9009b934e8e733efb29d1019cd1ec5f66b073d55bea56d0f02a`。verifier `7bd95e9d9b3467736a8903a4920e375fdd816d8c18d20cd549f5285ca023e3d4` 接受原证据，拒绝伪造归档/远端读取/timing/source/隔离。
- 首次 `network-probe-20261001T130434-d53bf068` 仍为 FAIL：仅全局 sync/drop_caches 后第五个任务的目标驻留864256B，未满足0驻留条件；没有容忍小比例残留。四个此前样本和第五个实际回复保留，不并入完整数据。修复仅是计时前目标条件检查/有界准备，不是 DMS 性能修复；残留页的具体忙/脏原因未证明。失败后容器/Node/Meta 正常收尾，再对核验 inode 下六个新建数据文件逐项清理，未删其他工作区。
- v3仅增加实际文件 device/inode 输出，以支持四个同文件归因案例；v2完整数据仍绑定原冻结源码/ELF，不拿当前v3源码声称原run由它构建。大工作集校验后按确定文件名清理，保留逐项记录及原始结果，不把18GiB数据文件打包到证据。
- 本轮尚无 MooseFS 顺序/随机对照、远端计时或强持久性对等资格化。C2的三方P1不能替代这些缺失；C3完整矩阵和整个性能阶段仍开放。

## 下一步限于 bind 的关键问题

交付同时保存[六个可离线核验的原始实验包](acceptance/results/README.md#可供另一台电脑离线核验的原始证据)，包括E15语义FAIL和E17原runner FAIL；另一台电脑不必依赖SSH才能读取关键反例、实际计时与冻结源码。最终observer `7e4a02d6c1934020a7be751e5c4b0b32a1fe3d5ae59df406bce7559504fdb826` 另将rootfs中实际执行ELF直接关联冻结input SHA，并重新接受三个容器实验及七类补充篡改的预期拒绝。冻结guest原始归档保持原SHA。

1. 关闭绝对路径祖先开销：检查仅合成不可变根属性的安全缓存，以及挂载根名称/epoch 的失效机制；或评估最终 Agent namespace 中保留同名路径但使用 native 祖先。两者都需要实证，后者还需核对 Agent 的根目录创建/枚举合同；当前未选择、未实现。不能直接给整个 workspace 正 TTL 或假设用户只走 cwd。
2. 对追加/偏移、POSIX owner 给出维持完整合同的仲裁/客户端路线，或取得明确合同修改。未获得豁免前持续保留架构未通过；不把它们当作另一位 AI 的通用可靠性兜底已解决。
3. 容器本地顺序/随机 ext4/native 已有 E19；补远端及 MooseFS 相同工作负载/屏障/缓存对照，先资格化适用持久性。本地P1或P2/P3不能冒充P4或三方完整性能阶段通过。
4. 通用 drain/retry/recovery 继续由另一位 AI 负责。当前分支保留此前 bind 基础及共享接口边界；交付仍为 issue → 隔离分支 → 不合入 PR，review 必须看到以上未决项。


## 2026-10-06 调查出口

用户要求先完成架构、性能对照、有限归因和 issue/MR 交接，再决定是否继续性能调优。本页既有 E17-E19 结果和严格门槛结论保持不变；核心容器本地当前接近原生，不扩大为宿主绝对路径/远端/所有屏障均达标。新增三方本地/两方远端同负载采样及原始失败记录将汇总到 [有界穿刺报告](native-bind-closeout.md)。性能差距不要求本分支修复。


2026-10-06最终五路径诊断收口：120 metadata、240 IO固定任务完整采集；native本地多数接近ext4，保留10.6% seq-close、25.1% random-fdatasync两项差距及屏障/CPU分解。远端metadata与热小读有明显差距，index查询放大／DIRECT_IO与baseline缓存策略已有有限机制归因，未继续修复。原10k remote失败、cache/B001未资格化及完整合同反例保留。最终结论、实际表、原始包与实现移交以有界穿刺报告为准，不以本注释宣告任何正式阶段PASS。
