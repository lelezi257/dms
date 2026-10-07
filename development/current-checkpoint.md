**2026-10-07 当前事实：** 用户授权后，A已停止旧DFS目录完整归档并经Linux229条恢复核验，释放500,518,912B（477.332MiB）；四角色沿原判据重新准入。G2.24首个预热写FAIL：A/C create元数据条件冲突，B探针后检查ENOTCONN；中继BrokenPipe掩盖部分失败记录，缺口已明示。零有效测量/零读轮次，未重试；四actualwait0/八PID消失、11保护进程和完整mount库存不变。G1历史8/8关闭、G2计数不变。下一独立小项为确定触发create竞争的回归/有界恢复，以及中继失败留证/排空；不扩大矩阵。 [新增证据](evidence/20261007-dfs-r3-multinode-runtime/README.md)。以下保留原时点记录。

**2026-10-07 current G2.24 preparation:** 9 exact Linux driver guards PASS; product runtime BLOCKED before start by A's existing capacity prerequisite (short148.492MiB). Zero data rounds; no performance/3FS verdict. No VM repair/budget relaxation; all4 mount inventories/11 protected identities unchanged. [Evidence](evidence/20261007-dfs-r3-multinode-preparation/README.md). G1历史8/8关闭，G2计数和普通Owner1.2/.8双判据不变；等待容量处理，独立文档收口继续。以下为原时点记录。

**新增小项（2026-10-07，事实）：G2.23当前7e6三同步副本64MiB小写数据完成，正式3FS对照仍待验收。** A/C1/六个不同generation新文件，1预热5计时，中位81.930440MiB/s；96不同4MiB chunks，每轮48物理份及B/C新开全SHA/EOF，四actualwait0/八PID消失及11保护进程/完整mount库存不变。单次产品运行，Linux8 C+7 driver+6 observer guards通过；初始错误文案断言/准备status假设失败留证。测前新case2GiB总预算，最终1,615,421,440B；日志23,261B的39ERRO/60WARN完整保留，不称零错误/完整POSIX。无Rust/vendor/ELF变化，不继承历史性能或升级3FS。G1历史8/8、G2新判据11限定完成/1bind功能进行中/15待验收不变；普通Owner仍1.2×MooseFS吞吐/.8×独立时延待验。下一G2.24小规模多节点读写。[证据](evidence/20261007-dfs-r3-write/README.md)。以下保留原时点记录。

**当前覆盖说明（2026-10-07，决策）：** 普通OwnerFs性能目标后续按[普通OwnerFs性能准则](ownerfs-performance-criteria.md)执行：吞吐>=同条件MooseFS的1.2倍，操作时延<=同条件MooseFS的0.8倍；吞吐默认按配对中位数判定，时延默认按测前声明的逐操作样本p95判定。下方日期快照原判据、原比例、原FAIL和当时计数保持历史身份，不追溯改写。

**2026-10-07新增事实：G2.22当前7e6/c3bb的A单读者64MiB小项完成内容/计时/正常闭合，正式3FS对照仍待验。** 1预热5读，中位65.624896MiB/s；前后48物理副本，四actualwait0/八PID消失及11保护进程/完整mount库存不变。Linux11工具guards及独立观察校验通过；首轮遗漏结果目录导致写前拒绝，原FAIL及四正常退出保留，修测试准备后一次数据运行，无Rust/vendor/VM修补。G1历史8/8、G2大项计数/defaultOFF不变，G2.23写性能/G2.27/full bind仍开放。当前OFF标准限定复用已由既有impact-map及当前安装恢复覆盖，不重跑整套；下一G2.23三同步副本小写入摸底。 [证据](evidence/20261007-dfs-r3-local-read/README.md)。

以下保留原时点记录。

**当前独立出口（2026-10-07，事实）：G2.13完成，限定当前7e6/C1的8个小规模核心case。** 同公开c3bb包/map151a，新增64MiB写0.969905/读0.965910×ext4，复用同候选六元数据0.964347–1.032311；全部>=测前0.90。189驱动/258 Linux独立checks及15受影响工具guards通过，四actualwait0/十二服务监督及OCI PID与正常mount闭合，一保护进程/26 mount完整库存不变，峰值245153792B<256MiB。OFF写0.728219/读0.457996 FAIL保留暂缓，不重测刷分；无Rust/vendor/C修改或重建。G1历史8/8不变；G2变为11限定完成/2普通性能FAIL/1bind功能进行中/13待验收。G2.12 full ON/复杂语义、全部PR43组合/冷热耐久/大规模/正式MooseFS及3FS资格不升级，默认OFF、Goal ACTIVE。下一当前7e6 OFF标准适用性/限定复用审计，仅真实受影响缺口才补测，再更新核心性能交付状态。 [版本、原始数据及8项组合账本](evidence/20261007-workspace-bind-data-current/README.md)。

以下保留原时点记录；当前入口以上方为准。

**新增当前DFS恢复小项（2026-10-07，事实）：** 产品7e6/map151a/公开c3bb普通包未变，64MiB非重复一写两读、重启前后各48物理chunk及三份Ready/Durable副本通过；Meta仅一次正常重启，三Node/FUSE/UDS身份不变。五actualwait0/十所属PID消失、三mount闭合及11保护进程/完整mount库存不变；峰值597,450,752B<1GiB。首观察器错误要求旧生命周期目录保留，原FAIL和重启前保存的退出回执不改，R2只读复核及12 guards通过；无产品重跑/环境修补。7+14 Linux工具guards通过，无Rust/vendor改动；不继承931计时或称3FS/fullG2 PASS，G1历史8/8与大项计数/defaultOFF不变。下一当前7e6 workspace64MiB读写配对，普通FAIL及复杂/后端专题后置。 [版本、原始证据与范围](evidence/20261007-dfs-r3-current-recovery/README.md)。

**当前元数据性能小项（2026-10-07，事实）：** 产品7e6e00a6/map151a/当前c3bb包，唯一工具修正为将容量遍历移到callback结束快照之后。旧工具Linux先1FAIL复现，修后7guards PASS；原931 FAIL保留。当前1000×4KiB/C1/六阶段OFF+ON各1预热5配对完成，ON中位0.964347–1.032311×ext4、六项>=.90 PASS，OFF六项FAIL留数；ON八个选定回调0、其它getattr24不冒称全0。190驱动/193独立检查PASS，四actualwait0/十二服务监督及OCI PID与mount闭合、保护库存/预算不变。无Rust/vendor/C修改或重建。不是完整ON/全G2.13/正式比较；G1历史8/8、大项计数/defaultOFF不变。下一当前7e6 DFS R3小一写两读内容/副本/正常生命周期回归，历史931五轮计时保留原身份不刷分。[版本、原始数据、失败及退出回执](evidence/20261007-workspace-bind-metadata-window/README.md)。

以下保留原时点记录；旧下一动作由上方当前入口覆盖。

**当前普通试用交付分项（2026-10-07，事实）：** 产品7e6e00a6/157-map151a2c6d，既有Linux release ELF两次打包字节一致；新普通包c3bb5a30不含测试探针。隔离Linux/ext4无编译器一次安装，OwnerFs+DFS各64MiB/R1/gRPC/local-file、两workspace开关OFF，43驱动检查及独立恢复/退出/保护库存检查PASS；Meta-only正常重启后完整SHA/EOF读回，三actualwait0/六服务监督PID消失、Node及两mount跨重启身份不变。15工具检查按受影响范围通过/复用，无Rust/vendor改动或重建。G1历史8/8不重开、G2大项计数不变；931 R3/旧标准与性能保留原身份，当前7e6 R3/全POSIX/完整G2.27性能及ON仍待验。固定[试用包](https://github.com/lelezi257/dms/releases/tag/afs-trial-7e6e00a)已发布，四附件远端SHA及main实际树已核对。下一回既定workspace元数据计数归因小项，普通性能FAIL与复杂可靠性后置。[当前版本、命令、结果及历史复用边界](evidence/20261007-current-trial-7e6/README.md)。

以下保留原时点记录；旧下一动作由上方当前入口覆盖。

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

**最终修复受影响检查（2026-10-07）：** 磁盘已由已归档历史副本移出释放37.64GiB；固定Rust8c6f1d50/157-map e15c，Linux release四门禁通过，普通10PASS/7忽略及root实际7PASS，test ELF SHA884ba8f9。仅此范围，不称新服务ELF/包/重启PASS。下一必要剩余源码检查、构建、新包及4KiB有序恢复；不重跑旧标准/性能，G1/G2计数和defaultOFF不变。

[本次版本、原始命令与完整结果](evidence/20261007-native-orderly-recovery-source/README.md)。

**VM归档移出完成（2026-10-07）：** 用户授权后只移除已完整校验的三历史目录，实际释放37.64GiB，磁盘100%→56%。主机9.49GiB归档保留、源端预移出清单及归档SHA复核一致；保护目录身份/旧ELF SHA不变，未扩盘/清当前缓存/动基线或工具链。磁盘阻塞解除，正在执行原计划最终修复的受影响Linux release检查；新包/实际重启尚未验收。

**构建VM历史归档（2026-10-07）：** 旧evidence/artifacts/logs已流式复制到主机，9.49GiB归档、11,852条目/10,661文件内容及基本元数据一致，源端前后无变化；VM原件保留，尚未释放空间，移除决定待用户答复。当前构建缓存/基线/工具链未动，最终修复验证仍阻塞。[归档回执](evidence/20261007-vm-archive/README.md)。

**Owner标准复用审计（2026-10-07）：** e925→已交付6d OFF普通操作未变，构造/statvfs等价变化及既有守卫支持历史限定复用；pjdf有e925 release，LTP6/短FSx仅dev。当前安装/套件只读身份匹配，未重跑标准、不增加PASS或计数。[审计与原proof索引](evidence/20261007-owner-standard-reuse/README.md)。afs-build满盘仍停分支等待用户决定，草稿验证/新包/实际重启尚未完成。

**当前状态（2026-10-07）：** 3FS基线资格按用户决定留专题，主线容器workspace。main f09185e、产品6d/157/map66/既有ELF包的通过范围保留；新rootfs私有副本/命令续号修复在独立分支fix/native-orderly-recovery-20261007，不视为新可用候选。G1历史8/8，G2为10限定完成/2性能FAIL/2bind进行中/13待验收，bind默认OFF。

**新增/阻塞：** 工具11个独立Linux guards通过；早期Rust10+4有输出但未冻结源码SHA，不计最终版本PASS。静态审阅发现并最小修正非root测试入口，原FAIL保留。afs-build85GiB根盘实际满：误用debug与root工具链下载、后续ENOSPC，已停止构建并求助；最终续号/源码门禁/新包/重启读回均未完成。[版本、失败与证据](evidence/20261007-native-orderly-recovery/README.md)。

**下一项：** 等环境处理决定后一次准入，复用release缓存、普通用户构建、仅真实test binary特权运行；新包后只做4KiB两阶段正常重启读回，不重跑标准/性能或3FS。独立文本/审查与证据收口不受该阻塞。

**历史说明：** 下方旧检查点按原版本/范围保存；其旧下一动作由上方当前入口和验收主表覆盖。当前native偏移/经典锁/watch失败与普通性能失败原始记录均保留。

# 2026-10-06 代码与目标检查点

**决策：** 本次按用户授权，将当前累积产品代码和相关文档一起版本化、发布GitHub。该检查点的Git提交同时标识代码、目标、状态、操作说明和交接；从本仓根执行 `git rev-parse HEAD` 获取当前检出身份。任务进度唯一来源是[三阶段验收表](trial-release-goals.md)。

## 当前范围

**事实：** 已交付g1.5试用包的G1为8/8完成。当前代码包含此后Owner索引/目录维护、DFS同批次完整校验共享、local-file恢复、写权限/未知结果处理、健康/容量语义、安装自检及native私有基础。当前源码与历史g1.5包不是同一输入，不用旧包的通过结果替代本检查点验证。

**旧e925/2b5d35c续跑证据（保持原版本）：** [当前E2E账本](evidence/20261006-e2e-current/README.md)记录Owner完整pjdfstest、Owner固定LTP、Owner/DFS短FSx通过；小读/写功能PASS但性能FAIL（0.5709/0.6297×ext4），小删除完成报告出口。DFS pjdfstest/LTP因statfs ENOSYS未进入断言，不计通过。G1保持8/8；G2逐行状态见[主表](trial-release-goals.md)，[双VM核心/中心恢复及DFS一写两读功能](evidence/20261006-e2e-current/crossvm-r1/README.md)已通过；正式比较/新性能包仍未完成，G3后置。旧发布回执原样保留；本轮文档/证据与R1整改一起版本化，产品154编译输入未改。

**决策：** OwnerFs优先；DFS首先一写多读。先小规模读/写/删除，再扩规模/并发/时间；按case自身依赖推进。Owner local吞吐≥90%ext4，remote与MooseFS持平，DFS在同FUSE/POSIX及三同步durable副本下与3FS持平；噪声容差测前固定。普通使用中的损坏、错误成功、权限绕过和核心恢复错误及时修复。

**同源release补充：** [独立结果](evidence/20261006-e2e-current/release-r1/README.md)：122秒构建，Owner完整pjdfstest及64MiB基础/单机中心有序恢复PASS；小读/写仍性能FAIL0.3477/0.5879，保留原dev失败。release DFS/双VM/LTP/FSx尚未新跑。最新用户性能优先容器workspace挂载访问，普通性能专项暂缓；[R1整改](repository-remediation.md)恢复/夹具验证已过，R2上游API缺口单列阻塞，不修改历史结论。

## 当前默认OFF安装回归

main25a8061/map8ef8b788的release ELF已生成两份逐字节一致的包。无编译器Linux/ext4 VM独立安装后，Owner/DFS各64MiB基础校验、目录fsync、中心Meta有序重启/全内容读回、托管退出0/两个exact mount移除通过。[证据和包摘要](evidence/20261007-installed-off/README.md)、[维护驱动及复现](installed-off-slice.md)。第一轮有限身份结果保留，复核加强安装文件inode/路径与mount ID后仅回归受影响小项。没有重跑标准全集或性能，不重开G1，不关闭G2.27及容器G2.12/13；用户已授权隔离VM安装官方runc，运行时准入已通过；实际OwnerFs容器功能仍待验收。

## 当前组合验证

[Linux验证入口](checkpoints/20261006-current/validate-linux.sh)及[输入绑定器](checkpoints/20261006-current/verify-inputs.py)随代码保存。只能在ARM64 Linux运行，Cargo输出与验证结果放源码树外；命令使用锁定依赖和offline模式，依赖需先按[构建说明](validation.md)准备。

```sh
export CARGO_TARGET_DIR=/path/on/linux-ext4/afs-target
bash development/checkpoints/20261006-current/validate-linux.sh /path/on/linux-ext4/new-checkpoint-results
```

当前执行的精确输入、命令、输出和返回值在[本次结果目录](checkpoints/20261006-current/results/README.md)。文档不计入编译输入摘要；验收驱动、安装脚本及本验证工具单独纳入同一输入清单。结果目录不计入工具输入，避免自引用。没有新跑正式性能、标准集或安装后的多节点恢复，不将源码门禁称为全部产品验收。

## historical-evidence

**事实（保留原版本范围）：** 以下小型结构化结果从研究归档按原字节复制，摘要一致，随GitHub可读取。它们不是本次候选的新测量；完整大ELF、磁盘数据及原始归档仍在研究区，不随Git clone携带。

| 结果 | 可携证明 | 可得结论与限制 |
| --- | --- | --- |
| g1.5试用/Owner B1核心恢复 | [271项审计](checkpoints/20261006-current/results/historical/g1.5-validation.json) | scope、源map、旧ELF、安装及实际恢复检查可查；不是当前输入，也不含完整raw归档 |
| DFS批次优化 | [48次paired结果](checkpoints/20261006-current/results/historical/dfs-batch-paired.json) | 同批次内容与计数/配对结果；热页缓存CPU诊断，非3FS性能 |
| 有限ext4工具 | [24次正向结果](checkpoints/20261006-current/results/historical/ext4-tool-positive.json) | 实际工具矩阵/内容/返回值；非AFS性能及完整evaluator |
| parent FD收益实验结案 | [完整失败报告](checkpoints/20261006-current/results/historical/parent-fd-paired-failure.json) | 48窗口/20对，四项耗时比分别约0.986217/0.982199/0.995844/0.983345，未达原≤0.95；保留FAIL，不反复无输入重跑 |

可携历史文件SHA256：

```text
24f354d7cbf18ac8c534c012f556f5b92ec0b1bd4d5691346b74b6770ca715ae  g1.5-validation.json
9c265bb36d4150dc31ea6c251add419c8eca2a68765f50731cf6b5c68d4cefbb  dfs-batch-paired.json
f29451f7513a54002c0f1529c691ac396209c034c50ff9760cc62d2b833367c6  ext4-tool-positive.json
fd2dbca678dcba28ec07665ddf7d9bcf3e98636af6842a58ca435cfb4ce02c5b  parent-fd-paired-failure.json
```

**事实（历史归档结论，完整raw未包含于本仓）：** Owner B2/B3/B4固定评估器与限定恢复通过；v37 OwnerFs/DFS pjdfstest各236文件/8819 TAP checks、v48远端DFS同全集，无意外失败/skip，28上游TODO登记；ext4标准参考（pjdfstest/FSx/冻结657 LTP命令）通过。它们均保留旧源码身份，不继承当前候选PASS。D20只关闭Owner local容量语义切片；native N2a/N2b1/N2c PF1只关闭身份/权限、服务端及私有引用基础。

研究归档名称供原工作区追溯：`evidence/afs-delivery/g2-owner-{index,structural,subtree}`、`g2-owner-local-d20-r1`、`g2-native-{n2a,n2b1,n2c-pf1}`、`g2-dfs-read-amplification`、`g2-ordinary-tool-qualification-r1`。这些是外部档案位置，不假设独立clone存在。

## trial-artifact

历史0.1.0-g1.5 Linux ARM64包由独立离线安装和中心local-file恢复验证；不是本次重新构建的性能release。本仓含[试用操作说明](../docs/guides/trial.md)与构建/打包脚本，旧包未随Git上传。原工作区产物在 `outputs/releases/afs-g1.5/`，跨电脑需单独传递并校验，不能只checkout旧base b259c44得到该dirty构建。

| 身份 | SHA256 |
| --- | --- |
| g1.5包 | `c767255d4a071fd33fcd5a38fbf9f27d4e939c8892c6201828ef4429b6b88829` |
| g1.5 afs-meta | `93c5528cd410d6f56f1ef87fa51f8da73ad4885e147b75e8a30d5ff87308bdcf` |
| g1.5 afs-node | `43813e8393af59509f1c27f9b16076dbb356f1bb6e481f9f8ca430837883696a` |
| g1.5源map（147输入） | `602a42843427e94c55435420f9460ee38846ca0582c4d209d86c50ececca83bf` |

## native-handoff

[Issue42](https://github.com/lelezi257/dms/issues/42)及[PR43](https://github.com/lelezi257/dms/pull/43)已纳入G2.12/13。既有交接读取head `80b0bca3d9d86a1357aa745bb65abfb567f5623f`，该交接对应草稿未合并分支，不把实验比值当主线生产资格；移植须检查与当前主线重叠的权限/写入/索引改动。

**事实/待验证：** 主线生产native入口仍禁用；已新增管理员实验ON配置及受管接线，尚非可用生产ON；[切片边界](native-workspace-slice.md)。将来要求显式开关、默认OFF，OFF FUSE独立回归，ON经过最终namespace/Root/epoch/Home核验、启动/停止、真正引用排空和重启对账。append/SEEK_CUR、经典kernel POSIX锁、watch/混合mmap、export detach后容器clone仍写等已知缺口必须在ON前闭合；不能用mutex/OFD/lazy detach代替证明。基础模块和测试存在不等于READY或物理排空ACK已完成。

## capacity

**决策：** 按case数据/副本/同时保留量与实测工件峰值准入，结果校验及正常关闭后复用自有空间。日志量先测，不能把完整矩阵100GiB准备条件施加到每个小case。A数据盘32→48GiB可作为独立环境任务，扩容前保存活动memory Meta/FDB状态并验证恢复，不能为此停止不相关服务。本次发布不扩盘、不运行大规模benchmark。

## 下一步

当前G2.04/07/08/11及Owner LTP分支按已记录范围复用；同源码release构建/Owner标准及单机恢复已过，小读写仍FAIL，原dev及release数据都保留，普通路径专项优化暂缓。独立R1仓库整改已完成并发布；R2官方fuser公开API缺口单列阻塞，不能冒称迁移完成。当前DFS本地R1 statfs/固定标准回归已通过，旧失败保留；性能优先容器workspace挂载G2.12必要功能出口→G2.13。bind默认OFF；大规模/复杂可靠性/etcd2GiB专题与Redis后置。

## Review boundary

本次静态审查覆盖配置、Meta/protocol、local-file、Owner私有native/索引、DFS批次/权限、Node健康及发布文件完整性。需确保所有被引用的新源码/驱动/脚本都入Git，并附实际当前验证回执。审查另记录DfsMeta解析权限trait默认退回open_write的未来adapter风险：生产GrpcDfsMeta及当前测试adapter均显式覆盖，未发现当前生产旁路；后续新增adapter需显式实现，见[REVIEW-01](issues.md)。静态审查不替代已列待验收的安装/标准/性能项。
