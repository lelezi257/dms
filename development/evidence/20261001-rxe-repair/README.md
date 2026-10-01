# Actual RXE replica and corruption recovery

## Qualified scope

This short A/B ARM64 Linux/ext4, memory/mTLS, explicit `data_mode=rdma` / `rxe0`,
N=2/M=1 flow passes its affected integration and independent live audit.
The [v62-r3 frozen source gate](../20261001-corruption/README.md) is reused:
all 143 compile inputs and qualified stripped binary hashes are unchanged.
No Rust tests or recompilation are repeated for this orchestration-only slice.
This is stage evidence. All 69 formal cases remain NOT_RUN, environment
PREPARING; acceptance thresholds and scheduled full matrices are unchanged.

## End-to-end proof

1. [A alone](runtime/probe-a-create-underreplicated.json) fsyncs a new isolated
   1 MiB DFS file. Meta reports one available copy and repair debt.
   [B joins](runtime/wait-repair-satisfied.json), completes the repair and holds
   exact [physical and FUSE bytes](runtime/verify-b-bytes-and-fuse.json).
   [Fresh B process counters](fault/metrics-b-initial.json) record 1048576
   RDMA replica receive bytes, zero gRPC replica/read file bytes.
2. [One bad copy](fault/one-bad.complete.json): stop A, back up and mutate its
   exact physical 64 KiB Chunk without changing length or inode, then restart
   to a cold FUSE mount. The first read returns the expected checksum from B.
   [Same B incarnation](fault/metrics-b-after-cold-read.json) records a 65536-byte
   RDMA read increment. Automatic repair installs a new physical inode and
   Quarantined → Durable journal, then restores two available copies.
   [Restarted A counters](fault/metrics-a-after-auto-repair.json) record 65536
   RDMA replica receive bytes. gRPC file-payload counters remain zero.
3. [Both copies bad](fault/all-bad.complete.json): separate unique content,
   stop/mutate/restart both nodes; cold reads return EIO with zero bytes.
   Both catalogs retain Quarantined; Meta records zero available sources and
   BlockedNoSource, with `loss_confirmed=false`. No manual restore is performed.

[Raw run](fault/report.json) UUID: `0836988b-b87f-48f4-9d38-b3a5380dffd7`.
The [independent live audit](audit/report.json) checks current process/config/
mount/ext4, both healthy physical/FUSE checksums, both all-bad EIO results,
REST state and exact source/controller/handoff identity.
[Counter audit](audit/rdma-counter-audit.json) binds snapshots to PID/start ticks
across each measured incarnation. Actual counters are updated after verbs
completion in the qualified product; descriptors are excluded. Route/GID and
product verbs/repair logs are preserved for [A](audit/rdma-log-a.json) and
[B](audit/rdma-log-b.json).

[Independent proof review](proof-review.json) found no scoped blocker.

## Artifact reuse and limits

[Artifact mapping](runtime/artifact-reuse.json) binds the qualified original
hashes to stripped binaries already verified by the previous stage. Hardlinks
reuse immutable executables; new config/state/mount directories and ports
18680..18685 isolate this lane. Node SHA is
`6afabb84e1e830c0dca452557c50308b1cabc9f748fc31566fe3525fe1f3ea8f`;
Meta SHA is `d10d5683935aa70c2e8d3968be9818a83df9cb1267d1b9a29f6494a1d146865c`.

Counters represent transferred bytes and can include retries. Content,
catalog durability, Meta task state and process identity establish successful
repair. Final all-bad restarts reset process counters; the original per-step
snapshots remain bound to their own process identities.

This does not complete RDMA-02/03/04/05, REL-09 or formal backend/resource/fault
matrices. It does not measure hardware RoCE performance or prove zero-copy,
MR/QP cleanup, link/cancellation faults, durable-Meta recovery, long stability
or large-file performance. Build/runtime kernel mismatch and unfrozen
comparators keep the formal environment PREPARING. The all-bad fixture remains
intentionally quarantined. Prior gRPC failures/results retain their identities.

[Archived coordinators](probes/) require the retained research workspace
helpers and runtime templates. All product file operations, faults, metrics
and checksums execute in Linux guests; macOS only edits and orchestrates.
`docs/handoff.md` remains at SHA256
`8941901e6b6f8b8b10e057a7477695dfa9b1fc4876b5762e71b66d8930eb7216`.
