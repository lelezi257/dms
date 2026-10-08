**当前 G2.14（2026-10-08）：** d14候选 bind ON/B Home/C远端一次64MiB/C1读，功能限定PASS／Owner侧测量COMPLETE：416.250925MiB/s，独立pread p50/p95/p99为1.821228/5.166909/7.134337ms，320原始间隔。六轮Home实际取数各64MiB、内容/权限/errno及正式阶段三actualwait0通过；首驱动warmup失败另保留三wait0，未刷分。Moose因旧官方服务文件uid501不满足root所有权门禁，启动前BLOCKED，比较NOT_RUN、正式1.2/.8双目标PENDING；不是容量阻塞，无产品改动或改善结论。 [版本、原始证据和失败](../development/evidence/20261008-owner-remote-direct-read/README.md)。G1历史8/8和原G2 12/0/15不变；b80 ON交付保留原身份。下一：等待Moose隔离部署选择，独立Owner远端工作继续，DFS仍暂缓。

**当前出口（2026-10-08，G2.15）：** 远端写一次839→d14对照189.875→199.831MiB/s（+5.24%）、独立pwrite p95 7.050→6.951ms（-1.40%），达到测前保留线，Owner-only TCP公开API入站256KiB帧改动保留；p99+5.85%及轮次波动完整留数，不称稳定广泛提升。功能限定PASS／测量COMPLETE／正式Moose1.2/.8仍PENDING。 复用原64MiB同inode，B/C512MiB、ctl256MiB预算贯穿准入/运行/退出，采样峰值B264,790,016B、盘余21,269,839,872B，容量PASS；五actualwait0/原mount及保护进程不变。160输入与已过四测试/构建候选完全一致，Linux身份/fmt及537保存证据核验、1690raw+27guest实际恢复PASS；严格Clippy既有失败和19,493B/5ERRO/60WARN保留。无第三方/环境修补/新包；旧FAIL_BUDGET不改。G1历史8/8关闭、原G2仍12/0/15，DFS写按用户暂缓。下一Owner远端核心的一项剩余产品成本，复用有效基线、不重跑本项或扩资格矩阵。 [本轮版本与证据](../development/evidence/20261008-owner-remote-write-server-pair/README.md)。

**2026-10-08 当前新增（事实/决策）：** 修复 managed runc 单独ON遗漏RootCommand监听，仅Node资格谓词改为host OR managed，双OFF和配置互斥不变。Linux先复现RED，再11个Rust/10个Python测试及受影响构建通过；新候选9d7基底/158-map866cd522真实官方runc/4KiB容器读回、wrong忽略/matching拒绝及自然关闭限定PASS：71运行谓词、23独立核验，3wait0+1拒绝wait1、9PID/runtime/FUSE闭合，原数据和保护库存不变。420raw/158编译输入/39工具版本Linux恢复通过；两启动前工具BLOCKED和3ERRO/1WARN保留，无VM修补、产品只运行一次。私有export不在host观察namespace，不能冒称宿主ON/完整bind/POSIX/性能。DFS历史f03一写多读/R3证据限定复用，不重标新ELF；8442 OFF smoke仍原版本。G1历史8/8/G2总数/defaultOFF不变，Release按用户决定等待。下一G2.21先核对原始时延/并发观察复用，仅补小规模缺口；点优化/3FS资格/复杂可靠性后置。 [版本、范围、失败和证据](../development/evidence/20261008-workspace-runc-root-command/README.md)。以下保留原时点记录。

**2026-10-08 当前新增（事实/决策）：** 8442 OFF标准影响核对：f03→8442七compiler路径变化，14操作/协议/Store/blob未改；RootManager共有准入变化仍纳入OFF范围。Linux158输入核验、既有59针对性tests/43安装恢复证据限定复用；现有已过标准VM新隔离目录实际Owner pjdf smoke4文件/241TAP PASS（236发现/232未选，0unexpected/skip/TODO），2实际wait0/4PID及原mount/process库存闭合，未重跑全集/Cargo或修环境。日志92ERRO及首读证工具FAIL保留；66raw/89guest/固定Git工具与更正实际Linux恢复PASS。G1历史8/8/G2总数/defaultOFF不变；历史e925/0891完整标准不重标为8442实跑，新Release按用户决定等待GitHub恢复。下一核对8442的DFS一写多读/三副本恢复复用范围，点优化/3FS资格/复杂可靠性后置。 [版本、范围、原TAP和索引](../development/evidence/20261008-standard-impact-8442/README.md)。以下为历史时点，不是当前待办。

**2026-10-08 当前新增（事实/决策）：** 8442新ELF的普通default-OFF包两次Linux复现SHA一致，沿用无编译器micro/ext4独立安装及OwnerFs/DFS各64MiB确认数据的local-file Meta正常重启全SHA/EOF读回PASS；43谓词、Node/挂载不变、3实际wait0/6PID、自有闭合及1保护进程/50旧ELF/26mount库存通过。15相同工具guards直接复用；Linux100raw/93guest及158compiler/13打包/2工具实际恢复通过。10,308B/26ERRO+12WARN、两个读证工具首FAIL均保留，产品仅运行一次，未修环境/重编译。不是完整POSIX、R3、性能或完整G2.27出口；旧f03/标准/性能结论不变。G1历史8/8/G2总数/defaultOFF不变。下一8442 OFF标准影响映射，未受影响结果按身份复用，仅补必要路径；点优化/3FS资格/复杂可靠性后置。 [本项版本、包和证据](../development/evidence/20261008-current-trial-8442/README.md)。**发布分项BLOCKED：** 新Release create固定120s超时/只读404，已按用户答复等待GitHub恢复、独立标准核对继续；main637394e9代码/证据正常推送并Git读回，本地包齐备，独立标准影响核对继续。以下为历史时点，不是当前待办。

**2026-10-08 当前新增（事实/决策）：** 新候选e6f基底/158输入mapd053a333的host bind ON生产RootCommand接收、精确Home拒绝及正常关闭限定PASS：59 Rust+5 Python针对性测试、58运行检查、46独立保存数据核验及Linux实际恢复通过。真实Store两提交/生产RPC日志匹配，Node自然wait1、Meta及bootstrap正常wait0，原4KiB/身份/权限不变；测试发行器不入普通包。Strict all-target/Owner-only Clippy被未修改文件既有lint阻塞，失败保留；受影响Clippy带既有warnings通过。Host ON控制错误fail-closed，OFF策略未改；不称生产issuer/durable ACK/full bind或性能。G1历史8/8/G2总数/defaultOFF不变。下一新候选default-OFF包复现/安装和OwnerFs+DFS local-file核心恢复，标准/八bind性能按身份复用；复杂可靠性和点优化后置。[版本、边界、原始证据](../development/evidence/20261008-workspace-bind-root-command/README.md)。以下保留原时点记录。

**2026-10-08 当前事实：** f03/7bfc的三同步副本正常Meta重启读回已补齐：三Node/挂载身份不变，前后48物理份及A/B/C全64MiB SHA/EOF，五actualwait0、12保护进程和预算通过。副本观察器15 Linux针对性测试、506保存数据检查及605raw/60guest实际恢复通过；R1原FAIL保留，51→54历史回执按三当前serving节点计数，无产品/vendor/环境变更。仅正常进程恢复，不称崩溃可靠性或3FS性能。G1历史8/8/G2总数/defaultOFF不变。 [当前版本、证据和下一项](../development/evidence/20261008-dfs-r3-recovery-qualified/README.md)。 以下保留原时点。

**2026-10-08 当前新增（事实/决策）：** 固定f03/7bfc复用停止的local-file/R3夹具，仅Meta正常重启，三Node/挂载/UDS身份不变、重启前A/B/C全64MiB SHA/EOF与48物理份通过，五actualwait0/12保护进程和预算闭合。原观察器重启后把同三节点跨epoch六历史回执计成六副本而FAIL，后续物理检查/读回NOT_RUN；post-closure实体及代码支持观察器判据不匹配，不称产品六物理副本或恢复PASS。Linux6独立guards/137保存数据检查、600raw/51guest及工具/失败脚本实际恢复通过；所有首失败保留，无产品修补/重跑/环境变更。G1历史8/8、G2总数/defaultOFF不变。下一独立小项：副本观察器跨epoch唯一serving-node/设备/catalog-floor/失效authority针对性覆盖，再补受影响R3正常恢复读回；f03 R1和旧标准直接复用，7e6 R3保持历史，点优化/3FS资格/复杂可靠性后置。 [版本、FAIL及证据](../development/evidence/20261008-dfs-r3-meta-recovery/README.md)。 以下保留原时点。

**2026-10-08 新增限定功能（事实/决策）：** 固定f03/7bfc，真实宿主workspace bind的物理Home目录原子交换后，身份轮询使Node自然失败关闭：56.44ms观察到Node/监督者及bind/FUSE消失，原Node实际wait1/source-replaced领域码，Meta正常wait0。原inode/4KiB全SHA/0700/501:501恢复、保护进程/旧mount与容量通过；Linux6独立guards及36保存数据审计、77raw/92guest记录/9工具/5失败版本恢复通过。原驱动日志判据FAIL、传输和三审计工具失败保留，修正驱动未重跑产品；仅对已保存实际数据补充正常恢复/独立审计。无Rust/vendor/产品ELF/环境更换，非Meta RootCommand watch/ACK或full bind，G1历史8/8、G2总数/defaultOFF不变。 [证据及原失败](../development/evidence/20261008-workspace-bind-source-replace/README.md)。下一小项：固定f03 DFS三同步副本的local-file Meta正常重启恢复，优先复用已停止小夹具/数据；f03 R1恢复已过，7e6 R3仍历史，不自动继承。普通点优化、3FS资格、大规模/长时及复杂可靠性后置。 以下保留原时点。

**2026-10-08 当前新增（事实/决策）：** 固定f03/7bfc/local-file/OFF新单文件64MiB一写同步调度B/C双读者限定功能PASS：各1预热5读完整SHA/EOF，5轮双方READY→START→DONE，16不同chunk×3Ready/Durable/48物理份在读前核验。Linux288独立检查、16fixture guards（14普通+2root）/8relay guards、700raw/62原始guest记录及4维护工具/4失败脚本实际恢复通过；四实际wait0/8PID、12保护进程含此前简查漏计Redis及原mount闭合。峰值591,343,616B<896MiB，日志5,310B/5ERRO+10WARN保留；准备/审计首FAIL只修工具，产品单次运行，无Rust/vendor/VM改变。B/C中位45.312043/45.324874、ctl共同窗口90.510222MiB/s仅诊断，无实际跨VM syscall overlap、操作分位数或3FS资格，不称性能达标。G1历史8/8/G2大项计数/defaultOFF不变。 [证据、版本和边界](../development/evidence/20261008-dfs-r3-sync/README.md)。下一：核对 DFS 核心恢复历史证据与固定 f03 的输入及判据，匹配则直接复用并跳过；3FS资格、普通点优化、大规模/长时后置，已过标准、删除及八bind核心性能不重复。 以下保留原时点。

**2026-10-08 当前证据复用（事实/决策）：** Linux直接审计f03/e622原2039-member档案，确认两份A写64MiB文件经fdatasync及目录屏障后，分别由C、B完整SHA/EOF读回；每份16不同chunk×3节点Ready/Durable/48物理份，原live ELF/四wait0及保护库存保留。两读者是顺序路由，不能冒称同文件同步并发。R1旧ELF标签FAIL及R2停止后观察时点保留，chunk数组相同而完整快照有修订差异；首版mount清单审计误判亦保留，仅修审计器、无产品重跑/VM改动。Linux保存数据审计与7-member新紧凑档案/原失败脚本实际恢复通过，不称新增产品测试或正式3FS/时延达标。G1历史8/8、G2计数/defaultOFF不变；G2.21仅补f03限定功能复用，931并发计时/7e6恢复保持原身份。 [证据与边界](../development/evidence/20261008-dfs-one-write-reuse/README.md)。下一小项是当前f03同文件同步B/C双读者的必要准入和测前冻结；3FS资格、普通点优化、大规模/长时后置，已过标准/恢复/八bind性能不重复。 以下保留原时点。

**2026-10-08 当前新增（事实）：** G2.14固定f03/7bfc，A远端读/B-Home/ctl local-file、OFF，测前冻结64MiB/C1底层热/客户端默认策略；1预热5交替对，各320真实区间。Owner/Moose中位384.806472/14457.621211MiB/s、p95 3152191/111257ns；吞吐0.026616×、p95 28.332518×，双项FAIL。24物理快照全热，客户端0/64MiB差异明示；内容、6实际wait0/12PID、11保护进程/旧mount保留及预算通过。Linux22fixture guards、61实时/98保存数据检查与146raw/313恢复核验通过。3034B/4ERRO+2WARN及工具首失败、范围文字勘误均保留，旧DIRECT失败不重跑、不修环境。G1历史8/8关闭、G2总数/defaultOFF不变；本小项留数收口，不继续单点打磨。[本项证据](../development/evidence/20261008-owner-remote-read-backing-hot/README.md)。下一核对DFS一写多读现有f03证据的复用范围；3FS比较资格、普通性能调优和大规模后置，已过标准/8bind性能/恢复不重复。 以下保留原时点。

**2026-10-08 当前新增（事实）：** 固定f03/7bfc，B/ext4/local-file/OFF，单列测前冻结的64MiB/C1“底层热、客户端默认缓冲”读比较；双方物理payload24个前后快照全热，客户端0/64MiB作为实现差异单独报告，不改旧客户端hot FAIL或repeat NOT_QUALIFIED。1预热5交替对/各320实际间隔：Owner/Moose中位6009.863/17409.447MiB/s，p95 198417/65125ns；吞吐0.345207×、p95 3.046710×，**双项FAIL，未达1.2/.8**。内容/实际五服务wait0/8PID与正常mount闭合通过，Linux5工具guards、98独立审计及152 raw/4执行工具恢复核验通过。日志8303B/3ERRO+9WARN及准备/首版guard失败保留。无Rust/vendor/VM变更，G1历史8/8/G2完成计数/defaultOFF不变，不重测刷分，局部优化另留专项。[本项证据](../development/evidence/20261008-owner-local-read-backing-hot/README.md)。下一普通Owner远端核心小读的默认缓冲/底层缓存准入，先独立测前冻结；旧DIRECT观察ENODEV继续停止，不修环境或改旧判据。已过标准/八bind性能/恢复不重复。 以下保留原时点。

**2026-10-08 新增限定通过（事实）：** 固定 f03/7bfc 新增真实授权失败下 FD/mmap 引用排空 PASS（56 checks）：关闭监听后 FD+mmap 保留原 mount，关闭 FD 后 mmap 仍保持；释放映射后12.09s内 Node/监督者及 bind/FUSE 正常关闭，原 Node 实际授权错误 wait1、触发 Node/Meta wait0。Linux11 guards及131 raw/8当前工具/2旧工具/18前轮工具版本实际恢复核验通过。 无 Rust/vendor/VM 改动，G1历史8/8关闭、G2总数/defaultOFF不变，完整 bind/正式性能未完成。[本项证据及边界](../development/evidence/20261008-workspace-bind-reference-runtime/README.md)。append/偏移已有真实 FAIL 保留；官方 WRITE/SEEK_CUR API 无支持的最小修补，作为协议专题后置，不改内核/vendor或绕过双开关安全拒绝。下一普通 Owner 核心小读的物理缓存可比性准入，预先固定条件，不能准入则停该项；已通过标准/八bind核心性能/恢复不重复。

**2026-10-08 新增限定通过（事实）：** 固定 f03/7bfc 复用此前授权错误关闭后的原配置、local-file Meta WAL/Home 与原 4KiB 文件，一次进程重启恢复 PASS（44 checks）：新会话 epoch5，原真实目录 bind 到 FUSE 的 workspace 一级目录，内容/EOF/inode/权限不变，Meta/Node 正常 wait0、无残留。首轮测试工具参数错误在启动前 BLOCKED，原记录保留；修正后 Linux8项检查及97 raw/18工具版本恢复通过。无 Rust/vendor/VM 修改；G1历史8/8关闭、G2总数/defaultOFF不变，完整 bind 仍进行中，不计 crash/即时撤权/完整POSIX/性能。[本项版本、结果与边界](../development/evidence/20261008-workspace-bind-recovery/README.md)。下一独立项：核对已有混合路径 append/文件偏移失败，确定最小必要修复及针对性回归；实时 root-command watch/即时 FD 撤权、Moose 比较资格继续单列，已通过标准、八项 bind 性能及恢复不重复测试。

**2026-10-08 新增限定边界（事实）：** 固定f03/7bfc在真实Meta同身份新会话注册后，原Node约9.94s自行报授权错误/actualwait1，host workspace bind与FUSE正常移除，4KiB确认数据保留；独立保存证据22 checks PASS。两份原驱动FAIL原样保留（第二Owner缺catalog被安全拒绝、文案断言及stop-all收尾问题）；Meta已单独normalwait0、无残留。维护工具8 Linux guards及248 raw/16工具版本恢复PASS；最终修正驱动未重跑，不称完整功能PASS。未改Rust/vendor/固定产品；G1历史8/8、G2总数/defaultOFF不变。[版本、原FAIL、真实关闭及范围](../development/evidence/20261008-workspace-bind-epoch/README.md)。下一workspace bind确认数据的重启恢复小项；生产RootManager实时watch/失效、即时FD撤权仍开放，远端Moose DIRECT观察阻塞数据保留。以下记录保留原时点身份。

**2026-10-08 当前远端小读（事实）：** f03/7bfc产品未改；测前冻结64MiB/C1/1MiB、1预热5交替对及独立p95。Owner5份测量/320原始区间内容PASS，中位396.945760MiB/s、p50/p95/p99=2424831/3125040/3325581ns；官方Moose4.59.2 DIRECT缓存驻留mmap返回ENODEV，零参考计时/无有效配对，G2.14仍待验。原输出目录遗漏零计时FAIL和六次ENODEV均保留；已增加raw首失败即停守卫，Linux15驱动/20fixture检查通过，不再重测或修环境。两轮六服务分别actualwait0，11保护进程/全部旧挂载未变；Linux110guest文件及74外层归档成员恢复SHA通过。历史6d时延复用及G1历史8/8/G2计数不变；缓存观察兼容性停止并单列，下一继续独立workspace bind功能缺口，不重复已过标准/八项bind性能/FD-mmap组件排空。 [版本与全部失败](../development/evidence/20261008-owner-remote-read-current/README.md)。以下保留原时点记录。

**2026-10-08 历史远端时延核对（事实/纠正）：** 6d普通远端读和写均已记录每轮实际操作p50/p95/p99；Linux只读审计核对824个历史证据文件SHA及20份测量payload/原始stdout/工具来源，941项一致性检查通过，均非新产品测试。缺口是原始间隔数组、测前固定判定分位数及缓存/持久/退出资格，不能笼统说没有独立时延，也不由五个p95重建pooledp95。原读mfsmount wait1/写durable-ACK限制及原结论保留；不把6d计时迁移为f03性能。当前f03/7bfc产品未改，G1历史8/8/G2计数不变。下一单项为当前远端小读缓存策略/原始时延准入；现有官方Moose4.59.2的DIRECT选项已只读核对，尚未启动新测量，不死磕旧资格。 [独立审计与复用边界](../development/evidence/20261008-owner-remote-latency-reuse/README.md)。以下保留原时点记录。

**2026-10-08 发布恢复（事实）：** GitHub新故障记录确认16:52–17:01 UTC受影响且17:27已缓解，按用户授权一次正常push成功。[mainc87707d6](https://github.com/lelezi257/dms/commit/c87707d6dc28377094a2224a07e51e4344d8a5bb)已纳入本地读64888834、本地写9a3a272c、删除历史复用18554a47及bind FD/mmap排空组件回归c87707d6；独立ls-remote/fetch与完整递归文件树一致。旧500/本轮观察器失败保留，不更改验收结论。下一普通远端小读独立时延准备；G1历史8/8/G2计数/full ON/性能资格不升级。[发布核对](../development/evidence/20261008-ownerfs-bind-reference-drain/publication-recovery.json)。此发布状态覆盖下方原时点“待推送”记录。

**2026-10-08 新增限定回归（事实）：** G2.12真实OwnerFs FUSE/物理Home组件的普通文件FD与mmap引用排空通过：本地授权缓存失效后校验拒绝，FD持有时普通卸载EBUSY且身份不变；关闭FD后存活的只读共享映射仍EBUSY，unmap后正常detach及outer FUSE unmount/join。Linux fmt/release测试编译/strict Clippy、精确1PASS（600未选）及17项退出/身份审计通过；三保护FDB进程/26 mount库存未变。仅测试fixture改变，f03/7bfc普通候选未重建/部署；两个观察器ENOSYS失败、前置输入/格式失败及原数据保留。不是即时FD/mmap撤权、真实Meta撤权、通用drain/full ON/POSIX/性能；默认OFF、G1历史8/8/G2计数不变。发布按用户决定等待GitHub恢复，最后远端8d989bf5，不反复push。下一既定普通远端小读的独立操作时延边界/准入准备；已过标准/宿主生命周期/八bind性能不重开。[证据](../development/evidence/20261008-ownerfs-bind-reference-drain/README.md)。以下保留原时点记录。

**2026-10-08 顺序核对（事实/决策）：** G2.11保留e925历史限定完成；源码影响图及f03 OFF安装中的两次基本unlink支持普通功能范围复用，不称当前100文件/5对重测或沿用旧性能数字。取消尚未启动的重复删除轮次；下一回既有G2.12授权变化/native FD与mmap排空实现边界，不重新做已过宿主生命周期或8核心bind性能。[复用边界](../development/evidence/20261008-owner-local-delete-reuse/README.md)。普通本地读写新数据仍仅摸底；GitHub500发布状态另见回执。以下保留原时点记录。

**2026-10-08 本地写独立小项（事实）：** 当前f03/7bfc产品未改，B/ext4/local-file/gRPC/OFF；64MiB/C1新建+fdatasync，1预热5交替配对，12文件全SHA/EOF/权限与六个Moose本地VALID副本通过。独立Linux复算各320时延样本：Owner/Moose中位909.994/1170.143MiB/s、合并p95 1184309/1374228ns；仅摸底，Moose强durable-ACK及底层缓存资格未过，正式G2.10仍待验，不改1.2/.8双判据。两AFS/三Moose actualwait0、8PID/挂载/UDS闭合，原库存和157编译输入不变。日志14251B/19ERRO+18WARN及首次导出guard拒绝保留，无产品重跑/环境修补。G1历史8/8/G2计数不变；上轮64888834被GitHub500拒绝、最后远端8d989bf5，发布状态另以推送回执为准。下一基础本地小删除，不死磕此组性能。[证据](../development/evidence/20261008-owner-local-write-latency/README.md)。以下保留原时点记录。

**2026-10-08 本地读独立小项（事实）：** f03/7bfc不变，C探针按需原始时延样本及5实测Linux守卫通过。B单机正式hot比较在首个Owner计时前因0驻留页拒绝，0有效配对；另行测前冻结repeat诊断完成，Owner/Moose中位5937.148/18687.490MiB/s，合并p95 206000/54542ns，各320原始样本。两侧mincore0/64MiB不同，仅摸底，正式G2.09仍待验，不改1.2/.8双判据。两轮5actualwait0及8进程/挂载/UDS闭合，原库存不变；所有失败、日志和源版本索引保留。G1历史8/8/G2计数不变，下一基础本地小写，缓存可比条件列专项而非死磕。[原始证据](../development/evidence/20261008-owner-local-read-latency/README.md)。以下保留原时点记录。

**2026-10-08 当前试用分项（事实）：** f03/map2b17新普通包7bfc在Linux两次复现、无编译器现有VM独立安装通过：OwnerFs/DFS各64MiB正常local-file Meta重启全SHA/EOF读回，43基础检查、三actualwait0/六PID及保护库存闭合。Linux完整包比较证明与f03多节点/删除原包仅指南/清单/SHA变更，按原范围复用，不重跑。INFO日志26ERRO/13WARN完整保留。G1历史8/8关闭，G2计数及完整G2.27性能出口不变；固定[f03 prerelease](https://github.com/lelezi257/dms/releases/tag/afs-trial-f03dc2b)已发布且四附件远端SHA一致，旧6d/7e6资产不变；普通Owner双指标/完整bind功能仍待验。[版本与紧凑证据](../development/evidence/20261008-current-trial-f03/README.md)。以下保留原时点记录。

**2026-10-08 发布恢复（事实）：** GitHub Git Operations恢复后一次正常push成功，[main7eedd7ec](https://github.com/lelezi257/dms/commit/7eedd7eca3c1fb810663b3a491b35b18b9650f03)已含f03 create修复及新候选三节点读写/小删除证据。独立ls-remote/fetch、完整19282 blob/21070-entry tree及commit→tree核对一致；旧两次500和错误tree-ID验证假设原记录保留，不改历史验收。bind核心/测试探针归属确认，旧子目录/default产品探针不在。G1历史8/8/G2计数不变；下一[f03普通试用候选的可重复打包与独立安装/正常Meta恢复](../development/current-trial-f03-slice.md)，仍非完整G2.27性能退出。 [发布证据](../development/evidence/20261008-github-publication-recovery/README.md)。以下保留原时点记录。

**2026-10-07 当前新增：** f03dc2b3/map2b17同候选DFS小删除功能/计时完成：local-file/configuredR3/gRPC/OFF，100×4KiB×6轮（1预热5测），600内容核验/删除成功，B/C各600 ENOENT、三远端driver退出、四actualwait0/八服务PID闭合、11保护进程/完整mount库存不变。Linux存储证据独立核验PASS，测量中位27.003911/pooled27.927077ops/s仅摸底，全部递减样本保留；旧6d R2不可直接比率，正式3FS/时延/物理回收未验。INFO日志720097B/2418ERRO+600WARN完整留源码树外，分类计数入紧凑索引；工具R1权限拒绝在启动前，R2修staging流程，无环境修补/产品重跑。最大阶段采样408625152B<2GiB，当前guest根保留待完整归档/实际恢复再清理。G1历史8/8/G2计数不变。GitHub500发布按用户决定等待恢复，最后确认origin9a8；当前成果仅本地main。下一G2.27必要当前组合/试用交付，不再调优此摸底或修3FS资格。 [当前版本、失败、结果与索引](../development/evidence/20261007-dfs-delete-current/README.md)。以下保留原时点记录。

**2026-10-07 当前新增：** 产品f03dc2b3/map2b17的DFS小规模多节点local-file/R3/gRPC运行和数据完成：三写者各64MiB、1预热1测量及两跨节点读路由，6文件/18 C样本/288物理份，四actualwait0/八服务PID及24远端worker闭合，11保护进程/完整mount库存不变。观察器旧ELF标签FAIL保留，仅R2只读复核，未重跑产品；INFO日志20,155B/23ERRO/60WARN完整保留，不称零错误。A已停止本case完整归档/Linux239条恢复后释放500,867,072B，free回2,327,810,048B。G1历史8/8及G2计数不变，正式3FS/时延/完整POSIX待验。GitHub两次正常推送500，local main修复在f03，origin仍9a8；用户决定等待恢复再推送，独立Linux继续。下一G2.25小删除和G2.27必要组合。 [版本、数据、失败及恢复索引](../development/evidence/20261007-dfs-r3-multinode-current/README.md)。以下保留原时点记录。

**2026-10-07 新增源码小项：** DFS create只对父目录revision/mtime/ctime漂移作64次有界重试，同名/权限/属性变化和真实冲突仍报错、原OperationId/inode/lease/digest不变。Linux6针对性+14 namespace回归、fmt/check及未放宽的strict all-features Clippy PASS；原并发创建FAIL/default-feature lint失败和工具准备记录完整保留。未构建/部署新候选，旧7e6运行FAIL及历史通过不改，G1历史8/8关闭、G2计数不变。下一独立构建新release，再新候选local-file/R3/gRPC小规模多节点回归；环境按B/C42GiB、A/ctl保持运行。 [命令、范围和证据](../development/evidence/20261007-dfs-create-contention/README.md)。以下保留原时点记录。

**2026-10-07 独立环境维护：** 用户决定 A/ctl 保持运行；B/C 数据盘各32→42GiB完成正常停机、字节相同离线备份/Linux只读恢复及扩容后全量身份检查（24,544/10,559条，无缺失/变更）。UUID/分区起点/挂载/内容权限不变，主机实空127.96GiB、本批全容量+备份+临时预留后91.04GiB；无运行测试被打断，不重跑历史通过项。大备份在源码树外，G1历史8/8和G2计数不变。[环境、分级预算与日志规则](../development/vm-capacity.md)、[证据](../development/evidence/20261007-vm-capacity-maintenance/README.md)。下一立即返回G2.24：create修复6+14 Linux回归及strict all-features clippy已有工作区证据，待独立提交/新候选运行，不继承旧7e6 PASS。以下保留原时点记录。

**2026-10-07 新增工具小项：** DFS失败探针sample现保留完整rc/stdout/stderr/error；旧Linux回归KeyError FAIL保留，修后10 worker+6实际进程relay guards PASS。中继保留原失败、排空尾部、逐进程收尾；Lima代理退出不替代远端PID核验。原7e6多节点预热FAIL不变，未跑产品/性能；create竞争Rust修复独立待验收，G1历史8/8及G2计数不变。 [证据](../development/evidence/20261007-dfs-cohort-failure-records/README.md)。以下保留原时点记录。

**2026-10-07 当前事实：** 用户授权后，A已停止旧DFS目录完整归档并经Linux229条恢复核验，释放500,518,912B（477.332MiB）；四角色沿原判据重新准入。G2.24首个预热写FAIL：A/C create元数据条件冲突，B探针后检查ENOTCONN；中继BrokenPipe掩盖部分失败记录，缺口已明示。零有效测量/零读轮次，未重试；四actualwait0/八PID消失、11保护进程和完整mount库存不变。G1历史8/8关闭、G2计数不变。下一独立小项为确定触发create竞争的回归/有界恢复，以及中继失败留证/排空；不扩大矩阵。 [新增证据](../development/evidence/20261007-dfs-r3-multinode-runtime/README.md)。以下保留原时点记录。

**2026-10-07 current G2.24 preparation:** 9 exact Linux driver guards PASS; product runtime BLOCKED before start by A's existing capacity prerequisite (short148.492MiB). Zero data rounds; no performance/3FS verdict. No VM repair/budget relaxation; all4 mount inventories/11 protected identities unchanged. [Evidence](../development/evidence/20261007-dfs-r3-multinode-preparation/README.md). G1历史8/8关闭，G2计数和普通Owner1.2/.8双判据不变；等待容量处理，独立文档收口继续。以下为原时点记录。

**新增小项（2026-10-07，事实）：G2.23当前7e6三同步副本64MiB小写数据完成，正式3FS对照仍待验收。** A/C1/六个不同generation新文件，1预热5计时，中位81.930440MiB/s；96不同4MiB chunks，每轮48物理份及B/C新开全SHA/EOF，四actualwait0/八PID消失及11保护进程/完整mount库存不变。单次产品运行，Linux8 C+7 driver+6 observer guards通过；初始错误文案断言/准备status假设失败留证。测前新case2GiB总预算，最终1,615,421,440B；日志23,261B的39ERRO/60WARN完整保留，不称零错误/完整POSIX。无Rust/vendor/ELF变化，不继承历史性能或升级3FS。G1历史8/8、G2新判据11限定完成/1bind功能进行中/15待验收不变；普通Owner仍1.2×MooseFS吞吐/.8×独立时延待验。下一G2.24小规模多节点读写。[证据](../development/evidence/20261007-dfs-r3-write/README.md)。以下保留原时点记录。

**2026-10-07新增事实：G2.22当前7e6/c3bb的A单读者64MiB小项完成内容/计时/正常闭合，正式3FS对照仍待验。** 1预热5读，中位65.624896MiB/s；前后48物理副本，四actualwait0/八PID消失及11保护进程/完整mount库存不变。Linux11工具guards及独立观察校验通过；首轮遗漏结果目录导致写前拒绝，原FAIL及四正常退出保留，修测试准备后一次数据运行，无Rust/vendor/VM修补。G1历史8/8、G2大项计数/defaultOFF不变，G2.23写性能/G2.27/full bind仍开放。当前OFF标准限定复用已由既有impact-map及当前安装恢复覆盖，不重跑整套；下一G2.23三同步副本小写入摸底。 [证据](../development/evidence/20261007-dfs-r3-local-read/README.md)。

以下保留原时点记录。

**当前目标调整（2026-10-07，决策）：普通OwnerFs性能改用[新准则](../development/ownerfs-performance-criteria.md)。** 本地和远端核心读写吞吐需>=同条件MooseFS的1.2倍，操作时延需<=同条件MooseFS的0.8倍，两项独立测量且同时满足；p50/p95/p99均记录，吞吐默认配对中位数判定，时延默认测前声明的逐操作p95判定。G1历史8/8不变；G2当前口径为11限定完成/1bind功能进行中/15待验收。旧e925本地ext4 FAIL和6d远端摸底数据保留原版本/原判据，不按新目标改写；G2.09/10/14/15在新目标下均待验收。

**当前独立出口（2026-10-07，事实）：G2.13完成，限定当前7e6/C1的8个小规模核心case。** 同公开c3bb包/map151a，新增64MiB写0.969905/读0.965910×ext4，复用同候选六元数据0.964347–1.032311；全部>=测前0.90。189驱动/258 Linux独立checks及15受影响工具guards通过，四actualwait0/十二服务监督及OCI PID与正常mount闭合，一保护进程/26 mount完整库存不变，峰值245153792B<256MiB。OFF写0.728219/读0.457996 FAIL保留暂缓，不重测刷分；无Rust/vendor/C修改或重建。G1历史8/8不变；G2变为11限定完成/2普通性能FAIL/1bind功能进行中/13待验收。G2.12 full ON/复杂语义、全部PR43组合/冷热耐久/大规模/正式MooseFS及3FS资格不升级，默认OFF、Goal ACTIVE。下一当前7e6 OFF标准适用性/限定复用审计，仅真实受影响缺口才补测，再更新核心性能交付状态。 [版本、原始数据及8项组合账本](../development/evidence/20261007-workspace-bind-data-current/README.md)。

以下保留原时点记录；当前入口以上方为准。

**新增当前DFS恢复小项（2026-10-07，事实）：** 产品7e6/map151a/公开c3bb普通包未变，64MiB非重复一写两读、重启前后各48物理chunk及三份Ready/Durable副本通过；Meta仅一次正常重启，三Node/FUSE/UDS身份不变。五actualwait0/十所属PID消失、三mount闭合及11保护进程/完整mount库存不变；峰值597,450,752B<1GiB。首观察器错误要求旧生命周期目录保留，原FAIL和重启前保存的退出回执不改，R2只读复核及12 guards通过；无产品重跑/环境修补。7+14 Linux工具guards通过，无Rust/vendor改动；不继承931计时或称3FS/fullG2 PASS，G1历史8/8与大项计数/defaultOFF不变。下一当前7e6 workspace64MiB读写配对，普通FAIL及复杂/后端专题后置。 [版本、原始证据与范围](../development/evidence/20261007-dfs-r3-current-recovery/README.md)。

**当前元数据性能小项（2026-10-07，事实）：** 产品7e6e00a6/map151a/当前c3bb包，唯一工具修正为将容量遍历移到callback结束快照之后。旧工具Linux先1FAIL复现，修后7guards PASS；原931 FAIL保留。当前1000×4KiB/C1/六阶段OFF+ON各1预热5配对完成，ON中位0.964347–1.032311×ext4、六项>=.90 PASS，OFF六项FAIL留数；ON八个选定回调0、其它getattr24不冒称全0。190驱动/193独立检查PASS，四actualwait0/十二服务监督及OCI PID与mount闭合、保护库存/预算不变。无Rust/vendor/C修改或重建。不是完整ON/全G2.13/正式比较；G1历史8/8、大项计数/defaultOFF不变。下一当前7e6 DFS R3小一写两读内容/副本/正常生命周期回归，历史931五轮计时保留原身份不刷分。[版本、原始数据、失败及退出回执](../development/evidence/20261007-workspace-bind-metadata-window/README.md)。

以下保留原时点记录；旧下一动作由上方当前入口覆盖。

**当前普通试用交付分项（2026-10-07，事实）：** 产品7e6e00a6/157-map151a2c6d，既有Linux release ELF两次打包字节一致；新普通包c3bb5a30不含测试探针。隔离Linux/ext4无编译器一次安装，OwnerFs+DFS各64MiB/R1/gRPC/local-file、两workspace开关OFF，43驱动检查及独立恢复/退出/保护库存检查PASS；Meta-only正常重启后完整SHA/EOF读回，三actualwait0/六服务监督PID消失、Node及两mount跨重启身份不变。15工具检查按受影响范围通过/复用，无Rust/vendor改动或重建。G1历史8/8不重开、G2大项计数不变；931 R3/旧标准与性能保留原身份，当前7e6 R3/全POSIX/完整G2.27性能及ON仍待验。固定[试用包](https://github.com/lelezi257/dms/releases/tag/afs-trial-7e6e00a)已发布，四附件远端SHA及main实际树已核对。下一回既定workspace元数据计数归因小项，普通性能FAIL与复杂可靠性后置。[当前版本、命令、结果及历史复用边界](../development/evidence/20261007-current-trial-7e6/README.md)。

以下保留原时点记录；旧下一动作由上方当前入口覆盖。

**新增测试边界整改（2026-10-07，事实）：** 产品main7e6e00a6/157-map151a2c6d：探针源码100%迁移到tests/support/workspace_probe.rs，显式Cargo example；默认cargo build与--bins仅两个产品binary，普通包无探针。实验适配器要求测试/管理员提供idle_command和identity_command，保留身份/授权/错误/正常排空检查；core/宿主bind无探针依赖、默认OFF、第三方未改。Linux11源码门禁/29选定Rust测试（含8特权）及4工具测试通过；新默认包实际单次启动/identity/读取/active Node正常关闭及独立postcheck通过，两个wait0/4服务与监督PID+容器/mount闭合、保护对象和模板不变。不是全ON/POSIX/性能资格；旧931 E2E候选、历史通过/失败和G1历史8/8均保留原身份，大项计数不变。下一返回已准备的DFS R3小规模一写两读，931结果保持自身版本；7e6的新DFS回归单列，不自动继承。[版本、命令及证据](../development/evidence/20261007-workspace-probe-boundary/README.md)。

**新增DFS R3准备（2026-10-07，事实）：** 冻结产品931/map9661/packagec7未变；Linux新64MiB非重复探针5项、当前driver4项、fixture12项检查通过，四role真实配置/TLS/ELF/容量准入PASS、合计399,278,080B。没有启动服务/数据/计时，不计G2.21通过；原工具与准入FAIL保留。按用户新任务，在此小阶段收尾后先整改workspace probe产品/测试边界，再回DFS一写多读。G1历史8/8和大项计数不变。[身份、原始输出与范围](../development/evidence/20261007-dfs-r3-preparation/README.md)。

**当前新增元数据观察（2026-10-07，事实）：** maine903基底/产品93169c8与157编译输入/ELF包未变。OFF1000×4KiB/C1六阶段完成1预热5配对、全部性能FAIL留数；ON首个预热内容正确，但全节点计数窗口readdir4未满足预定0，停止且ON比较未完成。窗口含遍历FUSE树的容量检查，不能把回调归因于业务或改判据称PASS。135驱动检查134PASS/1FAIL；117独立证据/正常闭合检查与13 Linux工具测试PASS，不等同用例PASS。实际Meta/Node四wait0、容器/PID/mount闭合、保护对象与原预算均核实；无Rust/vendor/C改动、重建、环境修补或刷分。G1历史8/8及大项计数/defaultOFF、先前数据子项PASS不变；下一DFS一写多读，计数归因/元数据ON补测单列后置。 [版本、FAIL、全部OFF数据与闭合](../development/evidence/20261007-workspace-bind-metadata-perf/README.md)。main仍唯一入口，修复分支全部有效成果已纳入，旧8dirty工作树/草稿保留且HASH复核一致；无PR/审批关卡。

以下按原时点保留历史身份。

**新增当前workspace数据性能子项完成（2026-10-07，事实）：** 产品93169c8/157编译输入与既有ELF包未变，普通官方runc容器绑定宿主workspace，OFF/独立宿主ON/ext4同候选64MiB/C1/1MiB块、1预热5配对；ON写0.944934/读1.036200×ext4配对速度达到预设0.90。OFF写0.738207/读0.348338 FAIL保留暂缓，不刷分。183驱动/232独立检查、15 Linux工具测试PASS；48原始C输出/48scrape、每轮OFF阳性/ON数据回调0、正常wait0/PID与mount闭合/保护身份/248,127,488B峰值预算均核实。缓存未观察、写后读小样本范围和单轮波动明确；不是全G2.13/full ON/当前POSIX或新试用包。G1历史8/8和总体大项计数/defaultOFF不变。下一小规模workspace元数据，然后DFS一写多读；普通性能、广义排空/混合语义/复杂可靠性仍分项后置。 [精确版本、原始数据和独立复核](../development/evidence/20261007-workspace-bind-data-perf/README.md)。

以下保留原时点/原版本记录。

**新增当前宿主运行小项通过（2026-10-07，事实）：** main产品93169c8/157编译输入未变，两次包字节一致；隔离Linux/ext4中无runc的真实Node宿主bind通过55驱动/50独立检查，12工具测试PASS。物理Home目录覆盖FUSE一级workspace，普通UID501双向64KiB内容、UID502 EACCES13、六类native数据/目录回调增量0、Meta/Node实际wait0及正常mount闭合通过；native背景getattr12保留，不冒称全部回调0。探针二次stat及外层旧回执路径错误FAIL保留，仅改工具后限定回归/复核；源码/ELF/环境不变。G1历史8/8、G2计数/defaultOFF不变；当前限定功能不继承标准/性能/full ON。下一继续既定workspace性能，旧6d摸底原身份复用，广义撤权/排空/mixed语义和复杂可靠性后置。 [版本、原始结果与失败](../development/evidence/20261007-ownerfs-workspace-host-runtime/README.md)。

以下保留原时点/原版本记录。

**当前宿主入口源码切片通过（2026-10-07，事实）：** main0e1d059基底/157-map9661a313；新增与runc无关的默认OFF独立宿主开关，只覆盖一个已存在的本地Home workspace。核心仍在单文件ownerfs/bind_mount.rs，Node持有worker并在FUSE关闭前正常卸载/join；旧容器配置兼容且两模式互斥，第三方未改。Linux12受影响门禁及50选定测试PASS（含5实际root bind），严格Clippy/release构建通过；新Node ELF9478f3e8已标识，未打包/部署。原格式、编译、ENOSYS、准入与输入冻结FAIL/BLOCKED全部保留；当前运行/标准/性能不继承。G1历史8/8、G2计数不变。下一是一次真实Node无runc的宿主可见性/内容/正常关闭验收，自动接管、多workspace、通用撤权/排空/重启和mixed append/经典锁/watch仍待验。 [版本、原始结果与失败](../development/evidence/20261007-ownerfs-workspace-host-entry/README.md)。

**当前源码小项（2026-10-07，事实）：** mainea73174基底/157-map58a71572，新增第一方FUSE callback指标，34实现回调入口计数、Node共享Registry接线；不改第三方、缓存/TTL/I/O/权限/生命周期。Linux5受影响门禁含新release构建、14FUSE dispatch测试及19Python工具测试PASS，新工具自互斥首FAIL保留。尚无新ELF运行计数/性能结论，下一仅实际短请求见证；原6d性能和d82关闭证据保留自身身份。G1历史8/8、G2计数/defaultOFF不变。 [源码版本与证据](../development/evidence/20261007-ownerfs-workspace-callback-source/README.md)。

**当前运行子项通过（2026-10-07，事实）：** main产品d82cc7d/157编译输入未变，预构建ELF两次打包字节一致；现有官方runc VM里workspace仍FinalVerified时直接停止Node，无public workspace Stop。43驱动/19独立检查通过，Node/Meta实际wait0及四服务/监督PID、容器消失，FUSE/control正常闭合，旧安装/保护身份不变。21项Linux工具测试通过；没有Rust/第三方改动。G1历史8/8、G2计数/defaultOFF保持，不等同完整ON/性能达标。下一回workspace核心性能，复用已有摸底，保留宿主独立开关/通用撤权及混合语义未完成项。 [版本及证据](../development/evidence/20261007-ownerfs-bind-active-node-stop/README.md)。

**以下记录保留原时点/原版本。**

**新增当前限定通过（2026-10-07，事实）：** main59f8753基底/157-map1fd613e5，Node保留workspace worker并在FUSE关闭前等待；真实EBUSY正常重试，终止性错误保留claim，监听失败先通知关闭。Linux9门禁（含release构建）及26选定测试通过，旧EBUSY FAIL和首测试编译FAIL保留。尚未打包或执行新Node/runc整机关闭；下一仅验该运行小项。G1历史8/8、G2计数/defaultOFF不变，历史标准/性能身份不升级。[证据](../development/evidence/20261007-ownerfs-bind-shutdown-drain/README.md)。

**新增当前限定通过（2026-10-07，事实）：** main253057f基底/157-map a062bfe8，两个真实Linux挂载清理重试缺陷先2FAIL复现、修复后2PASS；普通10测试、配置7测试及七项受影响源码门禁通过。仅确认clone卸载/容器删除阶段推进，失败保留同一authority/export；未构建新服务包或执行官方runc/Node E2E。G1历史8/8、G2计数/defaultOFF不变，6d已存性能数据保持历史身份并复用，不重复刷分。下一独立Node关闭时的忙引用所有权/排空小项；宿主独立开关及混合语义缺口仍未完成。[版本、原始失败与结果](../development/evidence/20261007-ownerfs-bind-cleanup-retry/README.md)。

**新增当前限定通过（2026-10-07，事实）：** main产品1451f60/map196a，复用已通过Linux构建，两个现有ELF包逐字节一致；当前Node受管workspace成功启动/权限errno双视图/正常停止通过，50驱动检查及30独立postcheck通过，Node/Meta实际wait0、四服务/监督PID及容器消失、保护身份不变。标准/性能/重启未重跑；G1历史8/8、G2计数和defaultOFF不变。宿主独立开关/通用排空及混合语义缺口仍未完成；下一既定workspace限定性能小项。 [版本与证据](../development/evidence/20261007-ownerfs-bind-node-accepted/README.md)。

**最新开发/交付入口（2026-10-07，决策）：main唯一入口。** 按用户授权，fix/native-orderly-recovery-20261007相对f09185e的全部有效修改（恢复、归属整改、测试、文档和证据）及本次已验证Node启动失败修复73842cd已无冲突fast-forward纳入本地main，本记录随main正常推送；远端实际提交/文件树以推送核对回执为准。其它分支逐项盘点无当前AFS有效遗漏；8个旧dirty工作树及1个untracked草稿原样保留并在源码树外归档、验证补丁可恢复。不创建PR/MR，不设单特性人工审批；必要Linux门禁继续，整体目标完成后统一项目review。合并不升级历史验收，G1历史8/8不重开。下一仍G2.12 accepted Node/workspace生命周期→G2.13性能；普通性能FAIL/复杂可靠性/fuser原版API阻塞后置或独立保留。[纳入、未纳入、版本和证据](../development/evidence/20261007-main-convergence/README.md)。下方按版本保留的历史记录不覆盖本入口。

**新增限定通过（2026-10-07，事实）：** Node控制器启动失败现进入既有服务排空/显式关闭链，保留首错且不发ready。df56基底/157-map196a4177，六项Linux源码门禁、5个Node测试及真实拒绝启动28checks通过（Node实际wait1/Meta wait0）。首工具错误格式判据FAIL保留；仅修工具后R2，源码/ELF/环境未变。不是accepted ON、通用drain、POSIX/性能或新试用包；G1历史8/8与G2计数不变。[版本及原始证据](../development/evidence/20261007-ownerfs-bind-node-startup/README.md)。

**新增限定通过（2026-10-07，事实）：** 165d基底/157-map6dd63d46，仅新增test-only真实FUSE用例；Linux fmt/测试编译/严格Clippy exit0，精确1PASS/585未选。实际物理Home ext4→FUSE一级native、双向fresh-open内容、EBUSY保留同一mount、子进程wait0后普通卸载/恢复原FUSE identity及正常outer umount/join通过；独立postcheck11项通过，无残留。不是Node宿主生命周期/full ON/POSIX/性能，新包未部署，G1历史8/8及G2计数/defaultOFF不变。

[当前单用例版本与原始证据](../development/evidence/20261007-ownerfs-bind-core-fuse/README.md)。

**当前整改收口（2026-10-07，事实）：** OwnerFs workspace bind mount核心已归属单个ownerfs/bind_mount.rs，runc保留独立适配层，配置原名兼容、默认OFF。6e基底/157-map fabab19a的新Linux受影响检查通过：25项独立测试（含4真实bind/3rootfs）、fmt/严格Clippy/release构建；独立静态审阅通过。新ELF尚未打包/部署，不继承3cc运行验收；G1历史8/8及G2计数不变。

[整改版本、命令与证据](../development/evidence/20261007-ownerfs-bind-remediation/README.md)。

**当前小阶段收口（2026-10-07）：** source3cc10a2/157-map e15c的新Linux release源码检查及4KiB容器有序恢复PASS，58项驱动检查/独立postcheck通过，四wait0、八服务PID与两容器消失、旧记录/模板/本地Meta目录保留；首collector路径FAIL保留、只修工具复核未重跑产品。不是全ON、性能或通用OwnerFs bind验收。G1历史8/8、G2计数及defaultOFF不变。

**当前入口纠正（事实/决策）：** 既有6d B-Home远端读/写/删除及DFS同步一写两读已有实跑证据，G2.16已完成，本轮未重跑。G2.12新增无runc的核心+真实FUSE限定检查现已通过；下一继续独立Node准入/生命周期边界，再G2.13性能。普通宿主可见性、一般引用排空及混合路径语义仍未验收，不因改名或本次单用例升级完整ON。

[本轮实际版本与证据](../development/evidence/20261007-native-orderly-recovery-runtime/README.md)。[独立整改计划](../development/ownerfs-workspace-bind-remediation.md)。

**最终修复受影响检查（2026-10-07）：** 磁盘已由已归档历史副本移出释放37.64GiB；固定Rust8c6f1d50/157-map e15c，Linux release四门禁通过，普通10PASS/7忽略及root实际7PASS，test ELF SHA884ba8f9。仅此范围，不称新服务ELF/包/重启PASS。下一必要剩余源码检查、构建、新包及4KiB有序恢复；不重跑旧标准/性能，G1/G2计数和defaultOFF不变。

[本次版本、原始命令与完整结果](../development/evidence/20261007-native-orderly-recovery-source/README.md)。

**VM归档移出完成（2026-10-07）：** 用户授权后只移除已完整校验的三历史目录，实际释放37.64GiB，磁盘100%→56%。主机9.49GiB归档保留、源端预移出清单及归档SHA复核一致；保护目录身份/旧ELF SHA不变，未扩盘/清当前缓存/动基线或工具链。磁盘阻塞解除，正在执行原计划最终修复的受影响Linux release检查；新包/实际重启尚未验收。

**构建VM历史归档（2026-10-07）：** 旧evidence/artifacts/logs已流式复制到主机，9.49GiB归档、11,852条目/10,661文件内容及基本元数据一致，源端前后无变化；VM原件保留，尚未释放空间，移除决定待用户答复。当前构建缓存/基线/工具链未动，最终修复验证仍阻塞。[归档回执](../development/evidence/20261007-vm-archive/README.md)。

**Owner标准复用审计（2026-10-07）：** e925→已交付6d OFF普通操作未变，构造/statvfs等价变化及既有守卫支持历史限定复用；pjdf有e925 release，LTP6/短FSx仅dev。当前安装/套件只读身份匹配，未重跑标准、不增加PASS或计数。[审计与原proof索引](../development/evidence/20261007-owner-standard-reuse/README.md)。afs-build满盘仍停分支等待用户决定，草稿验证/新包/实际重启尚未完成。

**当前状态（2026-10-07）：** 3FS基线资格按用户决定留专题，主线容器workspace。main f09185e、产品6d/157/map66/既有ELF包的通过范围保留；新rootfs私有副本/命令续号修复在独立分支fix/native-orderly-recovery-20261007，不视为新可用候选。G1历史8/8，G2为10限定完成/2性能FAIL/2bind进行中/13待验收，bind默认OFF。

**新增/阻塞：** 工具11个独立Linux guards通过；早期Rust10+4有输出但未冻结源码SHA，不计最终版本PASS。静态审阅发现并最小修正非root测试入口，原FAIL保留。afs-build85GiB根盘实际满：误用debug与root工具链下载、后续ENOSPC，已停止构建并求助；最终续号/源码门禁/新包/重启读回均未完成。[版本、失败与证据](../development/evidence/20261007-native-orderly-recovery/README.md)。

**下一项：** 等环境处理决定后一次准入，复用release缓存、普通用户构建、仅真实test binary特权运行；新包后只做4KiB两阶段正常重启读回，不重跑标准/性能或3FS。独立文本/审查与证据收口不受该阻塞。

**历史说明：** 下方旧检查点按原版本/范围保存；其旧下一动作由上方当前入口和验收主表覆盖。当前native偏移/经典锁/watch失败与普通性能失败原始记录均保留。

# Implementation Status

Updated 2026-10-07. [Three-stage acceptance checklist](../development/trial-release-goals.md) owns tasks and completion; [current code checkpoint](../development/current-checkpoint.md) binds this publication, validation and portable historical evidence.

| Stage | Status | Scope |
| --- | --- | --- |
| G1 colleague trial | **DONE, 8/8** | Historical g1.5: Linux build/offline install, memory demonstration, OwnerFs local/remote, DFS basic cross-node I/O, central local-file Meta restart, selfcheck and ordered lifecycle |
| G2 core performance version | **ACTIVE** | 27 independent tasks: 11 bounded outputs complete, 1 bind-function item in progress, 15 awaiting acceptance. Current Owner standards and short Owner/DFS FSx qualified in recorded scope; ordinary Owner local/remote read/write await the new MooseFS throughput and latency criteria; local R1 DFS statfs and fixed standards now pass; two-VM core recovery now passes; comparator qualification and new performance package remain open |
| G3 complex reliability/backends | **Deferred, 13 tasks** | Long-running/complex faults, expanded matrices/HA; etcd topic at 2GiB, Redis last |

## Current code capabilities

| Area | Present behavior | Acceptance boundary |
| --- | --- | --- |
| Runtime | afs-meta/afs-node, TLS gRPC, REST health, separate OwnerFs/DFS FUSE mounts | Current Linux source/tool gate is recorded with exact input hashes; current two-VM Owner/DFS core and orderly central Meta recovery pass in G2.08 scope; independent performance package delivery remains G2.27 |
| Meta | memory, local-file, etcd, Redis implementations; persistent capability is distinct from volatile state | G1 central local-file recovery is qualified on g1.5. Other backends have scoped historical tests; broad parity/faults remain G3 |
| OwnerFs | Home files, remote routing, write-authority/lease checks, error propagation, ordered namespace/index maintenance | B1–B4 internal correctness/evaluator outputs complete in limited scope; e925 local pjdfstest and fixed six-test LTP pass in their recorded scope; short FSx passes. Historical 64MiB/C1 local read/write correctness passed but failed the old ext4 throughput target; current ordinary local/remote read/write performance is pending under the new MooseFS throughput and latency rule because same-condition baselines and predeclared latency percentiles are incomplete. Current 6d small remote deletion correctness, quantified report and normal cleanup complete [G2.16](../development/evidence/20261007-owner-remote-delete-small/README.md) |
| DFS | immutable chunks, version commits, replica policies, coherent read plans, streaming integrity and shared verification within one read batch | Batch CPU/read-amplification diagnostic complete; R2/64MiB one writer and two independent Node reader views pass, including orderly central Meta recovery; performance comparison and three-sync-durable 3FS parity remain unqualified |
| Capacity/health | Observed backend capability and readiness, Owner local filesystem capacity/error handling | Owner D20 and current local R1 DFS real fstatvfs slices only; remote/replicated aggregate capacity and wider faults remain open |
| OwnerFs workspace bind mount | OwnerFs core in one bind_mount.rs, authorized physical Home source→FUSE first-level covering mount, normal identity-checked unmount; runc is a default-OFF experimental adapter | [Core ownership/naming plan](../development/ownerfs-workspace-bind-remediation.md); the independent default-OFF host switch covers a physical Home directory at the real FUSE first-level target; [931 fixed-Home host visibility/permissions/normal closure PASS](../development/evidence/20261007-ownerfs-workspace-host-runtime/README.md). The runc adapter uses a separate private namespace. Multi-workspace and generalized lifecycle remain open. [Runc adapter slice](../development/native-workspace-slice.md); [Official runc installation/runtime-only admission PASS](../development/evidence/20261007-runc-runtime/README.md); [Actual managed single-container lifecycle PASS](../development/evidence/20261007-managed-workspace/README.md); Mixed-path mmap bytes/permissions pass; locks, append offset and cross-watch fail in the [short semantic evidence](../development/evidence/20261007-managed-semantics/README.md). Managed ON functional/performance exit, production READY/revocation/drain and restart reconciliation remain unqualified |
| Packaging | Process control, offline package generation, trial configuration and selfcheck | g1.5 retained; main6d51aeb default-OFF fresh compiler-free install and bounded recovery35checks pass; [fixed trial package](https://github.com/lelezi257/dms/releases/tag/afs-trial-6d51aeb) is available. not a performance release or container ON package |
| Transport | gRPC and optional RDMA code plus scoped fault proofs | Actual RXE short results do not qualify the whole RDMA exception/resource matrix or physical NIC performance |

## Validation boundaries

Historical full pjdfstest results belong to v37/v48. Historical e925c5b Owner local pjdfstest passes 236 files/8819 checks (28 upstream TODO, zero skips/unexpected failures); this is not full POSIX certification. The e925 DFS ENOSYS/zero-TAP/six-TBROK failures remain historical receipts. The new map6161e25b release candidate passes local R1 DFS pjdfstest236/8819 with28TODO and zero skips/unexpected failures, plus fixed LTP6/6 (651 unselected), with normal stop/unmount. [Historical candidate evidence](../development/evidence/20261007-dfs-statfs/README.md); [6d scoped reuse audit](../development/evidence/20261007-standard-reuse/README.md), with no new6d full-suite run. Owner/DFS short FSx passes seed1/1000 only. [Current raw ledger](../development/evidence/20261006-e2e-current/README.md) records the scope and failures. ext4 reference results are not AFS passes. The original full 69-case manifest remains NOT_RUN with environment PREPARING; it is the expanded final catalogue, not the G1 progress denominator.

See [checkpoint results](../development/checkpoints/20261006-current/results/README.md) for fresh combined-source checks. The historical 271-check g1.5 audit, DFS batch paired results, ext4 finite-tool positive results and complete parent-FD failed experiment are available in the repository. Other archived scopes are explicitly identified as external archives in the checkpoint page.

## Next and priority

Latest user priority: standard pjdfstest for functional completeness; OwnerFs workspace bind mount access from Issue42/PR43 is the first performance lane, after necessary default-OFF bind functionality/safety. Ordinary local/remote/DFS measurements can remain baseline reports with optimizations deferred. [R1 historical evidence cleanup](../development/repository-remediation.md) passed restoration and fixture checks; R2 upstream migration remains independently BLOCKED by public lock/cancellation API gaps, with the full diff recorded. This does not block independent functional or workspace bind work.

Reuse passed current standards and single-/two-VM core recovery. Preserve both dev and same-source release local performance failures as old-criterion history. Release build, Owner standards/basic operations and single-VM orderly Meta recovery passed; ordinary read/write baselines are retained and targeted tuning is deferred. Remote comparisons require independent qualification. When entering DFS, one-writer/many-reader is highest priority. Each case freezes its interface, data, concurrency, cache, durability barrier, applicable replica semantics, latency percentile, fairness and budget; a missing complex baseline or large-disk condition does not block an independent safe small case. Ordinary Owner read/write target is throughput >=1.2x same-condition MooseFS and operation latency <=0.8x MooseFS; OwnerFs workspace bind remains near native ext4; DFS matches 3FS under the same POSIX interface and three synchronous durable copies. Delete requires correctness and a quantitative report, without a new hard ratio.

Bind functionality and performance are separate G2.12/G2.13 exits, with an explicit experimental switch default OFF. OFF delivery proceeds independently; known ON safety gaps must be resolved before enabling ON. Complex reliability/etcd/Redis work remains later, while ordinary-use corruption, unsafe success, permissions and core recovery defects are repaired immediately.
