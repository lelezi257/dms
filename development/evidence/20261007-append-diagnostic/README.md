# Mixed append diagnostic — data intact in this scope; offset FAIL retained

2026-10-07, ARM64 Linux `afs-g2-micro`, Ubuntu kernel6.8.0-106-generic
(6.8.0-106.106 / 6.8.12), guest ext4, local-file Meta/gRPC, official runc1.5.2.
Same Rust6d51aeb/map66dbbe3e/157 inputs and release package as the earlier
[managed lifecycle](../20261007-managed-workspace/README.md). Maintained Python
changes only; no Rust/vendor/cache/append-flag change or rebuild. Default OFF;
G1 historical8/8 and G2 counts unchanged.

| Independently checked result | Native/native control | Mixed FUSE/native | Evidence |
| --- | --- | --- | --- |
| Four sequential append offsets | PASS:12,26,38,52 | FAIL:12,26,24,52; third expected38 | [mixed offsets](results-r8/semantics/append/append-offsets.json) |
| Sequential fsync/close and both fresh reads | PASS | PASS:52 exact bytes/SHA on both paths | [mixed proof](results-r8/semantics/append/append-proof.json) |
| Concurrent one-write records, two roles×64 | PASS | PASS:128 unique, no missing/extra/duplicate, both fresh views equal | [mixed proof](results-r8/semantics/append/append-proof.json) |
| Per-record SEEK_CUR versus actual final byte end | PASS:all128 | FAIL:62 mismatches | [all child offsets](results-r8/semantics/append/append-concurrent-children.json) |
| Aggregate selected append semantics | PASS | **FAIL** | [control](results-r8/semantics-reference/result.json), [mixed](results-r8/semantics/result.json) |
| Normal stop/unmount/delete | — | PASS, despite selected FAIL | [result](results-r8/result.json), [checks](results-r8/checks.json), [independent postcheck](postcheck.json) |

This bounded result distinguishes content integrity from offset correctness. It
observes no lost/duplicate records in this run; it does not prove all schedules,
large append, complete close-to-open, full POSIX, production ON or G2.12/13.
The original [r6 failure](../20261007-managed-semantics/current/results-r6/semantics/append/step.json)
remains unchanged: it stopped before content/concurrent checks. The new checks
complete those independent observations while retaining the original offset
predicate. Original lock/watch FAIL remain separate and were not rerun.

## Execution and identity

[Predeclared slice](../../native-workspace-append-slice.md), [outer argv](driver.command.json),
[actual exit1](driver.exit.json), [raw stdout](driver.stdout), [raw stderr](driver.stderr),
[inner commands](results-r8/commands.json), [preflight](results-r8/preflight.json),
[tool digests](tool-inputs.json), [source map reference](source-map-reference.json),
[exact kernel build](kernel.stdout). `--semantics-groups append --semantics-only`
selects append only; `basic_payload_selected:false` and actual commands omit the
unchanged64MiB basic workload. Necessary install/identity/admission/cleanup remain.
A clean rootfs is built from six pinned regular template files, not a copied live
rootfs. Product ELF/package/rootfs/private installation key remain outside Git.

Package SHA256 `ee25d5892c4e884e67c86d4e5b9c6ab551af4c46649edab6a05da06659d1aff9`;
Meta `2c7b7d088b759e3b9375080002182aa484a424b4fa216da1fb79a1004e96168e`;
Node `2cf1f538fe7af332a711f3c66a074ace140c00773826a182709a6445b8ae2645`;
runc `d10ecae898361832a059be2089bab92d158aec54661b18ed7346ed79628b46b0`.
The postcheck proves no selected AFS/runc processes, exact host mount absent,
runtime list empty and controller socket/lock absent. No force/lazy cleanup.

## Tool verification and recovery

[Original Linux8-method run](original-linux-guards.stderr) has7 PASS/1 fixture
KeyError, before any product run: the stricter correlation checker required128
records but the old negative fixture supplied two. The helper stayed strict;
the fixture now supplies128 complete records, proves positive correlation then
perturbs one offset. [Only the failed method rerun](linux-guard-fixture-r2.stderr)
PASS; the unchanged other seven results are reused. This establishes8 unique
methods, not9 or a POSIX suite. [Original command/exit](original-linux-guards.exit.json),
[rerun command/exit](linux-guard-fixture-r2.exit.json), [version recovery](tool-recovery.json).
Canonical maintained tools are [driver](../../acceptance/native-workspace-linux.py),
[probe](../../acceptance/probes/native_mixed.py), [guards](../../acceptance/test_native_workspace.py).
Git provenance and reversible patches preserve all tool identities without full
Python snapshots. Review is recorded in `final-review.json`; all portable file
hashes are listed in `SHA256SUMS`.

## Mechanism assessment and next independent item

**Code facts:** eligible OwnerFs already usesTTL0; physical opens retainO_APPEND,
FUSE reply reports write count only; no userspace lseek override currently exists.
**Inference:** Linux's direct-write offset bookkeeping explains why the physical
append is correct while the FUSE descriptor offset is stale. Upstream
[Linuxv6.8 FUSE file code](https://raw.githubusercontent.com/torvalds/linux/v6.8/fs/fuse/file.c)
uses generic_file_llseek for SEEK_CUR, so adding a userspace lseek callback alone
cannot fix this observation. That tag corroborates the inference; it is not a
review of the exact patched Ubuntu build. Do not enable writeback as a workaround:
[upstream writeback assumptions](https://kernel.org/doc/html/latest/filesystems/fuse/fuse-io.html)
require changes to pass through FUSE, unlike these mixed external native writes.

This diagnostic is closed with a necessary ON offset-coherence gap retained.
It is a product/API limitation, not an environment blocker. Keep bind OFF,
preserve lock/watch failures and performance data, and proceed with the current
OFF trial-package/selected installation-recovery receipt as an independent
G2.27 branch. No complete G2.27 performance gate or all-table completion is claimed.
