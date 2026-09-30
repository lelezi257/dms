---
name: afs-acceptance
description: Prepare and run AFS OwnerFs/DFS delivery validation using the agreed ARM64 Linux VM environment, acceptance cases and comparison baselines; use during AFS development, regression and release acceptance.
---

# AFS acceptance

Project layout: `<work>/source` is this Git checkout; `<work>/experiments/afs-acceptance` holds materialized preparation inputs. Find `<work>` from the checkout location; do not hardcode a host username or machine path. Read `docs/handoff.md` in the checkout first.

Read `AGENTS.md`, `docs/acceptance.md`, `development/validation.md` and `docs/handoff.md` in the Git checkout. If the research workspace still exists, also read `execution/README.md` and the current task in `execution/plan.md`; do not infer current scope from old DMS skills or old VM services.

## Inputs and boundaries

`experiments/afs-acceptance/cases.json` is the case manifest; `acceptance.lock.json` binds exact environment, suites, reference systems, binaries and runner. A PREPARING lock is not an acceptance environment. TODO drivers and reserved REL-15 cannot produce PASS. Follow the runner's real implemented CLI; inspect `--help` rather than invent options.

Compile/test/run inside Linux; macOS can edit and orchestrate Lima only. Tested data uses guest ext4. Target ctl/A/B/C resources, independent mounts, Redis durable backend and real RXE requirements come from acceptance.md. Do not replace the environment, lower thresholds, add exclusions after failures or count fallback as RDMA.

## Use

1. Validate actual environment/source/binary identity against the lock. Stop the selected lane on mismatch and record BLOCKED. During preparation retain unknown values instead of filling them from expectations.
2. Select task cases and applicable backend/meta/replica/transport axes. Use smoke for development feedback and full for release. Smoke success never satisfies an unrun full matrix.
   Prefer memory-backed Meta for core functional development and short performance diagnostics. Keep etcd/Redis persistence, parity and recovery in separate lanes; memory success is not durable-backend evidence or a substitute for a qualified durable comparison.
3. Run the registered driver with bounded timeout. Save command, raw output, structured checks, successful watermarks and fault proof. Exit zero without semantic evidence is insufficient.
4. Repair defects within accepted architecture, add a regression, rerun the affected cases on Linux, then broader milestone cases. Keep pending commit identity and durability semantics intact.
5. Persist raw evidence in `evidence/afs-delivery/<task>/<run-id>/`, keep results immutable, and update execution checkpoint. Final report lists PASS/FAIL/BLOCKED/INCONCLUSIVE and pre-reviewed exclusions with complete discovered-test accounting.

## Release

Only frozen fair MooseFS/3FS baselines can justify performance ratios. Known missing strong-durability baseline evidence remains BLOCKED, not a waived gate. Run full 8GiB/FSx/8-hour soak and deployment matrices at final stage; exercise the underlying boundaries with small fixtures earlier. Complete final architecture/interface change review and package reproducibility before declaring the goal complete.

The runner and environment are still being prepared. Read `docs/handoff.md` for the portable checkpoint and preparation inputs; the original research workspace also preserves raw results under `evidence/afs-delivery/`. This skill is guidance, not evidence that any product acceptance gate has passed.
