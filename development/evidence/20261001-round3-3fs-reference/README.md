# Round3 patched 3FS normal reference qualification

This packet proves a bounded normal-flow reference on the locked ARM64 Linux ctl/A/B/C guests. It does not qualify a fair performance comparison, power-loss durability, bounded recovery, or formal acceptance. **Formal69 NOT_RUN / ENV PREPARING; ROUND3 continues.**

## Identity and topology

- 3FS source `22fca04564c7cc230fd8b9523b8b92864e1dad47`, with the recorded ARM64 Folly StampedPtr logical-shift patch, SHA256 `6c460875c6c098e0b4aacad309e099657a5daf70bdc3937060d402a5e59aad95`. This is a patched reference, not an unmodified upstream binary.
- Each guest uses `/opt/afs-3fs-round3-v84`, with 39 exact executable/library hashes checked before use. [Prefix manifest](preparation/prefix-manifest.json) and per-action prerequisite records bind binaries, templates and volumes; owned dependency paths resolve without missing libraries.
- ctl runs FDB7.3.63 on19000, mgmtd19001 and Meta19002. A/B/C run storage19003, one three-target chain and table1, 512KiB chunks, stripe1. Actual FDB status reports `ssd-2` storage and log engines, single redundancy and available database. Its explicit1GiB process limit and128MiB cache/storage budgets are small-fixture configuration, not frozen fair-performance defaults.
- Data and state are on each guest's declared ext4 data volume. Unrelated AFS/backend services remain untouched; their presence prevents an isolation claim. Role archives retain configuration, process receipts, raw command output and logs.

## Completed scope

| Check | Evidence and result |
| --- | --- |
| Actual FUSE write/sync | [A write](a/v84-write-r2.json):32 exact1MiB writes; fdatasync/file fsync/dir fsync return successfully. Full32MiB content SHA256 `626f47ade3da8112941c844a3a1d8a02b94c5e134a9c3391a3375aa22cd1dbc7` |
| A/B reads | [A](a/v84-read.json), [B](b/v84-read.json): full content/EOF and64 fixed4KiB/64KiB ranges each match |
| Physical replicas | [Before plan](preparation/physical-plan-before.json), A/B/C physical-before records:64 chunk identities, each with three committed/up-to-date/serving targets;192 exact indexed physical slots read directly |
| Full normal restart | Stop both FUSE clients, all three Storage processes and ctl Meta/mgmtd/FDB; restart retained state. [Post-restart plan](preparation/physical-plan-after-r3.json) and three physical-after records again prove192 exact slots. [Fresh B mount read](b/v84-read-after-restart.json) matches32MiB and64 ranges |
| Normal final stop | Each role's `v84-stop-final.json` records matching executable/start-ticks identity and termination without forced kill. Stopped resource snapshots show no owned requested processes or their user verbs resources; other cohorts can retain theirs |
| Probe feedback | [Final Linux regression](ctl/v84-driver-tests-r3.log):9/9 tests pass, [actual exit0](ctl/v84-driver-tests-r3.exit). Earlier failed fixture runs remain separate |

The inode is `0x12801`. Distinct physical `(path, offset)` slots are checked, as well as bytes; the deterministic pattern repeats within each1MiB block and a digest count alone would be insufficient. The query and pread proof is not a hardware flush proof. Resource-after snapshots record roughly1.6GiB Storage RSS per node, within these guests but neither optimized nor a sustained limit result.

## Failures retained and limits

1. Initial FDB launch rejected an underscore-containing cluster key. The corrected key is alphanumeric; 3FS cluster identity is unchanged. Initial failed launch output remains in the ctl archive.
2. Initial A/B FUSE launch inherited an inaccessible ctl log path and failed. Explicit per-guest startup `--cfg` fixes mounting; subsequent remote configuration warnings still reference the central path. Configuration update behavior needs qualification. The first write failed before a mount existed; final helpers require an owned FUSE mount and process identity.
3. The first physical query could not parse `512KB`. Strict Size parsing was corrected; original output is retained. Initial stop encountered a process-exit race after unmount; the helper now tolerates an already-exited owned PID and records final signals.
4. After the full restart an admin stat exceeded its60-second command timeout. The following query observed transient `SYNCING` and failed. Original empty/failed outputs and raw logs remain preserved; a later unchanged retry reached all targets serving/up-to-date. Fresh B eventually read exact content. **No bounded restart time or uninterrupted availability is established.** The timeout helper failed before writing a command receipt, so an absent receipt cannot supply an invented exit result.
5. Helper regression r1 had one stale-key expectation failure; r2 had two fixture errors. r3 passes9 tests. Earlier outputs are not relabeled.
6. No baseline throughput ratio is qualified. Resource isolation, fair configuration and acknowledgment durability remain incomplete. Do not use anomalous internal duration logs as benchmark timing.

## Durability contract remains BLOCKED

Normal sync return and retained-state restart are useful proof with a limited failure model. In this 3FS source, aligned writes use a separate `O_DIRECT` descriptor without `O_SYNC` (`GlobalFileStore.cc`, `ChunkFileView.cc`); chunk commit synchronizes LevelDB metadata by default, without an explicit data-file barrier in `ChunkReplica.cc`. A synchronous WAL on the same ext4/device can indirectly order completed data writes, so absence of an explicit data fsync alone does not prove data loss. Qualification still requires the actual data/WAL placement, kernel/VM flush propagation, and data-write → durable barrier → commit/ACK ordering. An external sync after the ACK cannot prove that original contract.

The `fsyncdir` callback is commented out in `FuseOps.cc`; [libfuse documents that ENOSYS is converted to success for this operation](https://libfuse.github.io/doxygen/structfuse__lowlevel__ops.html). The raw helper's `directory_fsync_qualification_gap=false` means only that the syscall returned successfully; **it does not qualify a directory barrier**. Namespace operations await FDB transaction commit, so the missing callback alone also does not establish namespace data loss. See [kernel flush ordering](https://docs.kernel.org/block/writeback_cache_control.html). Strong durable comparison stays BLOCKED alongside MooseFS B001; read/reference and independent resource/backend work may continue.

## Reproduction and audit

[Lifecycle helper](https://github.com/lelezi257/dms/blob/e925c5bcf0408851ebfa08a59df29953374da9e9/development/evidence/20261001-round3-3fs-reference/probes/round3-3fs.py), [physical query/pread helper](https://github.com/lelezi257/dms/blob/e925c5bcf0408851ebfa08a59df29953374da9e9/development/evidence/20261001-round3-3fs-reference/probes/round3-3fs-physical.py), [tests](https://github.com/lelezi257/dms/blob/e925c5bcf0408851ebfa08a59df29953374da9e9/development/evidence/20261001-round3-3fs-reference/probes/test_round3_3fs.py) and [Linux packet audit](https://github.com/lelezi257/dms/blob/e925c5bcf0408851ebfa08a59df29953374da9e9/development/evidence/20261001-round3-3fs-reference/probes/audit.py) are retained. These helpers are preparation fixtures, not formal case drivers. Run with the recorded owned prefix/templates and roles; preserve failed attempts and avoid replacing retained data. Per-role `runtime-raw.tar.gz` contains guest-generated logs/config/evidence/run records, not copied data files or binaries. Original volumes retain those files.

[Audit](audit.json) checks semantic content, exact physical slots, final process cleanup, raw archive coverage, unchanged143 compiler inputs and protected files. [Artifact hashes](artifact-hashes.json) bind this packet. This evidence-only batch reuses the unchanged [round2 Linux source gate](../20261001-round2-closure/README.md); no Rust gate is rerun or reattributed to changed code. Historical inactive extraction relocation retains exact content/metadata manifests and original-path symlinks in preparation records; tested data and active services were not relocated.

The audit's first attempt rejected the collector's explicit `NO_PROCESS_TARGETS` label; the corrected audit checks that label together with exact owned stop receipts. The original [audit failure](ctl/packet-audit-r1.log) is retained. [Four Linux negative checks](ctl/packet-audit-tests.log) reject changed restart content, false physical-byte matches, forced cleanup and failed unmount; [exit0](ctl/packet-audit-tests.exit) records the test result. The raw no-target snapshot alone is not a process-exit proof.
