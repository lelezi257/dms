**当前出口（2026-10-08，G2.21）：** DFS只读路径保留一个准确FileVersionId的不可变version/layout缓存；逐读GetInode/新鲜度/完整chunk校验和错误传播不变。一次64MiB/R3/C1双读者对照，B/C吞吐45.581→52.916／45.610→53.022MiB/s（+16.09%／+16.25%），独立p95 23.291→19.641／23.315→19.520ms；达到测前各>=5%且p95不退步保留线。**功能限定PASS／测量COMPLETE／产品改善保留／正式3FS性能PENDING**，不称原G2.21完成。[版本、条件、回归、关闭和证据](evidence/20261008-dfs-read-version-cache/README.md)。148 DFS/13完整性测试及受影响Linux门禁通过；六实际wait0、原挂载/12保护进程保留，原始数据归档恢复通过。G1历史8/8关闭、原27项仍12限定完成/0bind进行中/15待验收；b80有限ON试用保持原身份，不重包。G2.14 READ/G2.15 WRITE拒绝方向不重试，下一回远端读一个具体RPC/数据路径成本；普通本地、复杂可靠性和基线资格矩阵后置。

以下日期记录保留原版本与结论。

**2026-10-08 G2.21当前小项（事实）：** 原f03/7bfc档案在Linux核对700成员，复用10次正式整文件验证耗时和5个ctl窗口；只可给出事后诊断分位数，旧数据没有逐次read区间/跨VM时钟映射。仅测试C探针和driver补64个逻辑1MiB full_read区间（排除oracle），单独open/fstat/EOF/close；Linux先RED再31项针对性测试及独立ext4 helper验证通过，13成员紧凑证据核对。没有Rust/AFS服务/新性能运行，不宣称G2.21或3FS达标；历史功能/恢复、G1 8/8、G2计数/defaultOFF保留。下一仅小规模同步双读的缺失测量，3FS资格/实际跨VM syscall overlap/复杂场景后置。[版本、原数据、工具及边界](evidence/20261008-dfs-manyread-timing-reuse/README.md)。

**2026-10-08 当前新增（事实/决策）：** 修复 managed runc 单独ON遗漏RootCommand监听，仅Node资格谓词改为host OR managed，双OFF和配置互斥不变。Linux先复现RED，再11个Rust/10个Python测试及受影响构建通过；新候选9d7基底/158-map866cd522真实官方runc/4KiB容器读回、wrong忽略/matching拒绝及自然关闭限定PASS：71运行谓词、23独立核验，3wait0+1拒绝wait1、9PID/runtime/FUSE闭合，原数据和保护库存不变。420raw/158编译输入/39工具版本Linux恢复通过；两启动前工具BLOCKED和3ERRO/1WARN保留，无VM修补、产品只运行一次。私有export不在host观察namespace，不能冒称宿主ON/完整bind/POSIX/性能。DFS历史f03一写多读/R3证据限定复用，不重标新ELF；8442 OFF smoke仍原版本。G1历史8/8/G2总数/defaultOFF不变，Release按用户决定等待。下一G2.21先核对原始时延/并发观察复用，仅补小规模缺口；点优化/3FS资格/复杂可靠性后置。 [版本、范围、失败和证据](evidence/20261008-workspace-runc-root-command/README.md)。以下保留原时点记录。

**2026-10-08 当前新增（事实/决策）：** 8442 OFF标准影响核对：f03→8442七compiler路径变化，14操作/协议/Store/blob未改；RootManager共有准入变化仍纳入OFF范围。Linux158输入核验、既有59针对性tests/43安装恢复证据限定复用；现有已过标准VM新隔离目录实际Owner pjdf smoke4文件/241TAP PASS（236发现/232未选，0unexpected/skip/TODO），2实际wait0/4PID及原mount/process库存闭合，未重跑全集/Cargo或修环境。日志92ERRO及首读证工具FAIL保留；66raw/89guest/固定Git工具与更正实际Linux恢复PASS。G1历史8/8/G2总数/defaultOFF不变；历史e925/0891完整标准不重标为8442实跑，新Release按用户决定等待GitHub恢复。下一核对8442的DFS一写多读/三副本恢复复用范围，点优化/3FS资格/复杂可靠性后置。 [版本、范围、原TAP和索引](evidence/20261008-standard-impact-8442/README.md)。以下为历史时点，不是当前待办。

**2026-10-08 当前新增（事实/决策）：** 新候选e6f基底/158输入mapd053a333的host bind ON生产RootCommand接收、精确Home拒绝及正常关闭限定PASS：59 Rust+5 Python针对性测试、58运行检查、46独立保存数据核验及Linux实际恢复通过。真实Store两提交/生产RPC日志匹配，Node自然wait1、Meta及bootstrap正常wait0，原4KiB/身份/权限不变；测试发行器不入普通包。Strict all-target/Owner-only Clippy被未修改文件既有lint阻塞，失败保留；受影响Clippy带既有warnings通过。Host ON控制错误fail-closed，OFF策略未改；不称生产issuer/durable ACK/full bind或性能。G1历史8/8/G2总数/defaultOFF不变。下一新候选default-OFF包复现/安装和OwnerFs+DFS local-file核心恢复，标准/八bind性能按身份复用；复杂可靠性和点优化后置。[版本、边界、原始证据](evidence/20261008-workspace-bind-root-command/README.md)。以下保留原时点记录。

**2026-10-08 当前事实：** f03/7bfc的三同步副本正常Meta重启读回已补齐：三Node/挂载身份不变，前后48物理份及A/B/C全64MiB SHA/EOF，五actualwait0、12保护进程和预算通过。副本观察器15 Linux针对性测试、506保存数据检查及605raw/60guest实际恢复通过；R1原FAIL保留，51→54历史回执按三当前serving节点计数，无产品/vendor/环境变更。仅正常进程恢复，不称崩溃可靠性或3FS性能。G1历史8/8/G2总数/defaultOFF不变。 [当前版本、证据和下一项](evidence/20261008-dfs-r3-recovery-qualified/README.md)。 以下保留原时点。

**2026-10-08 当前新增（事实/决策）：** 固定f03/7bfc复用停止的local-file/R3夹具，仅Meta正常重启，三Node/挂载/UDS身份不变、重启前A/B/C全64MiB SHA/EOF与48物理份通过，五actualwait0/12保护进程和预算闭合。原观察器重启后把同三节点跨epoch六历史回执计成六副本而FAIL，后续物理检查/读回NOT_RUN；post-closure实体及代码支持观察器判据不匹配，不称产品六物理副本或恢复PASS。Linux6独立guards/137保存数据检查、600raw/51guest及工具/失败脚本实际恢复通过；所有首失败保留，无产品修补/重跑/环境变更。G1历史8/8、G2总数/defaultOFF不变。下一独立小项：副本观察器跨epoch唯一serving-node/设备/catalog-floor/失效authority针对性覆盖，再补受影响R3正常恢复读回；f03 R1和旧标准直接复用，7e6 R3保持历史，点优化/3FS资格/复杂可靠性后置。 [版本、FAIL及证据](evidence/20261008-dfs-r3-meta-recovery/README.md)。 以下保留原时点。

# Validation strategy

Validation follows [the three-stage acceptance checklist](trial-release-goals.md). The purpose is to make progress in independently reviewable units: first runnable, then standard and small performance cases, then broad reliability and backend matrices.

All Rust builds, filesystem tests, privileged FUSE runs and product runtime checks must run on the ARM64 Linux VM environment. macOS is allowed for editing, Git operations, documentation review and VM orchestration only.

## Evidence levels

| Level | Use | Minimum evidence |
| --- | --- | --- |
| Static/document check | Documentation, manifests and non-runtime scripts | Link check or grep-based consistency check, plus `git diff --check` |
| Unit/contract check | Narrow Rust or Python behavior change | Targeted test for the changed behavior, formatting/lint when applicable |
| Source gate | Shared Rust behavior, protocol, FUSE, Meta, Node or deployment changes | Linux format, strict Clippy/build and affected library/contract/integration tests |
| Runtime smoke | Candidate usability or affected distributed behavior | Identified binaries/configs, real mount/process identities, raw commands, content checks and cleanup |
| Standard suite | POSIX fallback and regression safety | pjdfstest, fixed LTP subset and short FSx with full accounting and predeclared exclusions |
| Performance case | G2 performance item | Baseline and candidate on the same frozen case, correctness proof, resource identity, raw timing, noise policy fixed before measurement |
| Formal release gate | G3/final release | Full suite matrix, 8 GiB cases, long soak, comparator qualification, durable backend/fault/RDMA/deployment evidence |

Short smoke success never replaces a full case. A full old run does not qualify a changed candidate. Each result must bind source, binary/package, command, environment, mount identity, raw output and pass/fail criteria.

## Stage-specific gates

### G1 retained trial

G1/g1.5 remains complete in its historical scope: OwnerFs/DFS installable trial, memory demo and central local-file Meta restart recovery. Do not use new candidate failures to erase that result. Do not upgrade it into full POSIX, formal 69-case or performance qualification.

### G2 current work

G2 now prioritizes usable **bind ON + correct remote access**. Execute G2.12 finite necessary functions and explicit ON scenario delivery, then remote cooperation/performance (G2.14–16), then DFS one-writer/many-readers (G2.21), then ordinary local FUSE (G2.09–11). Reuse unchanged standard/recovery evidence by version/scope; supplement only affected gaps. The DFS missing-read slice has ended after its single small formal run: function PASS / measurement COMPLETE / qualified3FS performance PENDING. G2.12 finite current-scenario function, orderly recovery and explicit ON trial delivery are closed at b80; continue with remote performance. A later Node candidate does not inherit that package's runtime or delivery identity.

Separate function, measurement, performance and delivery under existing IDs. Tool checks and document synchronization do not close product exits. Bind eight scoped core >=0.90 ext4 results are reused unless affected. Preserve ordinary Owner throughput >=1.2x MooseFS and independently measured latency <=0.8x; do not hide current remote/local FAILs. Optimize one demonstrated product problem per round with same-condition before/after results; packaging/install is concentrated at ON delivery. The checklist contains finite required vs expansion exits.

Performance cases start small, defaulting to 64 MiB unless the item says otherwise. 512 MiB and 8 GiB are separate records. Large data, long-running and complex mixed cases do not block smaller completed items.

### G3 deferred gates

G3 contains broad LTP/POSIX, long FSx/differential random, 8-hour soak, failure matrices, RDMA abnormal lifecycle, multi-Meta/HA, wider deployment, etcd resource topic and Redis. Keep any local evidence, but do not claim these gates until the full case exits pass.

## Comparator rules

Ordinary OwnerFs local and remote read/write use MooseFS as the comparator and require both throughput >=1.2x and operation latency <=0.8x under the same frozen case. OwnerFs workspace bind mount keeps the separate native-ext4 ON/OFF comparison. DFS targets 3FS parity under matched POSIX/FUSE and three synchronous durable copies. Delete cases require correctness and a measured comparative report; they have no new hard ratio unless a later checklist item adds one.

Baseline and candidate must use the same interface, data shape, concurrency, cache policy, durability barrier, applicable replica semantics, mount type and resource budget. Noise tolerance, timer clock, sample count, quantile method and latency judging percentile are fixed before the run; throughput defaults to paired-median comparison, p50/p95/p99 are recorded from a per-operation latency sample array, and latency is not inferred from throughput. Do not rerun an unchanged failed case until a new input or hypothesis exists.

## Capacity and logs

Capacity admission is per case. Small standard and 64 MiB performance cases should run before 8 GiB or long soak. A case that needs more disk must state the live data, baseline/candidate order, cleanup plan, log budget and host free-space requirement. Expanding a VM disk is allowed after protecting current services and state, but it is not a prerequisite for the first small cases.

## Reporting

Every report must separate:

- PASS/FAIL/BLOCKED/INCONCLUSIVE.
- Current candidate evidence from historical evidence.
- Product evidence from tooling or environment preparation.
- Memory Meta evidence from local-file, etcd and Redis persistence.
- OFF/FUSE evidence from bind/native ON evidence.

If a result is blocked by environment, record the blocker and continue with independent items that do not depend on that environment.

## Container workspace small diagnostic

The [fixed diagnostic slice](container-workspace-perf-slice.md) restores the two first-party C payloads by fixed Git identity, builds only on Linux, and times actual OCI-process work through OFF/FUSE or controlled ON exec. One warmup/five alternating paired rounds retain content, exact argv, namespace/source, live ELF and cleanup evidence. [Current data](evidence/20261007-container-perf/README.md) is diagnostic only; unobserved cache and unavailable exact FUSE counts remain limits, and failed mixed semantics block G2.13 qualification. Do not repeat this case merely to polish noisy small measurements.
