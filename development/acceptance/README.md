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
