# Verified reads and corruption recovery

## Result levels

The frozen v62-r3 candidate passes the full Linux source gate and the affected
short A/B corruption flows. This is a **stage gate**, not formal acceptance.
All 69 formal cases remain NOT_RUN and the environment PREPARING.

[Validation rules](../../validation.md) select original failures, related module
regressions and necessary compilation for small changes; affected integrations
for related batches; and the full Linux gate at batch/stage boundaries. This
batch adds a Meta control RPC, so interface/feature checks are included. The
final probe-only correction reuses the same qualified Rust source and binaries;
it does not repeat the Rust gate. Acceptance thresholds are unchanged.

## Code and source qualification

- Range reads collect the requested bytes from the same full-Chunk digest pass
  and return them only after verification. A pinned descriptor preserves inode
  identity, but does not excuse later checksum failures.
- Bad local bytes enter durable `Quarantined` state. Current-path revalidation
  protects a newer healthy replacement from an old failed pin. Startup
  quarantines missing/truncated files while keeping healthy files available.
- The existing serial repair worker reports its own device and quarantine
  catalog revision. Unknown ACK/CAS preserves the exact operation; definite
  rejection keeps quarantine, logs the error and allows other repair work.
- Meta authenticates caller/session/device, excludes only matching older copies,
  and commits copy/placement/task changes with the exact request outcome.
  Replay and a fresh old-Q report cannot re-mark a newer repaired copy.
- Identity-equal repair writes a new physical inode, syncs file/directory/catalog,
  and returns a newer Durable receipt. No file version is created by repair.

[75 distinct local regressions](qualified/local/report.json) include failure
replays, Chunk7, DFS read9, replication25, Meta repair1/corruption6, data18 and
peer9. The three named replay selections are within those counts. One existing
real-RDMA environment ignore is not an RDMA PASS.

[Full Linux source gate](qualified/full/report.json) passes library395 with two
existing environmental ignores, interface contracts58, shared errors4, local
API9 and actual privileged FUSE5, plus formatting, strict workspace/all-target/
all-feature Clippy, supported feature checks and binary build.
[143 compile inputs](qualified/compile-inputs.json) match the edited source
according to [source audit](source-audit.json). [Candidate patch](candidate.patch)
is based on `982398afa8f32acec7c95303c2f89bd20a1d123e`.

[Reader review](root-reader-review.json) and [Meta authority review](meta-authority-review.json)
report no scoped blocker. Reviewers inspected source; root owns Linux tests.

## Actual Linux fault flows

[Preparation](runtime/prepare.complete.json) creates isolated A/B ext4 roots
`corrupt-v62-a/b`, ports18580..18585, separate OwnerFs/DFS mounts, memory Meta,
mTLS/gRPC and initialization N=2/M=1. [Artifact mapping](runtime/build-artifact-sha256.txt)
binds qualified binaries to the debug-stripped deployment. Runtime Node SHA:
`6afabb84e1e830c0dca452557c50308b1cabc9f748fc31566fe3525fe1f3ea8f`;
Meta SHA: `d10d5683935aa70c2e8d3968be9818a83df9cb1267d1b9a29f6494a1d146865c`.

1. [One corrupted copy](fault-r2/one-bad.complete.json): create a fresh exclusive
   64KiB file, fsync and wait for two copies. Stop only Node A, mutate its exact
   physical Chunk without changing length/inode, and restart to a fresh mount.
   The first cold A read returns the original checksum from B. Automatic repair
   restores the physical checksum through a new inode and Durable → Quarantined
   → Durable journal with a newer receipt. Only then is restored Meta health
   accepted. No manual restoration occurs in this successful run.
2. [All corrupted copies](fault-r2/all-bad.complete.json): use a distinct fresh
   payload; stop both Nodes, mutate both exact copies and restart fresh mounts.
   The first cold read returns EIO with no returned bytes. Both journals retain
   Quarantined state. Meta has zero available copies and BlockedNoSource;
   `loss_confirmed` remains false. The committed data is not treated as a hole.

[Fresh run](fault-r2/report.json) UUID is `dba81f2a-c1ad-4918-a2ac-ec76fe62dab4`.
[Independent live audit](audit/report.json) checks both current PID/start ticks,
executables/config/mounts, controller hashes, healthy physical/FUSE bytes,
all-bad EIO on both mounts, live REST and source/handoff identity.
[Proof review](proof-review.json) confirms the scoped claim without running tests.

## Retained failures and environment observations

- [Original warm reader](original-reader/) returned damaged bytes; original cold
  fallback already passed. [Original cataloged-copy retry](original-retry/report.json)
  fails digest verification instead of replacing the damaged file.
- [v61 local slice](reader-local/report.json) is valid only for its own61-test
  scope and source identity; it does not qualify the later Meta RPC.
- [v62-r2 selection audit](failed-local-v62-r2/local-selection-audit.json) marks
  the two zero-test Meta selections INCONCLUSIVE despite their mechanical exit0.
  [Corrected module](failed-local-v62-r2/local-meta-corrected/report.json) then
  finds one new-test assertion failure: historical CopyIDs were wrongly counted
  as exactly two. The final test checks preserved records and effective live
  sources separately. Raw logs and that original Meta source are retained.
- [Original runtime attempt](fault-original/report.json) remains FAIL. Its cold
  fallback succeeds, but its repair check immediately sees the previous
  Completed task. [Analysis](fault-original/failure-analysis.json) records this
  observation race; the fresh r2 requires physical repair evidence first.
- [Original short lease observation](session-original/) remains FAIL after only
  five samples. [Unchanged-v60 awake observation](session-r2/awake-check.complete.json)
  covers eight samples over at least70s with a host sleep-prevention assertion.
  Host events overlap earlier expiry, but exclusive causation is unproven.
  This does not establish v62 liveness, a product lease fix or a stability gate.

## Limits and reproduction

The build kernel6.8.0-106 and runtime kernel6.8.0-142 are ARM64 Linux; their
mismatch keeps the formal environment PREPARING. This short memory/gRPC stage
has no durable-Meta, actual RXE corruption/repair, full REL-09, resource/long-run,
performance or release claim. Whole-Chunk scanning per nonempty range is a
known correctness cost. The new Node RPC requires Meta upgraded first.

[Archived probes](probes/) preserve exact orchestrator/runner sources and raw
commands. They require the research workspace layout and retained Linux
runtime templates; they are not the product installer. Build/fault operations
run inside Linux guests on ext4. No compilation or product test runs on macOS.

The handoff document stays unchanged at SHA256
`8941901e6b6f8b8b10e057a7477695dfa9b1fc4876b5762e71b66d8930eb7216`.
