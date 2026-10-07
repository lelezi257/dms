**2026-10-07 当前新增：** 产品f03dc2b3/map2b17的DFS小规模多节点local-file/R3/gRPC运行和数据完成：三写者各64MiB、1预热1测量及两跨节点读路由，6文件/18 C样本/288物理份，四actualwait0/八服务PID及24远端worker闭合，11保护进程/完整mount库存不变。观察器旧ELF标签FAIL保留，仅R2只读复核，未重跑产品；INFO日志20,155B/23ERRO/60WARN完整保留，不称零错误。A已停止本case完整归档/Linux239条恢复后释放500,867,072B，free回2,327,810,048B。G1历史8/8及G2计数不变，正式3FS/时延/完整POSIX待验。GitHub两次正常推送500，local main修复在f03，origin仍9a8；用户决定等待恢复再推送，独立Linux继续。下一G2.25小删除和G2.27必要组合。 [版本、数据、失败及恢复索引](evidence/20261007-dfs-r3-multinode-current/README.md)。以下保留原时点记录。

**2026-10-07 新增源码小项：** DFS create只对父目录revision/mtime/ctime漂移作64次有界重试，同名/权限/属性变化和真实冲突仍报错、原OperationId/inode/lease/digest不变。Linux6针对性+14 namespace回归、fmt/check及未放宽的strict all-features Clippy PASS；原并发创建FAIL/default-feature lint失败和工具准备记录完整保留。未构建/部署新候选，旧7e6运行FAIL及历史通过不改，G1历史8/8关闭、G2计数不变。下一独立构建新release，再新候选local-file/R3/gRPC小规模多节点回归；环境按B/C42GiB、A/ctl保持运行。 [命令、范围和证据](evidence/20261007-dfs-create-contention/README.md)。以下保留原时点记录。

**2026-10-07 独立环境维护：** 用户决定 A/ctl 保持运行；B/C 数据盘各32→42GiB完成正常停机、字节相同离线备份/Linux只读恢复及扩容后全量身份检查（24,544/10,559条，无缺失/变更）。UUID/分区起点/挂载/内容权限不变，主机实空127.96GiB、本批全容量+备份+临时预留后91.04GiB；无运行测试被打断，不重跑历史通过项。大备份在源码树外，G1历史8/8和G2计数不变。[环境、分级预算与日志规则](vm-capacity.md)、[证据](evidence/20261007-vm-capacity-maintenance/README.md)。下一立即返回G2.24：create修复6+14 Linux回归及strict all-features clippy已有工作区证据，待独立提交/新候选运行，不继承旧7e6 PASS。以下保留原时点记录。

**2026-10-07 新增工具小项：** DFS失败探针sample现保留完整rc/stdout/stderr/error；旧Linux回归KeyError FAIL保留，修后10 worker+6实际进程relay guards PASS。中继保留原失败、排空尾部、逐进程收尾；Lima代理退出不替代远端PID核验。原7e6多节点预热FAIL不变，未跑产品/性能；create竞争Rust修复独立待验收，G1历史8/8及G2计数不变。 [证据](evidence/20261007-dfs-cohort-failure-records/README.md)。以下保留原时点记录。

**2026-10-07 当前事实：** 用户授权后，A已停止旧DFS目录完整归档并经Linux229条恢复核验，释放500,518,912B（477.332MiB）；四角色沿原判据重新准入。G2.24首个预热写FAIL：A/C create元数据条件冲突，B探针后检查ENOTCONN；中继BrokenPipe掩盖部分失败记录，缺口已明示。零有效测量/零读轮次，未重试；四actualwait0/八PID消失、11保护进程和完整mount库存不变。G1历史8/8关闭、G2计数不变。下一独立小项为确定触发create竞争的回归/有界恢复，以及中继失败留证/排空；不扩大矩阵。 [新增证据](evidence/20261007-dfs-r3-multinode-runtime/README.md)。以下保留原时点记录。

**2026-10-07 current G2.24 preparation:** 9 exact Linux driver guards PASS; product runtime BLOCKED before start by A's existing capacity prerequisite (short148.492MiB). Zero data rounds; no performance/3FS verdict. No VM repair/budget relaxation; all4 mount inventories/11 protected identities unchanged. [Evidence](evidence/20261007-dfs-r3-multinode-preparation/README.md). G1历史8/8关闭，G2计数和普通Owner1.2/.8双判据不变；等待容量处理，独立文档收口继续。以下为原时点记录。

**新增小项（2026-10-07，事实）：G2.23当前7e6三同步副本64MiB小写数据完成，正式3FS对照仍待验收。** A/C1/六个不同generation新文件，1预热5计时，中位81.930440MiB/s；96不同4MiB chunks，每轮48物理份及B/C新开全SHA/EOF，四actualwait0/八PID消失及11保护进程/完整mount库存不变。单次产品运行，Linux8 C+7 driver+6 observer guards通过；初始错误文案断言/准备status假设失败留证。测前新case2GiB总预算，最终1,615,421,440B；日志23,261B的39ERRO/60WARN完整保留，不称零错误/完整POSIX。无Rust/vendor/ELF变化，不继承历史性能或升级3FS。G1历史8/8、G2新判据11限定完成/1bind功能进行中/15待验收不变；普通Owner仍1.2×MooseFS吞吐/.8×独立时延待验。下一G2.24小规模多节点读写。[证据](evidence/20261007-dfs-r3-write/README.md)。以下保留原时点记录。

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

**最新开发/交付入口（2026-10-07，决策）：main唯一入口。** 按用户授权，fix/native-orderly-recovery-20261007相对f09185e的全部有效修改（恢复、归属整改、测试、文档和证据）及本次已验证Node启动失败修复73842cd已无冲突fast-forward纳入本地main，本记录随main正常推送；远端实际提交/文件树以推送核对回执为准。其它分支逐项盘点无当前AFS有效遗漏；8个旧dirty工作树及1个untracked草稿原样保留并在源码树外归档、验证补丁可恢复。不创建PR/MR，不设单特性人工审批；必要Linux门禁继续，整体目标完成后统一项目review。合并不升级历史验收，G1历史8/8不重开。下一仍G2.12 accepted Node/workspace生命周期→G2.13性能；普通性能FAIL/复杂可靠性/fuser原版API阻塞后置或独立保留。[纳入、未纳入、版本和证据](evidence/20261007-main-convergence/README.md)。下方按版本保留的历史记录不覆盖本入口。

**当前状态（2026-10-07）：** 3FS基线资格按用户决定留专题，主线容器workspace。main f09185e、产品6d/157/map66/既有ELF包的通过范围保留；新rootfs私有副本/命令续号修复在独立分支fix/native-orderly-recovery-20261007，不视为新可用候选。G1历史8/8，G2为10限定完成/2性能FAIL/2bind进行中/13待验收，bind默认OFF。

**新增/阻塞：** 工具11个独立Linux guards通过；早期Rust10+4有输出但未冻结源码SHA，不计最终版本PASS。静态审阅发现并最小修正非root测试入口，原FAIL保留。afs-build85GiB根盘实际满：误用debug与root工具链下载、后续ENOSPC，已停止构建并求助；最终续号/源码门禁/新包/重启读回均未完成。[版本、失败与证据](evidence/20261007-native-orderly-recovery/README.md)。

**下一项：** 等环境处理决定后一次准入，复用release缓存、普通用户构建、仅真实test binary特权运行；新包后只做4KiB两阶段正常重启读回，不重跑标准/性能或3FS。独立文本/审查与证据收口不受该阻塞。

[append偏移边界与暂缓理由](native-append-offset-boundary.md)：当前公开回复不返回实际追加位置；保留失败；经典锁也已按公开原语边界收口，不改第三方或反复重测。

**历史说明：** 下方旧检查点按原版本/范围保存；其旧下一动作由上方当前入口和验收主表覆盖。当前native偏移/经典锁/watch失败与普通性能失败原始记录均保留。

# Delivery task map

The active task map is [the three-stage acceptance checklist](trial-release-goals.md). This file gives execution order and dependencies; it does not replace the checklist or the full contract in [docs/acceptance.md](../docs/acceptance.md).

## Current checkpoint

G1/g1.5 is complete in its scoped trial sense: installable OwnerFs/DFS, memory demo, basic local/remote behavior, DFS basic behavior, local-file Meta restart recovery and colleague-facing lifecycle/self-check evidence. That result remains historical and bounded. It is not a current-candidate full POSIX, G2 performance or formal 69-case PASS.

G2 is active. The completed G2 items are bounded internal outputs: OwnerFs indexing/structural optimizations, DFS read-batch amplification reduction, and a limited small performance-tool qualification. Bind/native has real progress but remains in progress; the feature must be separately accepted and default OFF. Historical standards remain scoped to their recorded inputs; current OFF delivery and bounded remote deletion are complete, while performance parity and ON qualification remain open.

G3 is deferred. Long soak, broad random/POSIX matrices, complex fault matrices, multi-Meta/HA, etcd 2 GiB topic and Redis are not the next blockers for small usable progress.

**High-priority independent maintenance:** the release slice has finished and stopped. R1 evidence cleanup is published. [R2 upstream gap report](repository-remediation.md) remains independently BLOCKED; continue independent E2E items. G1 and historical conclusions remain unchanged.

## Immediate sequence

Latest user override: pjdfstest targets functional completeness; after high-priority repository remediation, performance priority is the Issue42/PR43 OwnerFs workspace bind mount path (G2.12 prerequisite qualification, then G2.13). Ordinary OwnerFs read/write baselines retain data and defer targeted optimization, and the active target is now [throughput >=1.2x MooseFS plus operation latency <=0.8x MooseFS](ownerfs-performance-criteria.md). The original sequence below remains a dependency/reference map, not an instruction to optimize every ordinary case first.

| Order | Checklist item | Exit |
| --- | --- | --- |
| 0 | G0.03 publish checkpoint | Code and related docs are coherent on GitHub, with verification notes and known gaps |
| 1 | G2.04 OwnerFs pjdfstest current candidate | OwnerFs complete applicable accounting or documented blocker |
| 2 | G2.06 Owner-relevant fixed LTP subset | Frozen list, ext4 reference boundary and OwnerFs result ledger |
| 3 | G2.07 Owner-relevant short FSx | Fixed seed/profile, no content or length mismatch |
| 4 | G2.08 affected Owner/basic + local-file recovery | Owner local/remote basics and Meta local-file restart on current candidate |
| 5 | G2.09 Owner local small read | Correct data, same-condition MooseFS baseline, throughput >=1.2x and operation latency <=0.8x under the frozen case |
| 6 | G2.10 Owner local small write | Correct readback, matching durability barrier, throughput >=1.2x MooseFS and operation latency <=0.8x |
| 7 | G2.11 Owner local delete | Correct namespace and measured comparative report |
| 8 | G2.12/G2.13 bind function/performance | Explicit default-OFF switch, OFF regression, ON lifecycle and paired OFF/ON/ext4 numbers |
| 9 | G2.14-G2.16 Owner remote small cases | MooseFS throughput/latency target for read/write, delete report |
| 10 | G2.05/G2.06/G2.07 DFS standard entry | DFS pjdfstest, DFS-relevant LTP and short FSx before DFS performance claims |
| 11 | G2.08 DFS affected basic check | DFS basics and any needed local-file recovery combination on current candidate |
| 12 | G2.21 DFS one-writer/many-readers | Highest-priority DFS performance item, with correctness and 3FS comparison |
| 13 | G2.22-G2.26 remaining DFS and large cases | Single read/write, multi-node, delete, 512 MiB/8 GiB records |
| 14 | G2.27 core performance delivery | Reproducible package, selected core cases, recovery check and state report |

This order is intentionally small-to-large. Completing a small item keeps its completed state; a later larger scale is a new item, not a reason to reopen the smaller result.

## Working rules

- Start with memory or local-file Meta when the case only needs filesystem behavior. Move to etcd/Redis only for their backend-specific items.
- Fix corruption, unsafe success, permission bypass, acknowledged-data loss and core recovery failure immediately.
- Defer broad coverage when it does not block the current item: 8 GiB, 8-hour soak, long FSx, full differential random, extensive fault axes and backend parity.
- Keep bind/native independent. OFF/FUSE must stay usable while ON is incomplete.
- Do not let a comparator problem stop unrelated functional progress. A blocked MooseFS/3FS lane blocks only the comparisons that require it; incomplete ordinary Owner latency or comparator evidence stays pending, not inferred from throughput.
- Record failed experiments once with enough evidence; do not repeat unchanged failed cases without a new cause or implementation change.

## Historical evidence boundary

Historical results remain useful for regression selection and risk assessment:

- v37/v48 pjdfstest full results prove those historical candidates only.
- g1.5 proves the trial package and local-file recovery scope only.
- Owner B1-B4 and DFS batch work prove bounded internal optimizations, not final performance ratios.
- PR43/bind experiments prove direction and partial numbers, not a shipped ON feature.
- etcd/Redis local evidence proves local slices only; final backend parity is still later.

Current execution reports must cite the exact current source/binary identities before using any result as current-candidate evidence.
