# Staged delivery validation

[Acceptance](../docs/acceptance.md) owns targets and topology. The execution manifest expands its cases; neither scripts nor skills may weaken this contract.

## Environment

macOS is for editing, reading and host-side VM orchestration only. Compile, lint, unit/integration tests, probes, filesystem services, faults and benchmarks run in Linux. Acceptance uses the specified dedicated four ARM64 Ubuntu VM topology, ext4 volumes and verified RXE network. Builds use a separate Linux build VM and never compete with a measured lane.

Record exact source and binary identity, environment lock, mount/backend/replica/transport configuration and evidence. Sharing source with a VM is allowed; storing tested data on a macOS share is not. Preserve old experiment data; do not stop unrelated workloads. Before performance runs account for competing VMs/processes and validate resource isolation.

## Feedback stages

The four [delivery rounds](plan.md#whole-system-rounds) determine coverage and priority; the feedback stages below determine validation cost within each round. They are not sequential module-completion gates. Each round checks the whole system and maintains [the issue list](issues.md). At round end run the overall regression for that round's integrated candidate, assess coverage and unresolved consequences, and select the next priorities. Preserve valid old evidence under its original source and runtime identity.

1. Environment: pin images/dependencies, verify volumes/network/TLS/backends/RXE, ext4 reference suites, actual MooseFS/3FS mounts and baselines. Missing prerequisites are BLOCKED, not PASS.
2. Small change: replay the original failure, run regressions for the affected modules and perform necessary compile checks. Advance to the next development task when this scope passes. Use small fixtures for chunk boundaries, streaming, memory limits and errors; do not repeat the full source gate for each edit.
3. Related feature or repair batch: run the affected core integrations after the batch forms a working end-to-end flow. Select file write/read/sync/close/reopen, consistency, commit idempotency, replica recovery and RPC authorization according to the changed contracts and callers.
4. Stage completion, batch integration or release preparation: run the full Linux source gate together, including formatting, strict workspace/all-target/all-feature Clippy, library and interface contracts, privileged FUSE checks, supported feature configurations and binary build. Public interface, data-format or cross-module contract changes expand checks earlier; do not defer a necessary consumer or compatibility check until batch end.
5. Performance and release: keep 8 GiB files, full POSIX/FSx/random-operation matrices, paired performance repetitions and long stability/fault runs in their planned acceptance stages. Use frozen representative short workloads for diagnostics first. Do not start product performance tuning before valid comparison baselines. Short checks never replace the required full acceptance matrices.

### Select checks by impact

Before editing, record the original failure, changed contracts/callers, selected checks and the remaining batch gate. A small internal change normally needs the failing case, related module regressions and compilation of affected targets. Widen the selection when shared interfaces, serialization, persistence, authorization, durability or transport completion can affect other modules.

Deployment-script-only changes require script syntax/regressions and real deployment or restart checks. Reuse the existing qualified Rust source/binary evidence when those inputs are unchanged; do not rerun all Rust tests for a shell-only fix. A package or dependency change that alters product binaries is a broader change and must validate those binaries.

Core integration selection follows the risk: a write-state change needs visibility and barrier checks; a Meta commit change needs precise idempotent replay; a replica change needs durable completion and failure recovery; an RPC contract change needs both endpoints, authorization and every affected transport. Run these checks at the batch boundary, or earlier when needed to resolve a cross-module risk.

### Reuse frozen evidence

Valid evidence for the same frozen source can be reused. Bind it to exact input hashes, binary identity, toolchain/dependencies, test selection and relevant runtime configuration. When code changes, rerun the affected checks and identify which unaffected results remain applicable. Changed test code, dependencies or environment can also invalidate evidence. Preserve previous results under their original identities; never relabel an old result as a test of new inputs.

At batch end, complete the full source gate for the final candidate. A shell-only batch can reuse the unchanged Rust gate together with fresh script and runtime evidence. Record what ran, what was reused, why reuse is valid and what remains unrun. Failure or incompatible inputs require fresh validation of the affected scope.

### Result levels

| Level | What it establishes | What it does not establish |
| --- | --- | --- |
| Local regression PASS | The identified failing case, selected module regressions and necessary compilation pass | Other modules, the full source gate or release acceptance |
| Mainline flow PASS | The explicitly identified installation/deployment and healthy end-to-end flows work on the candidate and configuration tested | A complete source gate, fault recovery, untested axes or formal delivery |
| Stage gate PASS | The frozen candidate satisfies its complete source gate and the selected affected core integrations | Unrun formal cases, other backend/transport axes, performance or long stability gates |
| Formal acceptance PASS | A required case passed its declared matrix in the qualified environment with complete identity and semantic evidence | Other cases or overall delivery while any mandatory gate remains unresolved |

Overall delivery requires all mandatory acceptance gates. Keep local/stage results separate from formal case status; neither can turn a formal `NOT_RUN` into `PASS`.

Report current round, healthy coverage, stage-gate identity, formal status and deferred issues separately. A round can advance after its defined whole-system scope and regression pass with nonblocking issues assigned to later rounds; it cannot defer a known incorrect success, corruption, authorization failure, commit-order violation or normal-use resource leak. Full-source validation remains due at batch/round boundaries. Do not rerun the same unaffected gate for every small correction.

Core filesystem development uses a memory-backed Meta first, so POSIX, layout, visibility, replication and transport defects can be reproduced without persistent-backend variables. Keep existing etcd/Redis integration work and validate it in separate lanes. Memory-backed runs do not prove Meta restart durability or qualify a performance comparison that requires durable metadata. Backend parity and persistent recovery are completed after the core functional and performance development gates.

## Trustworthy results

Check content, EOF, attributes, errno, successful durability watermarks and recovery. Verify faults occurred and RDMA file bytes actually used verbs. A TCP fallback is not an RDMA PASS. Track every discovered suite test, including skips/TODO/unfinished items; no post-failure exclusions.

Capture minimal failure sequences and add regression cases when fixing defects. Use PASS/FAIL/BLOCKED/INCONCLUSIVE; NOT_RUN is a preparation status and cannot become release PASS. Publish raw valid paired benchmark runs, not fastest samples. No implicit tolerance or relaxed replica/durability configuration.

An acceptance skill references the same environment lock, case manifest and runner. It must reject missing evidence, stale binaries and altered contracts.
