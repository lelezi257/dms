# Contract identity and environment preparation guard

## Result

**Local tooling regression PASS. Formal environment qualification BLOCKED.**
The runner binds the actual acceptance file's SHA-256 and requires a hash-bound
environment bundle. A `FROZEN` flag and four correct release identity hashes
alone cannot produce a full release PASS. The actual lock stays **PREPARING**;
all **69 formal cases stay NOT_RUN**. Acceptance thresholds are unchanged.

The bounded evaluator checks captured host, Lima and guest metadata. The actual
[preparation report](linux/preparation.json) records **35 PASS, 11 BLOCKED,
0 FAIL**, exit 2. Guest inventories are reused observations from v65; host and
Lima metadata were captured for this slice. This is evaluation of immutable
observations, not a fresh live qualification of every service. Initial host
reserve has no pre-provisioning observation; current reserve cannot replace it.
Network/TLS/fault recovery, durable backends, cross-VM verbs, complete reference
accounting, comparator mounts and frozen run contracts need dedicated checks.

## Verification scope

- [Original runner failures](original/original.log): two assertions reproduce
  false full PASS with missing environment proof or a changed acceptance file.
- [Original malformed-input failures](nested-original/original.log): two methods
  retain four errors before input hardening.
- [Original metadata failures](metadata-original/original.log): one method retains
  seven failed subcases for failed commands, wrong mount/filesystem and image
  arch/location. Original scripts and hashes accompany each log.
- [Local Linux regression](linux/local.log): 18 tests PASS; these precede final
  path hardening and retain their own [input identity](linux/inputs.sha256).
- [Related ctl batch](linux/batch.log): 148 tests, 145 PASS and three FAIL because
  ctl's system Python lacks Hypothesis. This failed batch is preserved.
- [Failed-case replay](random-linux-a/replay.log): those three tests PASS with
  A's existing Hypothesis 6.168.3 environment. The subsequent
  [whole affected module](random-linux-a/regression.log) has **11 PASS**.
  [A identity](random-linux-a/identity.json) and
  [dependency freeze](random-linux-a/dependencies.txt) record the difference.
  Only the random driver module was rerun; unaffected ctl results retain their
  original identity. Combined scope validates 148 distinct methods, not 159.
- Linux `py_compile` PASS. Final [ctl inputs](linux/final-inputs.sha256) and
  [A inputs](random-linux-a/final-inputs.sha256) bind the tested scripts; shared
  files match. No Python tests, Rust build or product probes ran on macOS.
- Actual runner smoke/full calls remain BLOCKED with the preparing lock. The
  [standalone default-path probe](linux/portable-default.log) locates and hashes
  `docs/acceptance.md`; standalone list/full guard also ran on Linux. The first
  scratch list attempt omitted cases.json and failed before dispatch; after
  copying the unchanged manifest, [list](linux/runner-list.log) succeeds.
- [Independent review](static-review.md) is static evidence only.

The [Rust reuse check](rust-reuse.json) confirms 143 unchanged compile inputs
against the preceding file-commit gate. That gate remains under its original
identity; no full Rust rerun was needed for this Python-only batch. This result
does not claim a new Rust stage gate or any formal acceptance PASS.

## Interface and limits

`runner.py --contract` selects the actual contract file; default paths support
the research workspace and standalone product clone. `lock.contract.sha256`
binds accepted content; `contract_sha` retains Git provenance only.
`lock.environment_evidence` references a relative bundle path and digest.
Bundle artifact paths are relative to the bundle directory and SHA-256 bound.
Missing, malformed, altered or incomplete evidence cannot qualify full release.
Development smoke remains independently runnable within its declared scope.

No product RPC, module, binary format, dependency or runtime behavior changed.
This evaluator is partial by design; generic PASS receipts are not semantic
proof. Complete CLI diagnostic polish for malformed top-level JSON is not
established by this slice. SDK/cache/spill/Meta HA and full POSIX, performance,
8 GiB and long stability stages retain their existing scope and schedule.

The [artifact manifest](artifacts-manifest.json) binds this evidence set. The
handoff and AGENTS files remain unchanged; delivery goal remains active.
