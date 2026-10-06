# Actual managed OwnerFs workspace — bounded lifecycle PASS

2026-10-07; official runc1.5.2, ARM64 Linux `afs-g2-micro`, guest ext4, local-file Meta/gRPC, experimental native ON only in this administrator fixture. Default OFF remains unchanged. G1 historical8/8 and G2 task counts remain unchanged. This is a closed prerequisite of G2.12; full ON semantics/production READY/revocation/drain/restart and G2.13 performance remain open.

| Identity/result | Evidence |
| --- | --- |
| Initial25a8061/map8ef8b788: real start and64MiB content PASS, stop FAIL ENOENT | [r2 result](original/results-r2/result.json), [all commands](original/results-r2/commands.json), [final mount](original/results-r2/final-identity.json) |
| Fixed source6d51aeb45c1ed8669d80f612b3817e6d1bdabe04,157inputs/map66dbbe3e | [compiler inputs](compiler-inputs.json), [reproducibility](reproducibility.json) |
| Fixed actual r4 lifecycle and cleanup PASS | [result](current/results-r4/result.json), [checks](current/results-r4/checks.json), [commands](current/results-r4/commands.json), [live installed ELF/mount](current/results-r4/running-identity.json) |
| Final container6149, namespace4/4026532347, source64769/524970, unique mount4294970713, nosuid/nodev | [FinalVerified and physical storage match](current/results-r4/final-identity.json); runtime/controller raw under current/results-r4/control |
| Host→container nonzero seed; container→host64MiB+4, sync/rename/chmod/mkdir/rmdir; fullSHA/size/501:5010600, reopen after stop | [content](current/results-r4/content.json), [work raw output](current/results-r4/control/command-0006.stdout) |
| Stop→Idle; runtime empty; original container PID gone; normal final-clone unmount, runc delete, Node/Meta exit0, host mount/controller artifacts removed | [controller stop](current/results-r4/018.stdout), [command receipts](current/results-r4/commands.json), [checks](current/results-r4/checks.json), [Node log](current/results-r4/logs/node.log) |
| Linux scoped controller7 tests, physical mount4 tests; fmt/strictClippy/release/helperTERM | [affected source receipt](source-current/affected/source-proof.json), [physical mount raw](source-current/affected/physical-native.log), [release receipt](source-current/release/source-proof.json). Ignored4 in controller filtering are explicitly run in physical gate; no double count |
| Driver identity/content guards2 tests | [Linux stdout](original/unit.stdout), [exit](original/unit.exit) |

Fixed package `afs-0.1.0-g2-procfree-6d51aeb-linux-aarch64.tar.gz`: SHA256`ee25d5892c4e884e67c86d4e5b9c6ab551af4c46649edab6a05da06659d1aff9`. Meta`2c7b7d088b759e3b9375080002182aa484a424b4fa216da1fb79a1004e96168e`; Node`2cf1f538fe7af332a711f3c66a074ace140c00773826a182709a6445b8ae2645`. Helper/runc pinned from [official runtime admission](../20261007-runc-runtime/README.md). [Installed package manifest](current/results-r4/package-manifest.json). Package/ELF/rootfs/file payload remain outside Git.

## Failure history and narrow repair

1. [r1](original/results/result.json): fixture retained DFS mount while selecting Owner-only; start rejected before workload. Maintained driver removes that conflicting config line. [Reversible original-driver delta](original-driver-to-current.patch), pinned old/new hashes in reproducibility.json; no duplicate Python snapshot.
2. [r2](original/results-r2/result.json): actual final container start/exec/content passed, stopped-container procfs could not resolve `/proc/thread-self`. [Held-FD diagnosis](original/results-r2/failure-open-fds.json), [namespace diagnosis](original/results-r2/stopped-namespace-diagnostic.json), [O_PATH fstatvfs](original/results-r2/descriptor-flags-diagnostic.json). Original FAIL/cleanupFAIL immutable. [Administrative reconciliation](original/results-r2/failed-candidate-reconciliation.json) uses stopped-state checked ordinary runc delete and processctl stop; Node exit releases its namespace, which does not prove normal final-clone detach.
3. [First new regression](source-original/affected/source-proof.json): hidden-proc behavior and rejection paths ran, but the test's own manual setns did not restore its initial cwd. Fixture restored cwd; final four tests pass. Original3PASS/1FAIL retained; distinct compiler map in compiler-inputs-first-test.json.
4. [r3 admission](current/results-r3/result.json): orchestration accidentally supplied literal PLACEHOLDER for package SHA, correctly rejected before installation/product start. This invocation error is not an environment dependency block. Correct fixed SHA and fresh r4 output/root used; no discarded or overwritten receipt.
5. Initial limactl transfer into root-owned tools directory denied; ordinary `/var/tmp` transfer plus sudo extraction succeeded before tests, described in reproducibility.json.

Product repair changes only `src/node/native_workspace/mount.rs`: check mount flags with fstatvfs on the held descriptor; successful setns enters the held namespace; normal detach relative to pinned root; restore original namespace and cwd on success/error. Source/unique mount/restrictive flags and fail-closed errors remain enforced. No vendor/public protocol/reliability-policy change.

## Reproduction and remaining checks

[Predeclared slice](../../native-workspace-runtime-slice.md), [maintained driver](../../acceptance/native-workspace-linux.py), exact argv in current/results-r4/commands.json and source-current/*/*.command.json. The outer driver CLI takes fixed package/source/Meta/Node/runc hashes, fresh owned `/opt` root and output path, canonical controller CLI and six pinned regular rootfs inputs. Used rootfs `/dev` contains runtime-created device/link entries and must not be reused as the next trusted tree; create a fresh rootfs from pinned inputs.

Unchanged historical standard/OFF/recovery results retain their version/scope. No new full pjdfstest, ordinary performance retest or full POSIX claim. Next: short cross FUSE/native locks, append/SEEK_CUR, mixed mmap/watch, permission/error checks; then64MiB/C1 OFF/ON/ext4 read/sync-write/metadata paired measurements. Full production READY and complex reliability stay open; KILL+normal detach/delete here is not graceful drain.
