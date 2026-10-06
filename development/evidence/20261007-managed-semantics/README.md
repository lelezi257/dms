# Managed workspace mixed-path semantics — FAIL retained

2026-10-07, ARM64 Linux `afs-g2-micro`, local-file Meta/gRPC, official runc1.5.2, guest ext4. Same Rust source6d51aeb/map66dbbe3e/157 inputs and package as the [bounded lifecycle PASS](../20261007-managed-workspace/README.md); only maintained acceptance Python changes. Default OFF; no full G2.12/G2.13 or production ON claim. G1 historical8/8 and stage counts unchanged.

| Current bounded result | Evidence and limit |
| --- | --- |
| 4KiB mmap bidirectional bytes PASS | [Independent proof](current/results-r6/semantics/mmap_inotify/mmap-proof.json); native MAP_SHARED→fresh FUSE read and FUSE pwrite/fsync→native map; not general mmap qualification |
| Permissions/errors PASS | [Step](current/results-r6/semantics/permissions_errno/step.json), [actual controller receipts](current/results-r6/semantics/permissions_errno/control-artifacts.json); UID501 read and write denied for root0600, hash/mode unchanged, ENOENT/EEXIST |
| Mixed classic locks FAIL | [r6 report](current/results-r6/semantics/locks/locks-smoke/report.json); byte-range exclusive/exclusive, shared/exclusive and BSD exclusive conflict unexpectedly succeed. Legacy blocking-waiter PASS labels did not prove no early ACQUIRED; retain raw labels without counting those as qualified blocking checks |
| O_APPEND/SEEK_CUR FAIL | [Step](current/results-r6/semantics/append/step.json): third write expected EOF38, actual current offset24. Stops before mixed concurrent append, which is NOT_RUN; no inferred concurrent-data result |
| Cross-path watch FAIL | [Step](current/results-r6/semantics/mmap_inotify/step.json), [raw events](current/results-r6/semantics/mmap_inotify/inotify-events.json): primary FUSE watcher misses native-origin CREATE. Both watchers must observe both origins; own-path events are insufficient |
| Actual failed run cleans normally | [Result](current/results-r6/result.json), [checks](current/results-r6/checks.json), [commands](current/results-r6/commands.json), [independent postcheck](current/postcheck-r6.json); normal stop/unmount/delete and Node/Meta exit, no force/lazy cleanup |

Native/native r6 [positive control](current/results-r6/semantics-reference/result.json) reports4/4; its append, mmap/watch and permissions are valid small controls. The legacy blocking-lock limitation above also applies to this control. The bounded lock predicate correction and affected-only qualification are recorded below; no unchanged append/watch/permission or full standard retest.

## Affected-only corrected lock qualification

The maintained lock tool now uses fd-level bytes buffering and a monotonic deadline, rejects unsupported errno as conflict proof, and requires no early ACQUIRED event before unlock. [Final Linux guards](current/semantic-qualification-lock-final/) pass4 identity/permission/procfd methods plus6 lock negative guards; this is10 unique methods, not an additional full POSIX run.

[r7 exact outer command](current/results-r7/driver.command.json) selects only `--semantics-groups locks`. Native/native [8-step control](current/results-r7/semantics-reference/locks/locks-smoke/report.json) PASS; mixed [8-step report](current/results-r7/semantics/locks/locks-smoke/report.json) FAIL with5 failures: the same three conflicts plus both blocking waiters emitting ACQUIRED before unlock. The remaining3 raw PASS labels include fixture identity and release observations; they do not establish complete mixed lock coordination. Append, mmap/watch and permissions are unselected and retain r6 identity/results. Aggregate [FAIL with normal cleanup](current/results-r7/result.json), [checks](current/results-r7/checks.json), [independent postcheck](current/postcheck-r7.json). The lock [tool digest](current/results-r7/semantics/locks/locks-tool.json) is recorded before the failing command; no Rust rebuild or semantic threshold change.

## Identity and reproduction

[Source map reference](source-map-reference.json) links the original compiler manifest rather than duplicating157 inputs. Package SHA256`ee25d5892c4e884e67c86d4e5b9c6ab551af4c46649edab6a05da06659d1aff9`; Meta`2c7b7d088b759e3b9375080002182aa484a424b4fa216da1fb79a1004e96168e`; Node`2cf1f538fe7af332a711f3c66a074ace140c00773826a182709a6445b8ae2645`; runc`d10ecae898361832a059be2089bab92d158aec54661b18ed7346ed79628b46b0`. Exact argv/tool hashes/preflight/live process/mount identities in [r6 outer command](current/results-r6/driver.command.json), [tools](current/results-r6/tool-inputs.json), [preflight](current/results-r6/preflight.json), [final identity](current/results-r6/final-identity.json).

[Predeclared criteria](../../native-workspace-semantics-slice.md), maintained [driver](../../acceptance/native-workspace-linux.py), [probe](../../acceptance/probes/native_mixed.py) and [lock probe](../../acceptance/probes/locks_smoke.py). Every fixture builds a clean rootfs from the fixed six regular inputs. Product binaries, rootfs, payloads and private installation key remain outside Git.

## Original evidence and tool repairs

- [r5 original result](original/results-r5/result.json) retains both paths, all commands and raw failures. It reports mixed2/4, but its watch checks only own-origin events and cannot prove cross-path propagation. Its permission collector duplicated artifact stems; raw output has two real denials, not six independent checks.
- [r6](current/results-r6/result.json) strengthens both-origin watch, records mmap independently, fixes collector deduplication and removes an unnecessary generated wrapper after preserving procfs magic paths in the maintained lock probe. Aggregate FAIL remains FAIL; no discarded measurements.
- [Linux evidence guards](current/semantic-qualification-final/) record4 passing methods, including a negative guard against multiplying permission errors and Linux procfd path preservation.
- [Recovery map](tool-recovery.json) binds original/intermediate tool hashes and reversible deltas. Generated wrapper bytes are recoverable through AST literal extraction, verified before removal; originals remain outside Git/guest. No complete repeated Python snapshot is stored here.

## Next bounded output

Retain lock/append/watch FAIL as distinct native ON gaps. The next performance output is explicitly a small diagnostic for container workspace access; it cannot qualify usable ON/G2.13 while necessary semantic gaps remain. Ordinary baseline data and FAIL are retained without tuning; full production READY, drain/restart/revocation, large cases, long reliability, etcd and Redis remain separate later items. Historical standards/recovery and G1 conclusions are unchanged.
