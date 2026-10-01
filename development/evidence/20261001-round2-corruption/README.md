# Current-candidate corruption and RXE recovery

**Scoped fault integration PASS. Round2 remains incomplete.** Formal cases remain
69 NOT_RUN and ENV PREPARING. No product Rust, interface, protocol, persistent
format, dependency or architecture change is made by this batch.

## Candidate and topology

The installed candidate is the [Owner EIO source-gate candidate](../20261001-owner-sync-eio/README.md),
Node `a3fe6573fc5f5a41c30b823855cfe2fdd1428950f878f1e29756e7bfc0d7e9d9`,
Meta `895f39fd660b7f7d9735eaaa4f3a082c3692a409d954c4aa8b5b0cc515a700ad`.
Existing installed process controller is reused. Central memory Meta runs on
ctl:20380; A:20382 and B:20384 use separate OwnerFs and DFS mounts, mTLS and
**required RDMA on actual rxe0**. Initialization keeps desired2/synchronous1 and
local-required. Each test file is fsynced and has two actual available copies
before damage; this does not claim two synchronous copies at initial fsync.
State is on the locked ARM64 guest ext4 volumes. Builds and tests run in Linux.

Live configuration/mount snapshots for [Meta](ctl/evidence/binding-final.json),
[A](a/evidence/binding-final.json) and [B](b/evidence/binding-final.json) prove the
actual executable, config path, readiness and independent FUSE mounts. Meta
identity remains unchanged across all Node restarts. The snapshots precede the
last normal A/B restart; subsequent restart/read records bind the new incarnations
to the same candidate binaries. Guest inputs and original
probe variants are archived under each evidence/inputs directory; no TLS private
keys are included.

## Single bad copy

1. [Create](a/evidence/create-one.json) an exclusive 64 KiB file, complete both
   sync calls and exact read/length/EOF; [Meta](ctl/evidence/replicas-one.json)
   confirms two copies and Completed repair.
2. Stop A only, [mutate](a/evidence/inject-one.json) byte8192 of its exact physical
   Chunk, preserve length and inode, sync the damage and retain a backup. B and
   central Meta remain alive.
3. Start a fresh A process/mount. [First cold read](a/evidence/cold-one.json)
   returns exact acknowledged bytes. [B counters](b/evidence/metrics-one-read.json)
   increase actual RDMA read payload by65536 with zero gRPC file payload.
4. [Physical repair](a/evidence/repaired-one.json) restores the checksum through
   a new physical inode and Durable → Quarantined → Durable catalog revisions.
   [A counters](a/evidence/metrics-one-repaired.json) show65536 RDMA replica bytes.
   Only after physical replacement does [Meta health](ctl/evidence/repaired-one-rest.json)
   qualify two effective copies and Completed repair. No backup is manually restored.

## All copies bad

A distinct [file](a/evidence/create-all.json) and Chunk prevent reuse of the
repaired content. Both Nodes stop, and their exact physical copies are mutated
and synced separately ([A](a/evidence/inject-all.json), [B](b/evidence/inject-all.json)).
Fresh mounts return EIO with zero bytes ([A](a/evidence/cold-all.json),
[B](b/evidence/cold-all.json)); both physical journals end in Quarantined.
[Confirmed Meta report](ctl/evidence/blocked-all-rest-confirmed.json) has zero
available copies, BlockedNoSource and loss_confirmed=false. Committed data is
never silently returned as a hole.

After another normal A/B restart the bad file still returns EIO/zero bytes, while
the unaffected repaired file remains exact ([A](a/evidence/healthy-after-quarantine-restart.json),
[B](b/evidence/healthy-after-quarantine-restart.json)). A blocked repair does not
prevent these healthy reads.

[The first Meta poll](ctl/evidence/blocked-all-rest.json) observed stale Satisfied
health before asynchronous corruption reports. The first worker accepted either
healthy or blocked state, so that record **does not qualify blocked-state proof**.
The corrected probe requires an explicit expected state; the subsequent confirmed
record qualifies zero copies/BlockedNoSource. The original observation and input
remain unchanged. This is a probe selection defect, not evidence of bad-byte
success or a product consistency change.

## Verification and cleanup

[Linux semantic audit](audit.json) checks49 conditions and rejects six altered
negative variants. All143 compiler inputs match the executed Owner EIO candidate;
its 421 library/65 contract/4error/9LocalAPI/5privilegedFUSE, Clippy/features/build
source gate is reused because only probe/docs change. No repeated Rust gate ran.

That current gate also ran exact post-handler reply-loss tests for FileVersion and
repair claims/reports. They prove pending identity, same-inode mutation barriers,
other-inode progress and exact replay within their **memory/plain-loopback/R1
fixture scope**. They do not establish installed cross-VM Meta reply loss, durable
backend recovery or a transport fault matrix.

A/B/Meta stop normally. [A cleanup](a/evidence/cleanup.json),
[B cleanup](b/evidence/cleanup.json) and [Meta cleanup](ctl/evidence/cleanup.json)
verify stopped processes, absent test mounts, more than4GiB available guest data
space and the older round1 cohort still healthy. Damaged chunks, original backups,
catalogs and logs remain inspectable; no temporary network rule or device fault
was used. Actual RXE transfers are proved; physical RXE link interruption, posted
DFS deadline/native CQ timeout, sustained resources and full backend/device/fault
matrices remain unqualified. Performance, full POSIX,8GiB and soak remain later
rounds. AGENTS and handoff are unchanged.

[Publication audit](verification.json) checks compiler/protected-file identity,
local navigation and artifact hashes. [Independent read-only review](review.md)
finds no scoped blocker; it does not supply independent dynamic test evidence.
