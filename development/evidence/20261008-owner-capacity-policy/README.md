# Owner small-case capacity correction — 2026-10-08

The previous WRITE failure was a frozen256MiB directory-budget breach, not a full B disk. B/C/ctl currently have21,320,093,696/36,878,639,104/1,977,860,096B free. All earlier FAIL_BUDGET and performance conclusions retain their original versions and criteria. This supporting maintenance does not close a product goal.

For the next64MiB/C1 Owner pair, freeze prospective role ceilings B/C512MiB and ctl256MiB, keeping free floors4GiB/512MiB and the complete unused-ceiling reservation. Reuse the existing64MiB file and fixed839/d14 ELFs, baseline/candidate serially; do not copy a dataset per round. [Plan](plan.json).

The maintained `probes/dfs_r3_fixture.py` now admits `budget(additional_bytes=N)` and CLI `budget --additional-bytes N`: it rejects current allocation plus prospective growth above the ceiling, or growth/full-ceiling reservation encroaching on the backing floor. Defaults stay historical1GiB/1GiB and additional0; existing wrappers/failures are not rewritten. It only checks space and does not alter any disk, fixture or product configuration.

[Linux regression receipt](tests.json):19 tests pass, including3 new guards. The original RED and initial nonroot invocation failures remain as external raw records; the latter was corrected with existing sudo, not environment changes.

[Actual prospective capacity-only results](capacity-result.json) on existing stopped ctl/B/C fixtures: new B/C growth reservation256MiB (64MiB each for payload contingency, ELF/tools staging, logs, state/WAL), ctl128MiB (logs/state with remaining allowance for tools). Projected B482,971,648B <512MiB leaves53,899,264B; C415,842,304B <512MiB; ctl231,514,112B <256MiB. All backing free-space and full-ceiling reservation checks pass. No data, helper, binary, service or VM was changed by this check.

The next execution adapter must pass the same frozen ceilings to admission, running observations and closure; do not reuse an old hardcoded256MiB closure for a new512MiB run. Record a new run ID while retaining old dataset/device/inode/config identities. This capacity-only exit is not full launch admission, proof that every driver enforces a separate log cap, functional PASS or performance PASS. Check actual growth during the bounded run, stop and preserve output if a real limit is approached. [Standing capacity rules and larger tiers](../../vm-capacity.md).

The user requested a lasting correction to repeated space interruptions; the previous fixture-choice question no longer gates this future run. No extra confirmation is needed for the planned existing-file reuse. A/ctl memory preservation and deferred DFS A-capacity work remain separate unresolved maintenance; they do not block this Owner pair. G1 historical8/8 and original G2 counts stay unchanged.

2026-10-08 subsequent result: [one valid WRITE pair](../20261008-owner-remote-write-server-pair/README.md) passes prospective capacity and preset modest-improvement retention; current source now retains the identical candidate. Earlier build-only/FAIL_BUDGET states and criteria remain historical, not rewritten. Final MooseFS dual target remains pending.
