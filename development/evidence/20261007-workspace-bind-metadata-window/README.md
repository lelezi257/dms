# Workspace metadata: corrected window and current small performance

2026-10-07. Independent G2.13 slice, [plan](../../workspace-bind-metadata-window.md).
Current product [7e6e00a6](https://github.com/lelezi257/dms/commit/7e6e00a6e2d7fdf3c1d606ee743a016a419ec25d),157-map
`151a2c6d0de361c1aa3ed2bc9d197102f59563a1ad2a98f56930c981f902c8f2`;
published c3bb ordinary package/Meta76a1e34c/Nodec47be268, officialrunc1.5.2.
[Tested inputs](tested-inputs.json), [package members](package-source-proof.json),
[exact command](runtime-r1.command.json), [raw result](runtime/result.json).
No Rust/vendor/C change, rebuild, dependency installation or environment repair.

## Tool defect and historical boundary

The old931 before/after window contained a whole-tree allocation walk through
FUSE. Only move that capacity check after the after-metrics snapshot; retain
owned-directory checks/rmdir within the window and all eight callback criteria.
[Old-runner Linux guard](old-window-regression.stderr) reproduced one FAIL;
a real benchmark readdir still rejected. [Corrected seven guards](fixed-window-guards.stderr)
PASS, including six full mock pairs and preserved failure-path cleanup. Canonical
[runner](../../acceptance/workspace-bind-metadata-perf-linux.py) and
[tests](../../acceptance/test_workspace_bind_metadata_perf.py) are maintained once.

[Original931 ON failure](../20261007-workspace-bind-metadata-perf/README.md)
remains FAIL and its ON comparison incomplete. The new run does not relabel it;
all four old readdir calls were not individually attributed. New binary/window
identities justify one new case; there was no unchanged score retry.

## Current small case PASS

One actual run,1000×4096B/C1/absolute paths, OwnerFs/local-file/gRPC, fixed test
rootfs, ordinary UID/GID501/no capabilities/nnp/read-only container rootfs.
OFF FUSE and ON physical Home ext4→real FUSE first-level workspace each have
one warmup/five alternating native-ext4 pairs. The legacy native adapter is OFF;
the host switch is enabled only for the ON test. The old pinned rootfs helper is
a test artifact, not an OwnerFs/product dependency.

The C payload verifies content/sizes/directory count. Ratios below are medians
of five reference wall_ns/experiment wall_ns values, with unchanged>=.90 line.
Close establishes visibility; cache residency/durability are unqualified.
No OCI startup time is inside the six phase timings.

| Phase | ON speed/ext4 median | ON five-pair range | Selected result | OFF diagnostic median/result |
| --- | ---: | ---: | --- | --- |
| 创建/写入/close | 0.994725 | 0.698165–1.125685 | PASS | 0.013502 / FAIL |
| stat | 0.964347 | 0.928377–1.081245 | PASS | 0.028166 / FAIL |
| 读取/close | 0.966917 | 0.936444–1.037697 | PASS | 0.018668 / FAIL |
| readdir | 1.032311 | 0.933097–1.056947 | PASS | 0.015986 / FAIL |
| rename | 0.992779 | 0.969265–1.007210 | PASS | 0.053937 / FAIL |
| unlink | 0.984438 | 0.945882–1.013557 | PASS | 0.043421 / FAIL |

[190 driver checks](runtime/checks.json) PASS. Each OFF experiment has positive
create/write/read/readdir/rename/unlink/mkdir/rmdir deltas; each ON experiment
has zero for those eight. Other ON getattr totals24 remain recorded, not called
zero or attributed to the payload. This is bounded routing/performance evidence,
not all-callback/all-POSIX qualification. All24 raw C outputs/48 scrapes and every
round/ratio remain in [commands](runtime/commands.json) and per-round files.

[Independent Linux postcheck](independent-post.json)193 PASS recomputes payloads,
callback deltas, five-pair medians/status, exact frozen runner window order,
OCI physical source/namespace/security and unchanged bind identity. Four actual
Meta/Node wait0, all eight service/supervisor and four OCI PIDs gone, normal
FUSE/bind/control closure, runtime empty and protected objects/old binaries unchanged.
[OFF waits](runtime/off-actual-waits.json), [ON waits](runtime/on-actual-waits.json).
Recorded peak116,604,928B<256MiB; final free7,583,580,160B>1GiB. Runtime elapsed
9.96s; no force/lazy unmount. OCI cleanup retains the existing owned KILL behavior,
so service orderly shutdown is not a claim of graceful container-init exit.

The4,858,673B repetitive Node log is retained byte-exact in the host archive.
[Log index](runtime/service-logs/node-log-index.json) keeps SHA/member recovery,
all warning/error categories/counts and examples: NotFound, unsupported non-user
xattrs and opcode52 warnings are present; this is not a zero-error-log claim.
[External artifacts](external-artifacts.json) bind complete archives and collector;
[SHA manifest](manifest.json) covers the compact text packet. No Python source,
ELF, rootfs, keys or chunk-data snapshots are added to Git.

## Remaining work

G1 historical8/8 stays closed; major G2 counts/defaultOFF stay unchanged. This
closes the current small metadata subcase and window-tool defect. 931 data/R3
and historical standards keep their original versions; the current7e6 data/R3,
fullG2.13/ON, general drain/mixed append/classic locks/watch, formal comparators
and broad reliability are not automatically passed. Ordinary OFF failures are
retained and focused tuning deferred. Next is the current7e6 small DFS R3
one-writer/two-reader content/copy/lifecycle compatibility exit; old931 five-round
timing stays reusable under its original identity rather than remeasured for scores.
