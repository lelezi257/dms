# AFS Acceptance Experiment Manifest

This directory holds the first-stage AFS acceptance manifest derived from `source/docs/acceptance.md`.

## Files

- `cases.json` records contract case IDs, applicability, smoke/full boundaries and driver readiness. Formal driver registration remains TODO. Case status stays `NOT_RUN`; results are separate immutable artifacts.
- `acceptance.lock.json` remains `PREPARING`: the image and four guest inventories exist, and network/RXE probes have partial evidence. Comparator mounts, complete reference suites, faults, and release identities are still required before freezing it.
- `runner.py` dispatches registered drivers and verifies structured results and matrix coverage. `test_runner.py` exercises conservative failure handling; runner self-check success is not product acceptance.
- `results/` is reserved for future runner output. Result files should be immutable run artifacts containing commands, logs, JSON/JUnit, seeds, environment identity, success watermarks and raw evidence. Results must not be written back into `cases.json`.

## Active Scope

The manifest contains 69 active first-stage cases:

- `ENV-01` environment gate.
- `STD-01` through `STD-05`.
- `FUN-01` through `FUN-13`.
- `DIST-01` through `DIST-08`.
- `PERF-01` through `PERF-08`.
- `REL-01` through `REL-14`.
- `RDMA-01` through `RDMA-05`.
- `OPS-01` through `OPS-07`.
- `DEP-01` through `DEP-08`.

`REL-15 duplicate-active-meta` is present only as a reserved, non-active ID for later Meta HA design. `TODO-01` through `TODO-07` from the contract are not active manifest cases and do not count toward the first-stage release gate.

## Stages

The stage order follows the contract:

1. `environment` prepares the VM/RXE/baseline gate and deployability prerequisites.
2. `ext4baseline` freezes reference suite behavior and accounting.
3. `short` runs fast representative checks with small data.
4. `semantic`, `distributed` and `fault` cover the product behavior matrix.
5. `perf` runs paired baseline comparisons.
6. `full` runs long stability and full regression gates.

Long 900-second FSx runs, eight-hour soak and 8 GiB performance/basic-IO data are only full gates. The smoke fields keep smaller equivalent boundaries so runner development can validate wiring without pretending those reduced runs satisfy the full gate.

## Matrix Notes

Backend applicability is explicit per case:

- General POSIX, semantic, reliability, deployment and operations cases usually apply to both `OwnerFs` and `DFS` across both `etcd` and `Redis`.
- OwnerFs-only cases are limited to Home/workspace behavior, including `DIST-01`, `REL-12` and `OPS-07`.
- DFS-only cases cover replica, layout, Chunk, repair and multi-source semantics, including `DIST-02` through `DIST-05`, `DIST-08`, `REL-03`, `REL-07` and `REL-09`.
- `RDMA-01` is an environment preflight and is not product file-data-path proof; `RDMA-02` through `RDMA-05` apply to the relevant OwnerFs remote and DFS peer/RN data paths.

Standard suite exclusions are not incremented by this manifest. Exclusion counts belong in the future lock/results evidence and must be fixed before suite execution.

## Execution Boundary

The runner exists, but formal driver bindings and the full environment are still being prepared. `drivers/standard.py` implements pjdfstest execution, accounting and guarded remote orchestration; its Linux selftests and captured development runs are recorded in `../evidence/`. The STD-01 manifest entry remains TODO until its release environment and backend bindings are complete. TODO registrations return BLOCKED. An unfrozen lock blocks full acceptance. Development probes outside the manifest retain their own scope and identities, never substituting for a product case PASS. Concrete runs belong in `results/`; do not overwrite case statuses with preparation claims.
