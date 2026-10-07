**2026-10-08 新增限定通过（事实）：** 固定 f03/7bfc 新增真实授权失败下 FD/mmap 引用排空 PASS（56 checks）：关闭监听后 FD+mmap 保留原 mount，关闭 FD 后 mmap 仍保持；释放映射后12.09s内 Node/监督者及 bind/FUSE 正常关闭，原 Node 实际授权错误 wait1、触发 Node/Meta wait0。Linux11 guards及131 raw/8当前工具/2旧工具/18前轮工具版本实际恢复核验通过。 无 Rust/vendor/VM 改动，G1历史8/8关闭、G2总数/defaultOFF不变，完整 bind/正式性能未完成。[本项证据及边界](../evidence/20261008-workspace-bind-reference-runtime/README.md)。append/偏移已有真实 FAIL 保留；官方 WRITE/SEEK_CUR API 无支持的最小修补，作为协议专题后置，不改内核/vendor或绕过双开关安全拒绝。下一普通 Owner 核心小读的物理缓存可比性准入，预先固定条件，不能准入则停该项；已通过标准/八bind核心性能/恢复不重复。

**2026-10-08 新增限定通过（事实）：** 固定 f03/7bfc 复用此前授权错误关闭后的原配置、local-file Meta WAL/Home 与原 4KiB 文件，一次进程重启恢复 PASS（44 checks）：新会话 epoch5，原真实目录 bind 到 FUSE 的 workspace 一级目录，内容/EOF/inode/权限不变，Meta/Node 正常 wait0、无残留。首轮测试工具参数错误在启动前 BLOCKED，原记录保留；修正后 Linux8项检查及97 raw/18工具版本恢复通过。无 Rust/vendor/VM 修改；G1历史8/8关闭、G2总数/defaultOFF不变，完整 bind 仍进行中，不计 crash/即时撤权/完整POSIX/性能。[本项版本、结果与边界](../evidence/20261008-workspace-bind-recovery/README.md)。下一独立项：核对已有混合路径 append/文件偏移失败，确定最小必要修复及针对性回归；实时 root-command watch/即时 FD 撤权、Moose 比较资格继续单列，已通过标准、八项 bind 性能及恢复不重复测试。

**新增小项（2026-10-07，事实）：G2.23当前7e6三同步副本64MiB小写数据完成，正式3FS对照仍待验收。** A/C1/六个不同generation新文件，1预热5计时，中位81.930440MiB/s；96不同4MiB chunks，每轮48物理份及B/C新开全SHA/EOF，四actualwait0/八PID消失及11保护进程/完整mount库存不变。单次产品运行，Linux8 C+7 driver+6 observer guards通过；初始错误文案断言/准备status假设失败留证。测前新case2GiB总预算，最终1,615,421,440B；日志23,261B的39ERRO/60WARN完整保留，不称零错误/完整POSIX。无Rust/vendor/ELF变化，不继承历史性能或升级3FS。G1历史8/8、G2新判据11限定完成/1bind功能进行中/15待验收不变；普通Owner仍1.2×MooseFS吞吐/.8×独立时延待验。下一G2.24小规模多节点读写。[证据](../evidence/20261007-dfs-r3-write/README.md)。以下保留原时点记录。

**Current selected exit:** G2.13 current7e6 small-data plus same-candidate metadata eight-core performance PASS; OFF diagnostic FAIL retained, production ON/G2.12 open. [Evidence/accounting](../evidence/20261007-workspace-bind-data-current/README.md). Next current OFF standard applicability/reuse audit; no unchanged suite repeats.

**当前元数据窗口修正与小项通过（2026-10-07，事实）：** 7e6/c3bb当前包，容量检查移到after快照之后；旧工具先1FAIL、修后7Linux guards通过，原931 FAIL保持。新1000×4KiB/C1六阶段各1预热5配对，ON六项>=.90×ext4通过、OFF六项FAIL留数；八选定回调ON0/OFF阳性，其它getattr24保留。190驱动/193独立检查、四wait0/十二PID正常闭合。无Rust/vendor/C变更，不升级完整ON/G2。下一当前DFS R3小一写两读内容/副本回归；历史计时不刷分。[当前证据](../evidence/20261007-workspace-bind-metadata-window/README.md)。

以下为原时点记录。

**当前新增元数据观察（2026-10-07，事实）：** maine903基底/产品93169c8与157编译输入/ELF包未变。OFF1000×4KiB/C1六阶段完成1预热5配对、全部性能FAIL留数；ON首个预热内容正确，但全节点计数窗口readdir4未满足预定0，停止且ON比较未完成。窗口含遍历FUSE树的容量检查，不能把回调归因于业务或改判据称PASS。135驱动检查134PASS/1FAIL；117独立证据/正常闭合检查与13 Linux工具测试PASS，不等同用例PASS。实际Meta/Node四wait0、容器/PID/mount闭合、保护对象与原预算均核实；无Rust/vendor/C改动、重建、环境修补或刷分。G1历史8/8及大项计数/defaultOFF、先前数据子项PASS不变；下一DFS一写多读，计数归因/元数据ON补测单列后置。 [版本、FAIL、全部OFF数据与闭合](../evidence/20261007-workspace-bind-metadata-perf/README.md)。main仍唯一入口，修复分支全部有效成果已纳入，旧8dirty工作树/草稿保留且HASH复核一致；无PR/审批关卡。

以下按原时点保留历史身份。

**新增当前限定通过（2026-10-07，事实）：** main59f8753基底/157-map1fd613e5，Node保留workspace worker并在FUSE关闭前等待；真实EBUSY正常重试，终止性错误保留claim，监听失败先通知关闭。Linux9门禁（含release构建）及26选定测试通过，旧EBUSY FAIL和首测试编译FAIL保留。尚未打包或执行新Node/runc整机关闭；下一仅验该运行小项。G1历史8/8、G2计数/defaultOFF不变，历史标准/性能身份不升级。[证据](../evidence/20261007-ownerfs-bind-shutdown-drain/README.md)。

**新增当前限定通过（2026-10-07，事实）：** main253057f基底/157-map a062bfe8，两个真实Linux挂载清理重试缺陷先2FAIL复现、修复后2PASS；普通10测试、配置7测试及七项受影响源码门禁通过。仅确认clone卸载/容器删除阶段推进，失败保留同一authority/export；未构建新服务包或执行官方runc/Node E2E。G1历史8/8、G2计数/defaultOFF不变，6d已存性能数据保持历史身份并复用，不重复刷分。下一独立Node关闭时的忙引用所有权/排空小项；宿主独立开关及混合语义缺口仍未完成。[版本、原始失败与结果](../evidence/20261007-ownerfs-bind-cleanup-retry/README.md)。

# AFS acceptance tooling

This directory contains tooling and manifests for AFS acceptance development. The current execution order is defined by [the three-stage checklist](../trial-release-goals.md). The full contract remains [docs/acceptance.md](../../docs/acceptance.md).

The most important distinction: tooling readiness is not product acceptance. A driver that can run, a preparing lock, a smoke run or a historical result does not make a current candidate PASS.

## Files

- `cases.json` records the formal case IDs, applicability, smoke/full boundaries and driver registrations.
- `acceptance.lock.json`, when present, binds environment, suites, references, source, binaries and runner identity. A `PREPARING` lock blocks formal acceptance.
- `runner.py` dispatches registered drivers and verifies structured result and matrix accounting.
- `environment.py` evaluates hash-bound preparation observations. It can report missing or unsupported predicates, but it cannot freeze ENV-01 by itself.
- `drivers/` contains registered adapters for standard suites, health, accounting, target identity, mount isolation and related preparation checks.
- `results/` is reserved for immutable run artifacts. Do not write PASS results back into `cases.json`.

## Relationship to the three stages

| Stage | Tooling role |
| --- | --- |
| G1 trial | Historical g1.5 evidence is outside this manifest; do not relabel formal cases as PASS for G1 |
| G2 standard fallback | Use registered suite drivers for pjdfstest, fixed LTP subset and short FSx on the current candidate |
| G2 performance | Use the manifest only for identity/accounting support; each performance case must freeze its own comparator, resource and pass-line record |
| G3 formal release | Run the full manifest, long cases, failure matrices and backend axes only when the environment lock and prerequisites are qualified |

The formal manifest currently remains NOT_RUN/ENV PREPARING unless a result artifact for a specific case says otherwise.

## Active scope

The formal manifest contains 69 active first-stage cases:

- `ENV-01`.
- `STD-01` through `STD-05`.
- `FUN-01` through `FUN-13`.
- `DIST-01` through `DIST-08`.
- `PERF-01` through `PERF-08`.
- `REL-01` through `REL-14`.
- `RDMA-01` through `RDMA-05`.
- `OPS-01` through `OPS-07`.
- `DEP-01` through `DEP-08`.

`REL-15 duplicate-active-meta` is reserved for later Meta HA design and does not count toward the active gate. Contract TODO items are not active manifest cases.

## Standard suites for G2

The G2 fallback path uses mature standard suites first and custom cases as supplements:

- `STD-01 pjdfstest`: complete applicable accounting for OwnerFs and DFS.
- `STD-02 LTP-filesystem`: fixed file/permission/lock subset.
- `STD-03 FSx`: short fixed-seed run for current-candidate regression.
- `STD-05 suite-accounting`: discovered, passed, failed, excluded and unrun counts must balance.

Long FSx, broad differential random and the full formal matrix are G3/final gates unless a specific G2 item calls them in.

## Driver boundary

`READY` means dispatch code exists. It does not mean the target, environment or case has passed. `TODO` registrations return BLOCKED. A missing, ambiguous or unfrozen suite binding must block the case instead of silently choosing a reduced run.

Set `AFS_ACCEPTANCE_SUITE_BINDINGS` to an absolute JSON path for suite drivers. Bindings provide observed suite paths, mounts and process identities; they do not provide arbitrary shell commands, reduced full-mode seed counts or case overrides.

Remote standard-suite runs must bind the Meta and Node identities on the guests that actually host them. A PID from another VM is not a local identity. The driver must verify identities before and after the suite and keep raw child stdout/stderr/proof files.

## Result rules

Each result artifact must contain:

- Source, binary/package, runner, lock and mount identity.
- Exact command and environment.
- Raw logs plus structured JSON/JUnit/TAP when available.
- Discovered test accounting and predeclared exclusions.
- Success watermarks, content checks and cleanup observations.
- PASS/FAIL/BLOCKED/INCONCLUSIVE with the reason.

Never promote a smoke profile to full coverage. Never use memory Meta success as durable-backend evidence. Never use OFF/FUSE results as bind/native ON evidence. Never rewrite historical failures after a later candidate passes.

## Current DFS R3 normal recovery

`dfs_r3_small.py writer/check --candidate /absolute/expected-candidate.json` explicitly binds the current source/map/Meta/Node/probe identities; omitted candidate retains the historical931 default. `check` performs one fresh-open content/EOF read with no performance claim. `probes/dfs_r3_fixture.py --fixture-name NAME --meta-sha256 SHA --node-sha256 SHA` selects a fresh fixture while retaining exact generatedR2-to-finalR3 admission and initialized-state refusal. [Plan](../dfs-r3-current-recovery.md) / [current7e6 runtime evidence](../evidence/20261007-dfs-r3-current-recovery/README.md).

### No-touch physical backing-cache observation

`probes/physical_cache.py` observes a fixed regular-file range with read-only mmap/mincore, no payload access; validate with Linux `PYTHONDONTWRITEBYTECODE=1 python3 development/acceptance/probes/test_physical_cache.py -v`. Bounds/header exclusion, identity drift, symlinks, cleanup and cold no-prefault coverage are tested. Client FUSE cache and physical backing residency are separate layers; snapshots do not pin pages or prove disk/host cache. [Frozen local64MiB backing-hot/default-client FAIL and reproducible recipe](../evidence/20261008-owner-local-read-backing-hot/README.md) preserves old client-hotFAIL and independently measured p95. No default product/test binary or dependency changes.
