# Asynchronous immutable-chunk repair

## Contract

The initialization policy remains N desired copies / M synchronous copies.
File barriers publish after M durable receipts and retain repair work in the
same Meta commit. The source-owning worker claims a fenced task, verifies one
bounded local chunk, executes the existing source-first full-N chain and reports
exact receipts. Meta promotes copies and completes the task atomically. File
versions and extent maps are unchanged by adding replicas.

Claim/report timeouts retain the precise original request. Report retry does not
transfer bytes again. Expiry rejects new receive authorization; late exact
reports can finish a still-current Running claim. A specific superseded result
ends an obsolete claim; generic CAS conflicts retain the report. Corruption is
reported only for the bound source after local verification. No live source is
unavailability, never inferred permanent loss.

## Coherent Linux source gate

[Gate report](source/linux/qualified-linux-clean/report.json) binds ARM64 Linux
6.8.0-106, [143 inputs](source/linux/qualified-linux-clean/compile-inputs.json),
strict all-feature/workspace Clippy, formatting, feature checks and binaries.
[Root hash audit](root-source-verification.json) matches every input to the host
source. Host execution only copied/hashes files; Rust and filesystem validation
ran in Linux.

- Library: 371 passed, 2 existing environmental tests ignored.
- Contracts: 58 passed; the 5 privileged FUSE tests omitted by the ordinary
  contract invocation all pass in the separate real-root FUSE invocation.
- Error crate: 4 passed. Local API: 9 passed.
- Feature configurations: none, OwnerFs and DFS pass independently.

Unstripped Node SHA256:
`fda4102cc742267f20325097fc33e876449b48ff0caef6605b7a24666dd6e060`.
Unstripped Meta SHA256:
`d57423534870f9258ec7a76bf627c49b3296b418fcab503b27e7291b00029aeb`.

## Retained failed attempts

- v52/v53 diagnostics retain oversized staging, initial Arc move/enum sizing
  errors and a test constructor typo. The oversized copy was stopped before
  Cargo; only that new guest staging copy was removed, preserving original
  evidence. [Early diagnostics](source/early-diagnostics/v52/staging-first.txt).
- [v53 contracts](source/failed-v53/qualified-linux/contracts.log) rejected a
  valid ACK at catalog5 after unrelated advancement to6. Current placement is
  not the write's frozen floor. The existing test remains unchanged; receivers
  still enforce original grants and repair ACKs enforce the precise claim floor.
- [v54 contracts](source/failed-v54/qualified-linux/contracts.log) exposed a new
  REST fixture lacking required transaction conditions/outcome. The fixture now
  obeys the store contract; product validation was not relaxed.
- [v55 error check](source/failed-v55/qualified-linux/error.log) found the new
  superseded code missing from the language-neutral catalog. The entry was added.
- [v56 cache attempt](source/failed-v56-cache/qualified-linux/error.log) reused a
  test artifact embedding the old catalog despite updated inputs. A targeted
  `cargo clean -p afs-error` removed83 cache files/14.6MiB, followed by a complete
  fresh [gate](https://github.com/lelezi257/dms/blob/e925c5bcf0408851ebfa08a59df29953374da9e9/development/evidence/20261001-async-repair/source/gate.py). Original failure remains; no test was skipped.
- The authority subagent reported an unqualified macOS `cargo check --features
  dfs`/format attempt. It failed at the Linux-only fuser build script and is not
  validation evidence. All accepted formatting/build checks are the Linux gate.

[Catalog correction review](catalog-review.json) and
[integrated read-only review](integrated-review.json) found no scoped blocker.
Reviewer build/LSP/runtime checks were not performed; root owns Linux evidence.

## Identified A/B Linux runtime

[Runtime audit](runtime/root-independent-audit.json) verifies the actual v56
candidate on Linux A/B ext4, memory Meta, TLS/gRPC and initialization N=2/M=1.
The isolated runtime names retain `repair-v55-a/b`; they do not identify v55
binaries. [Artifact mapping](runtime/build-artifact-sha256.txt) binds original
qualified binaries to Linux debug-stripped copies. Actual `/proc` executable
hashes match the staged copies on both nodes.

1. A alone writes and fsyncs a deterministic1MiB file. Meta reports one durable
   copy and a Pending task. [Initial state](runtime/probe-a-create-underreplicated.json).
2. B joins; the source worker transfers, receives durable receipts and reports.
   Meta becomes Satisfied with two distinct available node copies and Completed
   work. [Result](runtime/wait-repair-satisfied.json).
3. B's exact local Chunk file and FUSE read match the original SHA256. Controlled
   B stop returns0; a new PID/start-tick with the same binary/config reads the
   same bytes. [Restart identity and bytes](runtime/restart-b.identity-change.json).
4. Root independently rereads live REST after restart, physical B Chunk/FUSE
   bytes, exact mounts and process/config identity. All143 compile inputs match;
   old v51 A/B processes and configs and the handoff SHA are unchanged.

Content SHA256: `3f8a853cfd1416af3ab78fd914f7574dd86045f3f132294cf9ba47717130d3a8`.
Runtime Node SHA256: `8cfea1e4a3b3df1498620f4d3b1420d0b9621fe8415bc8b7df2be838af388023`.
Runtime Meta SHA256: `f08de71b873f9e9f5564acdd09ce58ab62b62472b36db7a8b0a514c2d47849fb`.

The first runtime probe completed write/fsync but requested a REST path without
`/replication`, producing404. [Original failure](runtime/command-failure-1790816651597.json)
and full [commands](runtime/commands.jsonl) are retained. Corrected inspection
verifies the existing file (`created_this_attempt=false`); it does not rewrite
it or replace the original failure. Reproduction coordinators are copied under
`reproducers/`; their default layout is the research workspace's
`experiments/afs-acceptance/`, and product/filesystem commands run only in Linux.

## Remaining validation

These are development milestones, not release acceptance. Healthy A remains
available during B restart: this does not prove B-only serving after source loss.
Memory Meta does not establish durable Meta restart behavior. Complete
etcd/Redis, RXE repair/fault, storage/resource/shutdown, performance, deployment
and release matrices remain required. All69 formal release cases remain
NOT_RUN and environment PREPARING. `docs/handoff.md` is unchanged.
