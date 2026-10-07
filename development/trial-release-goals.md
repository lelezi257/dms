**2026-10-08 历史远端时延核对（事实/纠正）：** 6d普通远端读和写均已记录每轮实际操作p50/p95/p99；Linux只读审计核对824个历史证据文件SHA及20份测量payload/原始stdout/工具来源，941项一致性检查通过，均非新产品测试。缺口是原始间隔数组、测前固定判定分位数及缓存/持久/退出资格，不能笼统说没有独立时延，也不由五个p95重建pooledp95。原读mfsmount wait1/写durable-ACK限制及原结论保留；不把6d计时迁移为f03性能。当前f03/7bfc产品未改，G1历史8/8/G2计数不变。下一单项为当前远端小读缓存策略/原始时延准入；现有官方Moose4.59.2的DIRECT选项已只读核对，尚未启动新测量，不死磕旧资格。 [独立审计与复用边界](evidence/20261008-owner-remote-latency-reuse/README.md)。以下保留原时点记录。

**2026-10-08 发布恢复（事实）：** GitHub新故障记录确认16:52–17:01 UTC受影响且17:27已缓解，按用户授权一次正常push成功。[mainc87707d6](https://github.com/lelezi257/dms/commit/c87707d6dc28377094a2224a07e51e4344d8a5bb)已纳入本地读64888834、本地写9a3a272c、删除历史复用18554a47及bind FD/mmap排空组件回归c87707d6；独立ls-remote/fetch与完整递归文件树一致。旧500/本轮观察器失败保留，不更改验收结论。下一普通远端小读独立时延准备；G1历史8/8/G2计数/full ON/性能资格不升级。[发布核对](evidence/20261008-ownerfs-bind-reference-drain/publication-recovery.json)。此发布状态覆盖下方原时点“待推送”记录。

**2026-10-08 新增限定回归（事实）：** G2.12真实OwnerFs FUSE/物理Home组件的普通文件FD与mmap引用排空通过：本地授权缓存失效后校验拒绝，FD持有时普通卸载EBUSY且身份不变；关闭FD后存活的只读共享映射仍EBUSY，unmap后正常detach及outer FUSE unmount/join。Linux fmt/release测试编译/strict Clippy、精确1PASS（600未选）及17项退出/身份审计通过；三保护FDB进程/26 mount库存未变。仅测试fixture改变，f03/7bfc普通候选未重建/部署；两个观察器ENOSYS失败、前置输入/格式失败及原数据保留。不是即时FD/mmap撤权、真实Meta撤权、通用drain/full ON/POSIX/性能；默认OFF、G1历史8/8/G2计数不变。发布按用户决定等待GitHub恢复，最后远端8d989bf5，不反复push。下一既定普通远端小读的独立操作时延边界/准入准备；已过标准/宿主生命周期/八bind性能不重开。[证据](evidence/20261008-ownerfs-bind-reference-drain/README.md)。以下保留原时点记录。

**2026-10-08 顺序核对（事实/决策）：** G2.11保留e925历史限定完成；源码影响图及f03 OFF安装中的两次基本unlink支持普通功能范围复用，不称当前100文件/5对重测或沿用旧性能数字。取消尚未启动的重复删除轮次；下一回既有G2.12授权变化/native FD与mmap排空实现边界，不重新做已过宿主生命周期或8核心bind性能。[复用边界](evidence/20261008-owner-local-delete-reuse/README.md)。普通本地读写新数据仍仅摸底；GitHub500发布状态另见回执。以下保留原时点记录。

**2026-10-08 本地写独立小项（事实）：** 当前f03/7bfc产品未改，B/ext4/local-file/gRPC/OFF；64MiB/C1新建+fdatasync，1预热5交替配对，12文件全SHA/EOF/权限与六个Moose本地VALID副本通过。独立Linux复算各320时延样本：Owner/Moose中位909.994/1170.143MiB/s、合并p95 1184309/1374228ns；仅摸底，Moose强durable-ACK及底层缓存资格未过，正式G2.10仍待验，不改1.2/.8双判据。两AFS/三Moose actualwait0、8PID/挂载/UDS闭合，原库存和157编译输入不变。日志14251B/19ERRO+18WARN及首次导出guard拒绝保留，无产品重跑/环境修补。G1历史8/8/G2计数不变；上轮64888834被GitHub500拒绝、最后远端8d989bf5，发布状态另以推送回执为准。下一基础本地小删除，不死磕此组性能。[证据](evidence/20261008-owner-local-write-latency/README.md)。以下保留原时点记录。

**2026-10-08 本地读独立小项（事实）：** f03/7bfc不变，C探针按需原始时延样本及5实测Linux守卫通过。B单机正式hot比较在首个Owner计时前因0驻留页拒绝，0有效配对；另行测前冻结repeat诊断完成，Owner/Moose中位5937.148/18687.490MiB/s，合并p95 206000/54542ns，各320原始样本。两侧mincore0/64MiB不同，仅摸底，正式G2.09仍待验，不改1.2/.8双判据。两轮5actualwait0及8进程/挂载/UDS闭合，原库存不变；所有失败、日志和源版本索引保留。G1历史8/8/G2计数不变，下一基础本地小写，缓存可比条件列专项而非死磕。[原始证据](evidence/20261008-owner-local-read-latency/README.md)。以下保留原时点记录。

**2026-10-08 当前试用分项（事实）：** f03/map2b17新普通包7bfc在Linux两次复现、无编译器现有VM独立安装通过：OwnerFs/DFS各64MiB正常local-file Meta重启全SHA/EOF读回，43基础检查、三actualwait0/六PID及保护库存闭合。Linux完整包比较证明与f03多节点/删除原包仅指南/清单/SHA变更，按原范围复用，不重跑。INFO日志26ERRO/13WARN完整保留。G1历史8/8关闭，G2计数及完整G2.27性能出口不变；固定[f03 prerelease](https://github.com/lelezi257/dms/releases/tag/afs-trial-f03dc2b)已发布且四附件远端SHA一致，旧6d/7e6资产不变；普通Owner双指标/完整bind功能仍待验。[版本与紧凑证据](evidence/20261008-current-trial-f03/README.md)。以下保留原时点记录。

**2026-10-08 发布恢复（事实）：** GitHub Git Operations恢复后一次正常push成功，[main7eedd7ec](https://github.com/lelezi257/dms/commit/7eedd7eca3c1fb810663b3a491b35b18b9650f03)已含f03 create修复及新候选三节点读写/小删除证据。独立ls-remote/fetch、完整19282 blob/21070-entry tree及commit→tree核对一致；旧两次500和错误tree-ID验证假设原记录保留，不改历史验收。bind核心/测试探针归属确认，旧子目录/default产品探针不在。G1历史8/8/G2计数不变；下一[f03普通试用候选的可重复打包与独立安装/正常Meta恢复](current-trial-f03-slice.md)，仍非完整G2.27性能退出。 [发布证据](evidence/20261008-github-publication-recovery/README.md)。以下保留原时点记录。

**2026-10-07 当前新增：** f03dc2b3/map2b17同候选DFS小删除功能/计时完成：local-file/configuredR3/gRPC/OFF，100×4KiB×6轮（1预热5测），600内容核验/删除成功，B/C各600 ENOENT、三远端driver退出、四actualwait0/八服务PID闭合、11保护进程/完整mount库存不变。Linux存储证据独立核验PASS，测量中位27.003911/pooled27.927077ops/s仅摸底，全部递减样本保留；旧6d R2不可直接比率，正式3FS/时延/物理回收未验。INFO日志720097B/2418ERRO+600WARN完整留源码树外，分类计数入紧凑索引；工具R1权限拒绝在启动前，R2修staging流程，无环境修补/产品重跑。最大阶段采样408625152B<2GiB，当前guest根保留待完整归档/实际恢复再清理。G1历史8/8/G2计数不变。GitHub500发布按用户决定等待恢复，最后确认origin9a8；当前成果仅本地main。下一G2.27必要当前组合/试用交付，不再调优此摸底或修3FS资格。 [当前版本、失败、结果与索引](evidence/20261007-dfs-delete-current/README.md)。以下保留原时点记录。

**2026-10-07 当前新增：** 产品f03dc2b3/map2b17的DFS小规模多节点local-file/R3/gRPC运行和数据完成：三写者各64MiB、1预热1测量及两跨节点读路由，6文件/18 C样本/288物理份，四actualwait0/八服务PID及24远端worker闭合，11保护进程/完整mount库存不变。观察器旧ELF标签FAIL保留，仅R2只读复核，未重跑产品；INFO日志20,155B/23ERRO/60WARN完整保留，不称零错误。A已停止本case完整归档/Linux239条恢复后释放500,867,072B，free回2,327,810,048B。G1历史8/8及G2计数不变，正式3FS/时延/完整POSIX待验。GitHub两次正常推送500，local main修复在f03，origin仍9a8；用户决定等待恢复再推送，独立Linux继续。下一G2.25小删除和G2.27必要组合。 [版本、数据、失败及恢复索引](evidence/20261007-dfs-r3-multinode-current/README.md)。以下保留原时点记录。

**2026-10-07 新增源码小项：** DFS create只对父目录revision/mtime/ctime漂移作64次有界重试，同名/权限/属性变化和真实冲突仍报错、原OperationId/inode/lease/digest不变。Linux6针对性+14 namespace回归、fmt/check及未放宽的strict all-features Clippy PASS；原并发创建FAIL/default-feature lint失败和工具准备记录完整保留。未构建/部署新候选，旧7e6运行FAIL及历史通过不改，G1历史8/8关闭、G2计数不变。下一独立构建新release，再新候选local-file/R3/gRPC小规模多节点回归；环境按B/C42GiB、A/ctl保持运行。 [命令、范围和证据](evidence/20261007-dfs-create-contention/README.md)。以下保留原时点记录。

**当前覆盖说明（2026-10-07，决策）：** 普通OwnerFs本地/远端核心读写采用[普通OwnerFs性能准则](ownerfs-performance-criteria.md)：吞吐>=同条件MooseFS的1.2倍，操作时延<=同条件MooseFS的0.8倍，两项分别测量且同时满足；吞吐默认配对中位数，时延默认测前声明的逐操作样本p95。G2当前主表口径为11项限定完成、1项bind功能进行中、15项待验收；下方日期快照的旧ext4/持平判据、旧FAIL和旧计数保持历史身份。

**2026-10-07新增事实：G2.22当前7e6/c3bb的A单读者64MiB小项完成内容/计时/正常闭合，正式3FS对照仍待验。** 1预热5读，中位65.624896MiB/s；前后48物理副本，四actualwait0/八PID消失及11保护进程/完整mount库存不变。Linux11工具guards及独立观察校验通过；首轮遗漏结果目录导致写前拒绝，原FAIL及四正常退出保留，修测试准备后一次数据运行，无Rust/vendor/VM修补。G1历史8/8、G2大项计数/defaultOFF不变，G2.23写性能/G2.27/full bind仍开放。当前OFF标准限定复用已由既有impact-map及当前安装恢复覆盖，不重跑整套；下一G2.23三同步副本小写入摸底。 [证据](evidence/20261007-dfs-r3-local-read/README.md)。

以下保留原时点记录。

**当前独立出口（2026-10-07，事实）：G2.13完成，限定当前7e6/C1的8个小规模核心case。** 同公开c3bb包/map151a，新增64MiB写0.969905/读0.965910×ext4，复用同候选六元数据0.964347–1.032311；全部>=测前0.90。189驱动/258 Linux独立checks及15受影响工具guards通过，四actualwait0/十二服务监督及OCI PID与正常mount闭合，一保护进程/26 mount完整库存不变，峰值245153792B<256MiB。OFF写0.728219/读0.457996 FAIL保留暂缓，不重测刷分；无Rust/vendor/C修改或重建。G1历史8/8不变；G2变为11限定完成/2普通性能FAIL/1bind功能进行中/13待验收。G2.12 full ON/复杂语义、全部PR43组合/冷热耐久/大规模/正式MooseFS及3FS资格不升级，默认OFF、Goal ACTIVE。下一当前7e6 OFF标准适用性/限定复用审计，仅真实受影响缺口才补测，再更新核心性能交付状态。 [版本、原始数据及8项组合账本](evidence/20261007-workspace-bind-data-current/README.md)。

以下保留原时点记录；当前入口以上方为准。

**新增当前DFS恢复小项（2026-10-07，事实）：** 产品7e6/map151a/公开c3bb普通包未变，64MiB非重复一写两读、重启前后各48物理chunk及三份Ready/Durable副本通过；Meta仅一次正常重启，三Node/FUSE/UDS身份不变。五actualwait0/十所属PID消失、三mount闭合及11保护进程/完整mount库存不变；峰值597,450,752B<1GiB。首观察器错误要求旧生命周期目录保留，原FAIL和重启前保存的退出回执不改，R2只读复核及12 guards通过；无产品重跑/环境修补。7+14 Linux工具guards通过，无Rust/vendor改动；不继承931计时或称3FS/fullG2 PASS，G1历史8/8与大项计数/defaultOFF不变。下一当前7e6 workspace64MiB读写配对，普通FAIL及复杂/后端专题后置。 [版本、原始证据与范围](evidence/20261007-dfs-r3-current-recovery/README.md)。

**新增DFS三副本小项（2026-10-07，事实）：** 冻结产品931/map9661/既有ELF包未变，64MiB非重复内容一写/B+C两读通过；16不同chunk的三节点Ready/Durable及48物理文件先核对，完整前后SHA/EOF、1预热5同步读通过。共同窗口总吞吐中位98.052181MiB/s，B/C五轮C中位49.084466/52.197064；缓存/RPC未观察、同物理host三VM，不计3FS性能达标。四actualwait0/八服务监督incarnation消失、三FUSE/UDS正常闭合、11旧进程及完整mount库存不变，峰值607,477,760B低于1GiB。工具首错/关闭观察器误判和warmup汇总错均保留，只读修正复核、无产品重跑/环境修补。G1历史8/8/G2大项计数/defaultOFF不变，7e6最新DFS回归不继承。下一当前main的受影响DFS兼容/恢复小项，再新试用交付账本；普通性能/大规模/3FS资格后置。[版本、内容、计时、失败及回执](evidence/20261007-dfs-r3-small/README.md)。

**新增测试边界整改（2026-10-07，事实）：** 产品main7e6e00a6/157-map151a2c6d：探针源码100%迁移到tests/support/workspace_probe.rs，显式Cargo example；默认cargo build与--bins仅两个产品binary，普通包无探针。实验适配器要求测试/管理员提供idle_command和identity_command，保留身份/授权/错误/正常排空检查；core/宿主bind无探针依赖、默认OFF、第三方未改。Linux11源码门禁/29选定Rust测试（含8特权）及4工具测试通过；新默认包实际单次启动/identity/读取/active Node正常关闭及独立postcheck通过，两个wait0/4服务与监督PID+容器/mount闭合、保护对象和模板不变。不是全ON/POSIX/性能资格；旧931 E2E候选、历史通过/失败和G1历史8/8均保留原身份，大项计数不变。下一返回已准备的DFS R3小规模一写两读，931结果保持自身版本；7e6的新DFS回归单列，不自动继承。[版本、命令及证据](evidence/20261007-workspace-probe-boundary/README.md)。

**新增DFS R3准备（2026-10-07，事实）：** 冻结产品931/map9661/packagec7未变；Linux新64MiB非重复探针5项、当前driver4项、fixture12项检查通过，四role真实配置/TLS/ELF/容量准入PASS、合计399,278,080B。没有启动服务/数据/计时，不计G2.21通过；原工具与准入FAIL保留。按用户新任务，在此小阶段收尾后先整改workspace probe产品/测试边界，再回DFS一写多读。G1历史8/8和大项计数不变。[身份、原始输出与范围](evidence/20261007-dfs-r3-preparation/README.md)。

**当前新增元数据观察（2026-10-07，事实）：** maine903基底/产品93169c8与157编译输入/ELF包未变。OFF1000×4KiB/C1六阶段完成1预热5配对、全部性能FAIL留数；ON首个预热内容正确，但全节点计数窗口readdir4未满足预定0，停止且ON比较未完成。窗口含遍历FUSE树的容量检查，不能把回调归因于业务或改判据称PASS。135驱动检查134PASS/1FAIL；117独立证据/正常闭合检查与13 Linux工具测试PASS，不等同用例PASS。实际Meta/Node四wait0、容器/PID/mount闭合、保护对象与原预算均核实；无Rust/vendor/C改动、重建、环境修补或刷分。G1历史8/8及大项计数/defaultOFF、先前数据子项PASS不变；下一DFS一写多读，计数归因/元数据ON补测单列后置。 [版本、FAIL、全部OFF数据与闭合](evidence/20261007-workspace-bind-metadata-perf/README.md)。main仍唯一入口，修复分支全部有效成果已纳入，旧8dirty工作树/草稿保留且HASH复核一致；无PR/审批关卡。

以下按原时点保留历史身份。

**新增当前workspace数据性能子项完成（2026-10-07，事实）：** 产品93169c8/157编译输入与既有ELF包未变，普通官方runc容器绑定宿主workspace，OFF/独立宿主ON/ext4同候选64MiB/C1/1MiB块、1预热5配对；ON写0.944934/读1.036200×ext4配对速度达到预设0.90。OFF写0.738207/读0.348338 FAIL保留暂缓，不刷分。183驱动/232独立检查、15 Linux工具测试PASS；48原始C输出/48scrape、每轮OFF阳性/ON数据回调0、正常wait0/PID与mount闭合/保护身份/248,127,488B峰值预算均核实。缓存未观察、写后读小样本范围和单轮波动明确；不是全G2.13/full ON/当前POSIX或新试用包。G1历史8/8和总体大项计数/defaultOFF不变。下一小规模workspace元数据，然后DFS一写多读；普通性能、广义排空/混合语义/复杂可靠性仍分项后置。 [精确版本、原始数据和独立复核](evidence/20261007-workspace-bind-data-perf/README.md)。

以下保留原时点/原版本记录。

**新增当前宿主运行小项通过（2026-10-07，事实）：** main产品93169c8/157编译输入未变，两次包字节一致；隔离Linux/ext4中无runc的真实Node宿主bind通过55驱动/50独立检查，12工具测试PASS。物理Home目录覆盖FUSE一级workspace，普通UID501双向64KiB内容、UID502 EACCES13、六类native数据/目录回调增量0、Meta/Node实际wait0及正常mount闭合通过；native背景getattr12保留，不冒称全部回调0。探针二次stat及外层旧回执路径错误FAIL保留，仅改工具后限定回归/复核；源码/ELF/环境不变。G1历史8/8、G2计数/defaultOFF不变；当前限定功能不继承标准/性能/full ON。下一继续既定workspace性能，旧6d摸底原身份复用，广义撤权/排空/mixed语义和复杂可靠性后置。 [版本、原始结果与失败](evidence/20261007-ownerfs-workspace-host-runtime/README.md)。

以下保留原时点/原版本记录。

**当前宿主入口源码切片通过（2026-10-07，事实）：** main0e1d059基底/157-map9661a313；新增与runc无关的默认OFF独立宿主开关，只覆盖一个已存在的本地Home workspace。核心仍在单文件ownerfs/bind_mount.rs，Node持有worker并在FUSE关闭前正常卸载/join；旧容器配置兼容且两模式互斥，第三方未改。Linux12受影响门禁及50选定测试PASS（含5实际root bind），严格Clippy/release构建通过；新Node ELF9478f3e8已标识，未打包/部署。原格式、编译、ENOSYS、准入与输入冻结FAIL/BLOCKED全部保留；当前运行/标准/性能不继承。G1历史8/8、G2计数不变。下一是一次真实Node无runc的宿主可见性/内容/正常关闭验收，自动接管、多workspace、通用撤权/排空/重启和mixed append/经典锁/watch仍待验。 [版本、原始结果与失败](evidence/20261007-ownerfs-workspace-host-entry/README.md)。

**当前源码小项（2026-10-07，事实）：** mainea73174基底/157-map58a71572，新增第一方FUSE callback指标，34实现回调入口计数、Node共享Registry接线；不改第三方、缓存/TTL/I/O/权限/生命周期。Linux5受影响门禁含新release构建、14FUSE dispatch测试及19Python工具测试PASS，新工具自互斥首FAIL保留。尚无新ELF运行计数/性能结论，下一仅实际短请求见证；原6d性能和d82关闭证据保留自身身份。G1历史8/8、G2计数/defaultOFF不变。 [源码版本与证据](evidence/20261007-ownerfs-workspace-callback-source/README.md)。

**当前运行子项通过（2026-10-07，事实）：** main产品d82cc7d/157编译输入未变，预构建ELF两次打包字节一致；现有官方runc VM里workspace仍FinalVerified时直接停止Node，无public workspace Stop。43驱动/19独立检查通过，Node/Meta实际wait0及四服务/监督PID、容器消失，FUSE/control正常闭合，旧安装/保护身份不变。21项Linux工具测试通过；没有Rust/第三方改动。G1历史8/8、G2计数/defaultOFF保持，不等同完整ON/性能达标。下一回workspace核心性能，复用已有摸底，保留宿主独立开关/通用撤权及混合语义未完成项。 [版本及证据](evidence/20261007-ownerfs-bind-active-node-stop/README.md)。

**以下记录保留原时点/原版本。**

**新增当前限定通过（2026-10-07，事实）：** main59f8753基底/157-map1fd613e5，Node保留workspace worker并在FUSE关闭前等待；真实EBUSY正常重试，终止性错误保留claim，监听失败先通知关闭。Linux9门禁（含release构建）及26选定测试通过，旧EBUSY FAIL和首测试编译FAIL保留。尚未打包或执行新Node/runc整机关闭；下一仅验该运行小项。G1历史8/8、G2计数/defaultOFF不变，历史标准/性能身份不升级。[证据](evidence/20261007-ownerfs-bind-shutdown-drain/README.md)。

**新增当前限定通过（2026-10-07，事实）：** main253057f基底/157-map a062bfe8，两个真实Linux挂载清理重试缺陷先2FAIL复现、修复后2PASS；普通10测试、配置7测试及七项受影响源码门禁通过。仅确认clone卸载/容器删除阶段推进，失败保留同一authority/export；未构建新服务包或执行官方runc/Node E2E。G1历史8/8、G2计数/defaultOFF不变，6d已存性能数据保持历史身份并复用，不重复刷分。下一独立Node关闭时的忙引用所有权/排空小项；宿主独立开关及混合语义缺口仍未完成。[版本、原始失败与结果](evidence/20261007-ownerfs-bind-cleanup-retry/README.md)。

**新增当前限定通过（2026-10-07，事实）：** main产品1451f60/map196a，复用已通过Linux构建，两个现有ELF包逐字节一致；当前Node受管workspace成功启动/权限errno双视图/正常停止通过，50驱动检查及30独立postcheck通过，Node/Meta实际wait0、四服务/监督PID及容器消失、保护身份不变。标准/性能/重启未重跑；G1历史8/8、G2计数和defaultOFF不变。宿主独立开关/通用排空及混合语义缺口仍未完成；下一既定workspace限定性能小项。 [版本与证据](evidence/20261007-ownerfs-bind-node-accepted/README.md)。

**最新开发/交付入口（2026-10-07，决策）：main唯一入口。** 按用户授权，fix/native-orderly-recovery-20261007相对f09185e的全部有效修改（恢复、归属整改、测试、文档和证据）及本次已验证Node启动失败修复73842cd已无冲突fast-forward纳入本地main，本记录随main正常推送；远端实际提交/文件树以推送核对回执为准。其它分支逐项盘点无当前AFS有效遗漏；8个旧dirty工作树及1个untracked草稿原样保留并在源码树外归档、验证补丁可恢复。不创建PR/MR，不设单特性人工审批；必要Linux门禁继续，整体目标完成后统一项目review。合并不升级历史验收，G1历史8/8不重开。下一仍G2.12 accepted Node/workspace生命周期→G2.13性能；普通性能FAIL/复杂可靠性/fuser原版API阻塞后置或独立保留。[纳入、未纳入、版本和证据](evidence/20261007-main-convergence/README.md)。下方按版本保留的历史记录不覆盖本入口。

**新增限定通过（2026-10-07，事实）：** Node控制器启动失败现进入既有服务排空/显式关闭链，保留首错且不发ready。df56基底/157-map196a4177，六项Linux源码门禁、5个Node测试及真实拒绝启动28checks通过（Node实际wait1/Meta wait0）。首工具错误格式判据FAIL保留；仅修工具后R2，源码/ELF/环境未变。不是accepted ON、通用drain、POSIX/性能或新试用包；G1历史8/8与G2计数不变。[版本及原始证据](evidence/20261007-ownerfs-bind-node-startup/README.md)。

**新增限定通过（2026-10-07，事实）：** 165d基底/157-map6dd63d46，仅新增test-only真实FUSE用例；Linux fmt/测试编译/严格Clippy exit0，精确1PASS/585未选。实际物理Home ext4→FUSE一级native、双向fresh-open内容、EBUSY保留同一mount、子进程wait0后普通卸载/恢复原FUSE identity及正常outer umount/join通过；独立postcheck11项通过，无残留。不是Node宿主生命周期/full ON/POSIX/性能，新包未部署，G1历史8/8及G2计数/defaultOFF不变。

[当前单用例版本与原始证据](../development/evidence/20261007-ownerfs-bind-core-fuse/README.md)。

**当前整改收口（2026-10-07，事实）：** OwnerFs workspace bind mount核心已归属单个ownerfs/bind_mount.rs，runc保留独立适配层，配置原名兼容、默认OFF。6e基底/157-map fabab19a的新Linux受影响检查通过：25项独立测试（含4真实bind/3rootfs）、fmt/严格Clippy/release构建；独立静态审阅通过。新ELF尚未打包/部署，不继承3cc运行验收；G1历史8/8及G2计数不变。

[整改版本、命令与证据](evidence/20261007-ownerfs-bind-remediation/README.md)。

**当前小阶段收口（2026-10-07）：** source3cc10a2/157-map e15c的新Linux release源码检查及4KiB容器有序恢复PASS，58项驱动检查/独立postcheck通过，四wait0、八服务PID与两容器消失、旧记录/模板/本地Meta目录保留；首collector路径FAIL保留、只修工具复核未重跑产品。不是全ON、性能或通用OwnerFs bind验收。G1历史8/8、G2计数及defaultOFF不变。

**当前入口纠正（事实/决策）：** 既有6d B-Home远端读/写/删除及DFS同步一写两读已有实跑证据，G2.16已完成，本轮未重跑。G2.12新增无runc的核心+真实FUSE限定检查现已通过；下一继续独立Node准入/生命周期边界，再G2.13性能。普通宿主可见性、一般引用排空及混合路径语义仍未验收，不因改名或本次单用例升级完整ON。

[本轮实际版本与证据](evidence/20261007-native-orderly-recovery-runtime/README.md)。[独立整改计划](ownerfs-workspace-bind-remediation.md)。

**当前状态（2026-10-07）：** 3FS基线资格按用户决定留专题，主线容器workspace。main f09185e、产品6d/157/map66/既有ELF包的通过范围保留；新rootfs私有副本/命令续号修复在独立分支fix/native-orderly-recovery-20261007，不视为新可用候选。G1历史8/8，G2为10限定完成/2性能FAIL/2bind进行中/13待验收，bind默认OFF。

**新增/阻塞：** 工具11个独立Linux guards通过；早期Rust10+4有输出但未冻结源码SHA，不计最终版本PASS。静态审阅发现并最小修正非root测试入口，原FAIL保留。afs-build85GiB根盘实际满：误用debug与root工具链下载、后续ENOSPC，已停止构建并求助；最终续号/源码门禁/新包/重启读回均未完成。[版本、失败与证据](evidence/20261007-native-orderly-recovery/README.md)。

**下一项：** 等环境处理决定后一次准入，复用release缓存、普通用户构建、仅真实test binary特权运行；新包后只做4KiB两阶段正常重启读回，不重跑标准/性能或3FS。独立文本/审查与证据收口不受该阻塞。

# AFS 三阶段目标与独立验收清单

2026-10-06，按本轮用户讨论对齐。本文是目标、优先级、独立验收项和出口的唯一主表，替代旧G2.2a/b/c整套准备优先的执行顺序。旧实验/失败/检查点保留原身份；[完整验收目录](../docs/acceptance.md)按本表分阶段执行。

## 总纲与状态口径

**最新用户优先级（2026-10-07）：** pjdfstest优先保证功能完备性。性能第一优先级是Issue42/PR43的OwnerFs workspace bind mount访问（G2.12必要功能/安全出口→G2.13性能），显式开关默认OFF。普通OwnerFs本地/远端核心读写的新目标见[普通OwnerFs性能准则](ownerfs-performance-criteria.md)：吞吐>=同条件MooseFS的1.2倍，操作时延<=同条件MooseFS的0.8倍，两项分别验收且都要满足；旧ext4/持平数据保留原判据和原结论。DFS性能可先摸底留数据，未达标的专项优化暂缓，不无输入复测。[高优先级仓库整改](repository-remediation.md)R1已验证；R2原版fuser迁移因公开API缺口独立阻塞，其它功能/容器路径继续推进，不重开G1。

**决策：** 先交付简单稳定的试用版，再从小规模核心case提高性能，最后复杂可靠性。Meta：memory演示→local-file持久恢复→etcd→Redis；OwnerFs优先，DFS内部一写多读优先。正常使用的数据损坏、错误成功、越权或核心恢复缺陷立即修复。

**决策：** 小项预先确定范围、样本/资源预算、通过线与本轮止损点；完成后保持完成状态，新候选/新规模另列回归，不无输入重复已过项。卡点仅阻塞依赖它的case，不把全套工具、冷热证明、完整100GiB准备或native缺口作为所有工作的前置。失败收益实验可结案，不无限优化残差，不后验修改阈值。

**口径：** 完成=指定版本/范围有通过证据；进行中=有成果但出口未过；待验收=无本项合格结果；后置=阶段三。每项完成记录source/提交与ELF/包、命令、原始结果、判据和限制；每次汇报新增完成、失败、暂缓和下一项。历史失败结案不等于性能通过。

| 阶段 | 独立交付结果 | 当前可信状态 |
| --- | --- | --- |
| 阶段一 G1 | 同事独立安装OwnerFs/DFS；memory演示；中心local-file Meta可重启恢复 | **已完成，8/8；推荐g1.5**。新候选回归不重开G1；不是完整POSIX/69项认证 |
| 阶段二 G2 | 标准回归；OwnerFs核心性能；DFS一写多读；可开关bind功能/性能 | **进行中，27项：11项限定完成、1项bind功能进行中、15项待验收**。Owner普通读写旧ext4/远端持平数据保留，按新MooseFS吞吐/时延双判据均待验收；DFS本地R1标准缺口已补齐，旧失败保留；系统对照/新性能包未完成 |
| 阶段三 G3 | 长时间、复杂并发/故障、扩展矩阵和最后的后端 | **后置，13项**；局部证据保留，不称整体验收 |

## 启动前收尾（不重开阶段一）

**当前6d候选补充（2026-10-07）：** [默认OFF独立安装/恢复新回执](evidence/20261007-installed-off-6d/README.md)，Owner/DFS各64MiB精确内容、中心有序恢复、正常清理PASS；35checks，不关闭G2.27全性能出口。同包复现已PASS，[限定试用清单](../docs/guides/trial-6d.md)已审阅，prerelease发布准备中；下一Owner远端小读/删独立摸底；必要ON缺口保留，不死磕相同诊断。

**历史候选补充（2026-10-07）：** main25a8061的默认OFF独立安装、Owner/DFS各64MiB基础与中心local-file有序恢复/正常关闭通过；[版本/证据](evidence/20261007-installed-off/README.md)。属于G2.08当前回归分支，27项计数不变；不是G2.27性能包出口、标准全集或容器ON资格。

| ID | 独立任务与出口 | 当前状态 |
| --- | --- | --- |
| G0.01 | 整理试用版/候选/保留与暂缓改动、实验失败及身份 | **完成**；current-checkpoint.md区分版本、失败与未交付项 |
| G0.02 | 固化本表与导航，完成状态有证据，新项有出口 | **本轮文档更新完成** |
| G0.03 | 收尾代码/文档同步GitHub：可审查提交、身份和验证结果 | **完成（本次代码/文档发布检查点）**；源/工具门禁18条通过，提交包含代码及相关文档，Git身份与结果见current-checkpoint.md |
| G0.04 | [独立容量维护/分级预算](vm-capacity.md)，扩盘前保存状态与备份、恢复后重新准入 | **B/C各32→42GiB及完整恢复核验完成**；A/ctl因用户保留memory状态暂缓，afs-build不扩；不重开G1、不阻塞具备独立条件的小项 |

## 阶段一 G1：可运行、可测试、中心local-file恢复

**事实：** 按已交付内测试用版验收，不追溯升级为全集POSIX、全部后端、8GiB或8小时验收。[g1.5包与说明](current-checkpoint.md#historical-evidence)、[271项Linux核对](current-checkpoint.md#historical-evidence)、[g1.5范围](current-checkpoint.md#historical-evidence)、[原G1跨节点结果](current-checkpoint.md#historical-evidence)、[g1.5同ELF恢复](current-checkpoint.md#historical-evidence)可复核。

| ID | 独立验收项 | 已通过出口与版本范围 | 状态 |
| --- | --- | --- | --- |
| G1.01 | Linux构建与基础源码门禁 | g1.5的147输入/14命令及271项核对；相关真实FUSE/feature/lint/build | **完成** |
| G1.02 | 可安装、可复现试用包 | g1.5离线无源码/编译器安装、包校验、不同umask字节一致 | **完成** |
| G1.03 | memory基础演示 | g1.5 memory R1、普通用户双mount操作及易失健康声明；新memory R2未跑且不在本出口 | **完成** |
| G1.04 | OwnerFs本地基本操作 | G1/g1.5：64MiB、覆盖、sync/close/reopen/EOF、基本命名空间/权限/锁/mmap | **完成** |
| G1.05 | OwnerFs远端基本操作 | G1跨节点及g1.5同ELF local-file Home/remote写入和精确读回/恢复 | **完成** |
| G1.06 | DFS本地/跨节点基本操作 | G1/g1.5 R1/R2核心文件与A/B读回，local-file R2物理内容/复制记录 | **完成** |
| G1.07 | 中心local-file Meta重启恢复 | g1.5同ELF Meta重启，确认路径/数据/结果保留，可继续写读 | **完成** |
| G1.08 | 同事自检与正常生命周期 | g1.5使用说明、普通用户自检、有序启停、重装/卸载保留状态 | **完成** |

**决策：** G1保持8/8完成。历史标准集证据单列下方；当前优化候选标准回归/核心恢复是G2.04–08，不以未运行的新回归否定已交付G1。G1跨节点读回不等于DFS并发一写多读性能。

## 阶段二 G2：独立标准回归与核心性能

**决策：** 标准集兜底、自定义case补充：pjdfstest完整适用项、LTP固定基础文件/权限/锁子集、FSx短固定种子分别登记；选择/版本/排除项测前固定，不因失败删适用项。专项补中心Meta恢复、跨节点可见性/多读者及安装生命周期。广LTP、长时FSx/差分序列后置。

**决策：** 同接口/数据/并发/缓存/屏障/适用副本语义/资源比较。普通OwnerFs local/remote读写统一按[新准则](ownerfs-performance-criteria.md)：吞吐>=同条件MooseFS的1.2倍，操作时延<=同条件MooseFS的0.8倍，两项分别测量且同时满足；p50/p95/p99都记录，吞吐默认用配对中位数判定，时延默认用测前声明的逐操作样本p95判定，不能从吞吐反推时延。DFS同FUSE/POSIX、三份同步durable条件与3FS持平；bind接近native ext4。测量噪声容差、计时边界、计时器、样本数、分位数算法和判定分位数逐case开跑前固定，不按成绩调值、不追1%–2%残差。删除先要求正确性及操作数/秒、耗时/延迟对照报告，没有新增硬比例。

**决策：** 小规模默认64MiB，512MiB扩展与8GiB大case各自独立记录；必要时先1–8MiB明确标注诊断。先C1再多并发。DFS一写多读先一写节点+两个读取视图，再扩读者/节点；记录读者分布、总吞吐及逐读者表现。先校验后统计；无驻留实证仅标buffered/repeat，不宣称hot。完整36组/432样本蓝图和高级冷热/长时矩阵不作为所有小case前置。

| ID | 独立验收项 | 本项出口 | 当前状态 |
| --- | --- | --- | --- |
| G2.01 | OwnerFs内部优化 | 属性/identity、别名/目录索引等冻结切片的算法评价/正确性通过 | **完成（限定成果）**；[B1](current-checkpoint.md#historical-evidence)、[B2](current-checkpoint.md#historical-evidence)、[B1–B4索引](../docs/status.md)；非ext4达标 |
| G2.02 | DFS批次读放大优化 | 同批次共享完整校验，不掩盖错误，Linux/成对诊断/独审通过 | **完成（限定成果）**；[48次诊断/1766项审计](current-checkpoint.md#historical-evidence)；非3FS达标 |
| G2.03 | 小尺度性能工具资格 | ext4正向/错误注入、返回/内容/生命周期校验 | **完成（有限工具）**；[15tests/24正向/46负控制](current-checkpoint.md#historical-evidence)；非完整正式评估器 |
| G2.04 | 当前候选OwnerFs pjdfstest | 完整适用项、TAP/排除/版本/挂载身份可复核 | **完成（历史e925 Owner本地；6d OFF限定复用）**；[逐路径等价/受影响守卫审计](evidence/20261007-owner-standard-reuse/README.md)，非6d标准实跑；e925c5b dev及release各236文件/8819checks/28TODO，0skip/意外失败；[release](evidence/20261006-e2e-current/release-r1/README.md)；[proof](evidence/20261006-e2e-current/r2/owner-pjdfstest-r2/artifacts/std-01-pjdfstest/proof.json) |
| G2.05 | 当前候选DFS pjdfstest | 独立mount/state、完整适用项与逐项结果 | **完成（历史DFS本地R1；当前限定复用）**；0891cbfe/map6161e25b release：236文件/8819checks/28TODO，0skip/意外失败，正常关闭；[新proof](evidence/20261007-dfs-statfs/runtime-r2/std-01-pjdfstest-full/artifacts/std-01-pjdfstest/proof.json)。e925 ENOSYS/0TAP[原失败](evidence/20261006-e2e-current/r5/README.md)保留，非remote/RN标准资格；[6d差异审计/限定复用](evidence/20261007-standard-reuse/README.md)，非6d实跑 |
| G2.06 | 当前候选LTP基础子集 | 固定基础文件/权限/锁清单，完整结果账本 | **完成（历史固定6项，版本分列；DFS限定复用）**；Owner e925 dev固定6/6[限定复用审计](evidence/20261007-owner-standard-reuse/README.md)，未称6d/release实跑；DFS map6161e25b release固定6/6PASS，0TBROK/TCONF/FAIL/TIMEOUT；651库存未选；[DFS新proof](evidence/20261007-dfs-statfs/runtime-r2/std-02-ltp-smoke6/artifacts/std-02-ltp/proof.json)。旧6TBROK保留；不宣称新ELF Owner已实跑；[DFS限定复用](evidence/20261007-standard-reuse/README.md) |
| G2.07 | 当前候选FSx短测试 | 固定种子/时长/文件上限，无内容/长度mismatch | **历史通过（短范围，限定复用）**；[Owner6d OFF复用条件](evidence/20261007-owner-standard-reuse/README.md)；e925c5b Owner/DFS均seed1/请求1000、默认262144字节上限、无mismatch；[Owner](evidence/20261006-e2e-current/r2/owner-fsx-r1/artifacts/std-03-fsx/proof.json)/[DFS](evidence/20261006-e2e-current/r5/dfs-fsx-r1/artifacts/std-03-fsx/proof.json) |
| G2.08 | 当前候选基本组合/local-file恢复 | 受改动影响的Home/remote/DFS核心操作、中心重启、错误及正常关闭 | **完成（各自限定版本）**；[7e6普通新包OFF单节点Owner/DFS64MiB+Meta正常恢复/三wait0 PASS](evidence/20261007-current-trial-7e6/README.md)；跨节点/R3仍按原版本，不继承；e925c5b Owner双VM A写B读/B改A读、rename/delete及中心有序恢复；DFS R2一写两读/中心恢复，正常stop/卸载；[34项+关闭核验](evidence/20261006-e2e-current/crossvm-r1/README.md)。非Node崩溃矩阵，不重开G1 |
| G2.09 | OwnerFs本地小规模读 | 与同条件MooseFS比较，内容/EOF正确，吞吐>=1.2×且独立操作时延<=0.8×，测前固定p95 | **正式待验收**；e925旧ext4 FAIL保留原判据。[当前f03证据](evidence/20261008-owner-local-read-latency/README.md)：正式hot在计时前因0驻留页失败、0配对；另行冻结repeat诊断中位5937.148/18687.490MiB/s、合并p95 206000/54542ns，各320原始样本。缓存0/64MiB不同，仅摸底；缺同缓存正式比较，后续专项。 |
| G2.10 | OwnerFs本地小规模写 | 与同条件MooseFS比较，读回正确，吞吐>=1.2×且独立操作时延<=0.8×，测前固定p95 | **正式待验收；当前功能/诊断数据完成**。[f03新建+fdatasync证据](evidence/20261008-owner-local-write-latency/README.md)：12×64MiB读回/EOF/权限通过，Owner/Moose中位909.994/1170.143MiB/s、合并p95 1184309/1374228ns；各320原始样本，仅摸底，缺强durable-ACK/缓存比较资格。e925旧ext4 FAIL保留原版本/判据/结论。 |
| G2.11 | OwnerFs本地删除 | 固定小文件集合，删除正确，操作性能对照报告 | **历史e925完成（限定小项），不重开**；100×4KiB/C1/5对正确性及ext4报告保留；[当前复用审计](evidence/20261008-owner-local-delete-reuse/README.md)支持普通OFF功能限定复用，f03安装仅两次基本unlink已过；不称当前100文件重测，不迁移旧计时。 |
| G2.12 | OwnerFs workspace bind mount功能验收（独立开关） | 显式可配置、默认OFF；OFF原FUSE回归；ON受管挂载/启动/停止、必要语义/权限、引用排空及重启对账；不安全配置拒绝 | **进行中**；[本地授权失效后的普通FD/mmap引用排空组件PASS，非通用撤权/full ON](evidence/20261008-ownerfs-bind-reference-drain/README.md)；[独立宿主开关源码/组件PASS；931真实Node固定Home宿主运行PASS](evidence/20261007-ownerfs-workspace-host-runtime/README.md)；[当前main Node accepted限定回归PASS](evidence/20261007-ownerfs-bind-node-accepted/README.md)；[交接](current-checkpoint.md#historical-evidence)、[基础资格](current-checkpoint.md#historical-evidence)；[管理员实验接线](native-workspace-slice.md)，默认OFF；[实际受管单容器生命周期/清理PASS](evidence/20261007-managed-workspace/README.md)；[短语义](evidence/20261007-managed-semantics/README.md)：mmap字节/权限PASS，锁/append偏移/watch传播FAIL；[append r8独立数据](evidence/20261007-append-diagnostic/README.md)顺序/128记录PASS，偏移仍FAIL，本轮诊断收口；[经典锁原语/机制边界](native-classic-lock-boundary.md)收口、产品仍FAIL；[活动容器source注入拒绝](evidence/20261007-native-source-rejection/README.md)新有限PASS：真实identity/artifacts不变、20B合法exec及收尾；[活动控制额度耗尽](evidence/20261007-native-control-capacity/README.md)新有限PASS：63busy/64ledger、ENOSPC无副作用、Status及Stop收尾；[有序恢复窄修复](evidence/20261007-native-orderly-recovery/README.md)原ENOSPC及草稿身份保留；[3cc新包4KiB两阶段有序恢复](evidence/20261007-native-orderly-recovery-runtime/README.md)限定通过；独立高优先级[命名/模块归属整改](evidence/20261007-ownerfs-bind-remediation/README.md)已完成限定收口，固定Home宿主可见性/有序关闭已过，通用bind生命周期仍待功能验收；完整ON及生产开关未资格化 |
| G2.13 | OwnerFs workspace bind mount性能验收（独立开关） | 同候选OFF/ON/ext4配对；核心数据读写和元数据接近ext4，内容/语义正确 | **完成（限定当前7e6/C1的8核心case）**；[当前64MiB写0.969905/读0.965910及组合8项证据](evidence/20261007-workspace-bind-data-current/README.md)，189/258 checks；[同候选六元数据0.964347–1.032311](evidence/20261007-workspace-bind-metadata-window/README.md)，190/193 checks。固定>=0.90，OFF失败数据/原931窗口FAIL及历史计时保留。缓存/物理耐久未资格化；G2.12 full ON、全部PR43组合及大规模/并发等另项，不等于生产开关完成 |
| G2.14 | OwnerFs远端小规模读 | 同Home/缓存/接口MooseFS对照，内容正确，吞吐>=1.2×且操作时延<=0.8× | **新判据待验收；小数据已留**；6d B-Home/C1/64MiB/1预热5配对，内容PASS，427.371/14854.399MiB/s、配对比0.028803；缓存未观察/旧负载限制、Moose客户端wait1清理FAIL、缺测前固定时延分位数，不计达标。[证据](evidence/20261007-owner-remote-small/README.md) 历史逐操作p50/p95/p99已有；缺原始数组/预定判定分位数，仍待正式验收。[来源审计](evidence/20261008-owner-remote-latency-reuse/README.md) |
| G2.15 | OwnerFs远端小规模写 | 同持久屏障/数据量，跨节点读回正确，吞吐>=1.2×且操作时延<=0.8× | **新判据待验收；当前6d小功能/清理PASS、数据已留**；A远端/B-Home，64MiB/1预热5配对，12内容校验+B6fresh全量读、6正常退出PASS；234.630/430.767MiB/s，配对比0.528615。缓存未观察/波动、Moose强持久ACK基线独立BLOCKED、缺测前固定时延分位数，不计达标。[当前证据](evidence/20261007-owner-remote-write-small/README.md) 历史逐操作p50/p95/p99已有；缺原始数组/预定判定分位数，仍待正式验收。[来源审计](evidence/20261008-owner-remote-latency-reuse/README.md) |
| G2.16 | OwnerFs远端删除 | 跨挂载可见性正确，操作性能对照报告 | **完成（当前限定小项）**；6d/100×4KiB/1预热5配对，12sample PASS、B600路径ENOENT、6实际wait0；Owner1049.602/Moose1542.929ops/s、配对比0.680266，无硬比例。旧wait1 FAIL保留。[完整证据](evidence/20261007-owner-remote-delete-small/README.md) |
| G2.17 | OwnerFs本地大规模读 | 8GiB顺序核心case，内容正确，吞吐>=1.2×MooseFS且操作时延<=0.8×MooseFS | **待验收** |
| G2.18 | OwnerFs本地大规模写 | 8GiB同持久屏障，读回正确，吞吐>=1.2×MooseFS且操作时延<=0.8×MooseFS | **待验收** |
| G2.19 | OwnerFs远端大规模读 | 8GiB同条件MooseFS对照，正确，吞吐>=1.2×且操作时延<=0.8× | **待验收** |
| G2.20 | OwnerFs远端大规模写 | 8GiB同持久语义MooseFS对照，正确，吞吐>=1.2×且操作时延<=0.8× | **待验收** |
| G2.21 | DFS小规模一写多读 | 一写确认、多读者相同数据，逐读者/总吞吐与3FS对照 | **931 R3非重复64MiB三副本小功能/数据PASS，3FS正式性能待验收；DFS最高优先级**；[R3证据](evidence/20261007-dfs-r3-small/README.md)：16不同chunk×3物理份，B/C1预热5读，共同中位98.052181MiB/s；[最新7e6/c3bb内容/48副本及正常Meta恢复](evidence/20261007-dfs-r3-current-recovery/README.md)已过，未继承931五轮计时。以下6d为历史范围：64MiB一写/B+C各1预热5读，前后全SHA/EOF、4AFS wait0/3stdio rc0；同一ctl公共窗口中位45.149、B/C纯C50.326/22.716MiB/s。实际A+B durable、uniform去重/缓存未观察，不计三同步/3FS。[同步证据](evidence/20261007-dfs-sync-read-small/README.md)；[旧含启动/预检/预热父窗口](evidence/20261007-dfs-manyread-small/README.md)保持原范围，非改善对比；[历史e925功能/核心恢复](evidence/20261006-e2e-current/crossvm-r1/README.md)保留原版本 |
| G2.22 | DFS单节点小规模读 | 同副本/接口/缓存与3FS对照，正确并持平 | **当前7e6小功能/计时已留，正式3FS对照待验收**；[A单读者64MiB/1预热5读](evidence/20261007-dfs-r3-local-read/README.md)，中位65.624896MiB/s，前后48物理副本/四wait0；缓存/RPC位置未观察，首写前工具拒绝保留，不计持平 |
| G2.23 | DFS单节点小规模写 | 三份同步durable，屏障/读回正确并持平 | **当前7e6小写数据完成，正式3FS对照待验收**；[六个64MiB新文件/独立generation数据](evidence/20261007-dfs-r3-write/README.md)：A/C1，1预热5写中位81.930440MiB/s，96不同4MiB chunk，每轮48物理份及B/C完整SHA/EOF；四actualwait0/八PID消失与保护库存不变。新case测前2GiB总预算、原per-role1GiB保留；不是3FS持平或断电耐久 |
| G2.24 | DFS多节点读写 | 固定文件/并发，读写分别验收，不用平均数掩盖失败 | **当前f03小功能/计时数据完成，正式性能待验收**；[新候选证据](evidence/20261007-dfs-r3-multinode-current/README.md)：local-file/R3/gRPC，三写者各64MiB、1预热1测量及两跨节点读路由、六文件/288物理份，四wait0/保护库存闭合。单测量窗口写103.498/读142.149与143.988MiB/s，仅摸底；缓存/正式3FS/时延不升级。旧7e6首预热FAIL和零有效测量不改：[原容量阻塞](evidence/20261007-dfs-r3-multinode-preparation/README.md)、[原运行失败](evidence/20261007-dfs-r3-multinode-runtime/README.md)；[create源码修复](evidence/20261007-dfs-create-contention/README.md)、旧观察器标签FAIL及R2只读复核均保留 |
| G2.25 | DFS删除 | 固定文件集合，删除正确，操作性能对照报告 | **当前f03小功能/计时完成，正式对照待验收**；[同当前候选600删除/B+C各600ENOENT/正常闭合](evidence/20261007-dfs-delete-current/README.md)，中位27.003911/pooled27.927077ops/s，配置R3，仅摸底；旧6d/R2身份保持：29.861/30.682ops/s，[原候选证据](evidence/20261007-dfs-delete-small/README.md)；patched3FS R2一次615.210/509.565、B/C各600ENOENT通过；A超预算4,067,328B/FDB-15使原资源/关闭资格FAIL，18owned已消失，不调判据/不重测。[完整新数据及失败](evidence/20261007-threefs-delete-small/README.md)。非3FS持平/物理回收/复杂故障 |
| G2.26 | DFS大规模一写多读 | 512MiB/8GiB分别留结果；读者正确、总/单读者与3FS对照 | **待验收**；各规模独立验收 |
| G2.27 | 核心性能版本交付 | 同候选已选核心case/必要组合回归、可复现包、独立安装/恢复及完整状态报告 | **待验收（当前f03普通安装/恢复/固定发布分项已过）**；[f03两次复现/新安装/正常恢复](evidence/20261008-current-trial-f03/README.md)，与同f03已运行包的完整运行成员一致，历史证据不重标；[7e6当前普通包两次复现/无源码无编译器安装/Owner+DFS64MiB恢复PASS](evidence/20261007-current-trial-7e6/README.md)，固定prerelease四附件远端SHA核对通过；[6d OFF安装/恢复](evidence/20261007-installed-off-6d/README.md)和[同包复现/试用清单](evidence/20261007-off-trial-handoff/README.md)分项已过，固定prerelease已发布并核对；完整选定性能/组合出口仍待验收，native保持OFF，不称G2全表完成 |

**决策：** native已知append、实际kernel锁、混合mmap/watch、最终namespace/Root/epoch及排空缺口属于G2.12启用前条件，不能延期后冒充通过。OFF版本和普通FUSE性能独立推进；开关不掩盖ON错误，当前不声称已有可用生产开关。

**当前独立出口（2026-10-07）：** 普通OwnerFs性能目标已更新；G2.09/10/14/15按新MooseFS吞吐/时延双判据待验收，旧ext4/持平数据保留原版本和原结论。G2.16限定完成。G2.25两边小删除功能/量化已留数据，3FS A预算与FDB退出判据FAIL原样保留，不关闭正式项。按用户选择，3FS基线资格留专题、普通性能调优后置；主线回OwnerFs workspace bind必要语义。OFF限定试用下载/安装恢复/复现已过，G1 8/8不变。

**前序顺序（范围保留，当前动作以上述出口为准）：** 当前已启动release切片已测完并正常停止，普通读写旧判据未达标和远端摸底数据保留、专项优化暂缓；R1仓库整改验证完成并发布，R2原版fuser公开API缺口独立阻塞。DFS本地R1受影响标准回归已完成；其它标准项按功能范围复用或补齐，不为了性能反复跑标准集。性能优先G2.12 OwnerFs workspace bind mount的必要功能/安全出口→G2.13性能。其它核心性能按独立摸底项保留数据，不无限优化；大规模/复杂可靠性/后端仍后置。

## 阶段三 G3：复杂可靠性及最后的后端

| ID | 独立验收项 | 出口 | 当前状态 |
| --- | --- | --- | --- |
| G3.01 | 8小时长时间运行 | 持续负载正确、资源有界，无卡死/错误成功 | **后置**；未整体验收 |
| G3.02 | 广LTP/POSIX矩阵 | 更广syscalls/权限/锁及适用项完整账本 | **后置**；保留历史单项/标准结果 |
| G3.03 | FSx长时/差分随机 | 原长时固定种子FSx、10×10000差分及缩减反例 | **后置**；ext4参考有证据，最终候选未完整验收 |
| G3.04 | 节点异常退出/恢复 | Home/remote/DFS分别检查确认水位及恢复 | **后置**；有代表性切片 |
| G3.05 | 网络/未知提交故障 | 断网/超时/ACK丢失/重连不错误成功/重复提交/损坏 | **后置**；有D18等局部闭合证据 |
| G3.06 | 磁盘故障/容量/损坏修复 | ENOSPC/EIO/坏副本/修复及错误可见逐case通过 | **后置**；有EIO/修复切片，D20仅local闭合 |
| G3.07 | 复杂并发/native扩展 | append/rename/撤权/mmap/多Agent的复杂组合与长时 | **后置**；不代替G2.12必要安全条件 |
| G3.08 | RDMA异常/资源生命周期 | CQ超时、迟到完成、引用/设备拆除不越权/损坏/泄漏 | **后置**；有posted deadline等限定证明 |
| G3.09 | 复杂性能矩阵 | 扩展随机/并发、专用cold/真实hot等完整原矩阵 | **后置**；不将无驻留证据repeat升级hot |
| G3.10 | 多Meta/HA | 权威切换、恢复及一致性 | **后置** |
| G3.11 | 更广部署与最终组合 | 剩余安装/升级/内核/拓扑等适用矩阵及完整账本 | **后置**；正式69 NOT_RUN/ENV PREPARING，不是G1进度分母 |
| G3.12 | etcd后端 | 功能/重启/故障/资源，D17专题暂2GiB | **后置，倒数第二**；已有局部恢复/失败 |
| G3.13 | Redis后端 | 持久配置、功能/恢复/故障/资源一致性 | **最后/TODO**；已有实现/局部证据 |

## 已完成历史证明和结案（不重复计入当前候选PASS）

| 成果 | 已完成内容 | 边界/证据 |
| --- | --- | --- |
| pjdfstest全集 | v37 OwnerFs/DFS各236文件/8819 TAP checks；v48远端DFS同全集；无意外失败/skip、28上游TODO登记 | [旧版本状态](../docs/status.md)；不是g1.5/当前候选全集 |
| ext4标准参考 | pjdfstest/FSx、预冻结657命令LTP及差分参考已有通过 | [参考状态](../docs/status.md)；不是AFS标准全集通过 |
| D20本地容量语义 | statvfs/stat/df、权限/错误、Linux门禁/独审 | [本地切片](current-checkpoint.md#historical-evidence)；不是整个D20/测试盘容量已足 |
| native基础 | N2a身份/权限、N2b1服务端、N2c PF1私有引用分别资格化 | [N2a](current-checkpoint.md#historical-evidence)、[N2b1](current-checkpoint.md#historical-evidence)、[PF1](current-checkpoint.md#historical-evidence)；非生产ON/READY/全排空 |
| parent FD收益实验 | 48窗口/20对、正常关闭和失败审查结案 | [结果](current-checkpoint.md#historical-evidence)：未达原≥5%改善，非优化PASS，无新输入不重跑 |

## 当前边界

**事实/决策：** G1完成，完整G2仍ACTIVE；本次将当前代码、三阶段表和相关文档共同上库；新Linux源码/工具验证见current-checkpoint.md，G2标准/性能/安装后恢复仍按独立项验收。后续编译/运行/验证仅ARM64 Linux；本次按用户明确授权同步刷新跨电脑handoff。旧结果/失败/包/检查点不改写。

**决策：** 容量按case实际守卫；基线/候选顺序运行、结果校验/关闭后复用自有空间；同时保留两份8GiB才至少20GiB加实测开销。日志先量化，A扩盘保护活动memory Meta/FDB状态并停机；薄置备不是host物理空闲。见[容量分级](current-checkpoint.md#capacity)、[NEXT](current-checkpoint.md)、[CURRENT](current-checkpoint.md)。
