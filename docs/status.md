**2026-10-07新增事实：G2.22当前7e6/c3bb的A单读者64MiB小项完成内容/计时/正常闭合，正式3FS对照仍待验。** 1预热5读，中位65.624896MiB/s；前后48物理副本，四actualwait0/八PID消失及11保护进程/完整mount库存不变。Linux11工具guards及独立观察校验通过；首轮遗漏结果目录导致写前拒绝，原FAIL及四正常退出保留，修测试准备后一次数据运行，无Rust/vendor/VM修补。G1历史8/8、G2大项计数/defaultOFF不变，G2.23写性能/G2.27/full bind仍开放。当前OFF标准限定复用已由既有impact-map及当前安装恢复覆盖，不重跑整套；下一G2.23三同步副本小写入摸底。 [证据](../development/evidence/20261007-dfs-r3-local-read/README.md)。

以下保留原时点记录。

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
| G2 core performance version | **ACTIVE** | 27 independent tasks: 10 bounded outputs complete, 2 have performance failures, 2 bind tasks in progress, 13 awaiting acceptance. Current Owner standards and short Owner/DFS FSx qualified in recorded scope; local read/write below target; local R1 DFS statfs and fixed standards now pass; two-VM core recovery now passes; comparator qualification and new performance package remain open |
| G3 complex reliability/backends | **Deferred, 13 tasks** | Long-running/complex faults, expanded matrices/HA; etcd topic at 2GiB, Redis last |

## Current code capabilities

| Area | Present behavior | Acceptance boundary |
| --- | --- | --- |
| Runtime | afs-meta/afs-node, TLS gRPC, REST health, separate OwnerFs/DFS FUSE mounts | Current Linux source/tool gate is recorded with exact input hashes; current two-VM Owner/DFS core and orderly central Meta recovery pass in G2.08 scope; independent performance package delivery remains G2.27 |
| Meta | memory, local-file, etcd, Redis implementations; persistent capability is distinct from volatile state | G1 central local-file recovery is qualified on g1.5. Other backends have scoped historical tests; broad parity/faults remain G3 |
| OwnerFs | Home files, remote routing, write-authority/lease checks, error propagation, ordered namespace/index maintenance | B1–B4 internal correctness/evaluator outputs complete in limited scope; e925 local pjdfstest and fixed six-test LTP pass in their recorded scope; short FSx passes. 64MiB/C1 read/write correctness passes but performance fails at 0.5709/0.6297×ext4; MooseFS read/write parity remains unqualified; current 6d small remote deletion correctness, quantified report and normal cleanup complete [G2.16](../development/evidence/20261007-owner-remote-delete-small/README.md) |
| DFS | immutable chunks, version commits, replica policies, coherent read plans, streaming integrity and shared verification within one read batch | Batch CPU/read-amplification diagnostic complete; R2/64MiB one writer and two independent Node reader views pass, including orderly central Meta recovery; performance comparison and three-sync-durable 3FS parity remain unqualified |
| Capacity/health | Observed backend capability and readiness, Owner local filesystem capacity/error handling | Owner D20 and current local R1 DFS real fstatvfs slices only; remote/replicated aggregate capacity and wider faults remain open |
| OwnerFs workspace bind mount | OwnerFs core in one bind_mount.rs, authorized physical Home source→FUSE first-level covering mount, normal identity-checked unmount; runc is a default-OFF experimental adapter | [Core ownership/naming plan](../development/ownerfs-workspace-bind-remediation.md); the independent default-OFF host switch covers a physical Home directory at the real FUSE first-level target; [931 fixed-Home host visibility/permissions/normal closure PASS](../development/evidence/20261007-ownerfs-workspace-host-runtime/README.md). The runc adapter uses a separate private namespace. Multi-workspace and generalized lifecycle remain open. [Runc adapter slice](../development/native-workspace-slice.md); [Official runc installation/runtime-only admission PASS](../development/evidence/20261007-runc-runtime/README.md); [Actual managed single-container lifecycle PASS](../development/evidence/20261007-managed-workspace/README.md); Mixed-path mmap bytes/permissions pass; locks, append offset and cross-watch fail in the [short semantic evidence](../development/evidence/20261007-managed-semantics/README.md). Managed ON functional/performance exit, production READY/revocation/drain and restart reconciliation remain unqualified |
| Packaging | Process control, offline package generation, trial configuration and selfcheck | g1.5 retained; main6d51aeb default-OFF fresh compiler-free install and bounded recovery35checks pass; [fixed trial package](https://github.com/lelezi257/dms/releases/tag/afs-trial-6d51aeb) is available. not a performance release or container ON package |
| Transport | gRPC and optional RDMA code plus scoped fault proofs | Actual RXE short results do not qualify the whole RDMA exception/resource matrix or physical NIC performance |

## Validation boundaries

Historical full pjdfstest results belong to v37/v48. Historical e925c5b Owner local pjdfstest passes 236 files/8819 checks (28 upstream TODO, zero skips/unexpected failures); this is not full POSIX certification. The e925 DFS ENOSYS/zero-TAP/six-TBROK failures remain historical receipts. The new map6161e25b release candidate passes local R1 DFS pjdfstest236/8819 with28TODO and zero skips/unexpected failures, plus fixed LTP6/6 (651 unselected), with normal stop/unmount. [Historical candidate evidence](../development/evidence/20261007-dfs-statfs/README.md); [6d scoped reuse audit](../development/evidence/20261007-standard-reuse/README.md), with no new6d full-suite run. Owner/DFS short FSx passes seed1/1000 only. [Current raw ledger](../development/evidence/20261006-e2e-current/README.md) records the scope and failures. ext4 reference results are not AFS passes. The original full 69-case manifest remains NOT_RUN with environment PREPARING; it is the expanded final catalogue, not the G1 progress denominator.

See [checkpoint results](../development/checkpoints/20261006-current/results/README.md) for fresh combined-source checks. The historical 271-check g1.5 audit, DFS batch paired results, ext4 finite-tool positive results and complete parent-FD failed experiment are available in the repository. Other archived scopes are explicitly identified as external archives in the checkpoint page.

## Next and priority

Latest user priority: standard pjdfstest for functional completeness; container-mounted workspace access from Issue42/PR43 is the first performance lane, after necessary default-OFF bind functionality/safety. Ordinary local/remote/DFS measurements can remain baseline reports with optimizations deferred. [R1 historical evidence cleanup](../development/repository-remediation.md) passed restoration and fixture checks; R2 upstream migration remains independently BLOCKED by public lock/cancellation API gaps, with the full diff recorded. This does not block independent functional or container workspace work.

Reuse passed current standards and single-/two-VM core recovery. Preserve both dev and same-source release local performance failures. Release build, Owner standards/basic operations and single-VM orderly Meta recovery passed; ordinary read/write baselines are retained and targeted tuning is deferred. Remote comparisons require independent qualification. When entering DFS, one-writer/many-reader is highest priority. Each case freezes its own prerequisites, fairness and budget; a missing complex baseline or large-disk condition does not block an independent safe small case. Owner local target is ≥90%ext4 throughput; remote target is MooseFS parity; DFS matches 3FS under the same POSIX interface and three synchronous durable copies. Delete requires correctness and a quantitative report, without a new hard ratio.

Bind functionality and performance are separate G2.12/G2.13 exits, with an explicit experimental switch default OFF. OFF delivery proceeds independently; known ON safety gaps must be resolved before enabling ON. Complex reliability/etcd/Redis work remains later, while ordinary-use corruption, unsafe success, permissions and core recovery defects are repaired immediately.
