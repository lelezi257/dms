# Current workspace metadata observation: route witness failed

**Fact, 2026-10-07:** maine903b4b2 starting anchor; product93169c8 release ELFs, exact157 compiler inputs, reused package and official runc1.5.2. No Rust/vendor/C change or rebuild. [Fixed plan](../../workspace-bind-metadata-perf.md), [maintained runner](../../acceptance/workspace-bind-metadata-perf-linux.py), [five new guards](../../acceptance/test_workspace_bind_metadata_perf.py), [ten frozen runtime tools](tool-inputs.json), [tested identities](tested-inputs.json).

**Case FAIL; ON comparison incomplete.** [Actual result](runtime/result.json), [135 driver checks:134 PASS/1 FAIL](runtime/checks.json). OFF finished one warmup/five alternating pairs. ON stopped at its first experiment warmup because the predeclared eight-callback zero criterion failed: create/write/read/rename/unlink/mkdir/rmdir0, **readdir4**, with opendir2/releasedir2/getattr16 retained. No ON reference warmup or measured pair ran; no ON ratio or performance PASS is reported. The raw successful ON C payload remains in [commands](runtime/commands.json). No threshold/exclusion changed, no score rerun or environment repair.

| OFF diagnostic phase | Five-pair speed/ext4 median | Range | Outcome |
|---|---:|---:|---|
| create/write/close |0.014085|0.010958–0.015797|FAIL, retained |
| stat |0.027669|0.024212–0.032501|FAIL, retained |
| read/close |0.018303|0.016913–0.020093|FAIL, retained |
| readdir |0.014609|0.013745–0.016639|FAIL, retained |
| rename |0.067135|0.057684–0.070530|FAIL, retained |
| unlink |0.042462|0.041649–0.044891|FAIL, retained |

All six complete OFF pairs used1000×4096B,C1,absolute paths and the pinned first-party C `/benchmark`. Its13 successful invocations (12 OFF+1 ON warmup) checked sizes/content/readdir count and errors; phases use payload wall_ns, not runc startup. Close is visibility only; durability/cache residency unqualified. Ratios are medians of five paired reference wall_ns/experiment wall_ns values, with fixed>=0.90 line. [All raw OFF ratios](runtime/off-analysis.json). Ordinary performance tuning deferred.

**Measurement limitation, not an attributed product defect:** the before/after window includes `budget()`, which recursively enumerates the whole owned tree including the FUSE root ([allocation helper](../../acceptance/orderly-runtime-checks.py)). Global callback counters therefore include measurement activities as well as the workload. This gives a concrete instrumentation confound; attribution of the four readdir calls remains unverified. Actual ON Home ext4→host workspace→ordinary OCI device/inode/mount proof passed. Do not infer that the workload itself traversed FUSE or retroactively relabel the FAIL. A future isolated callback window is a separate targeted tool task; no such corrected runtime evidence exists here.

## Independent verification and closure

[117 independent Linux checks](independent-post.json) **PASS for evidence/closure**, while `case_status=FAIL`. Collector independently read13 actual C outputs/26 metric scrapes, exact argv/input hashes, six OFF pairs/raw medians and the precise ON failure; it confirmed no unrun ON qualification. [Collector command](post.command.json). Linux13 targeted tool tests passed (five new,three affected data guards,five ordinary OCI/payload guards); this is not a Rust build or POSIX suite result.

Both OFF and ON service incarnations have actual Meta/Node wait0; all eight service/supervisor and four OCI PIDs gone, runtime empty, FUSE/bind mounts normally closed. [OFF waits](runtime/off-actual-waits.json), [failure-path ON waits](runtime/failure-actual-waits.json). Four OCI specs/rootfs manifests, ordinary501/501/no caps/nnp/read-only-rootfs, source identity and original installed binaries/protected objects were verified. Original256MiB ceiling/1GiB floor retained; recorded peak118,915,072B. No force/lazy detach, resizing or installs.

## Main and remaining work

[Main recheck](main-recheck.json): fix/native-orderly-recovery-20261007 remains fully ancestral to main; all eight preserved old worktree HEADs/patch hashes exactly unchanged, and the untracked draft/recovery copy hash matches. Completed uncommitted Node startup slice73842cd is already main. [Original full convergence and old-branch disposition](../20261007-main-convergence/README.md). Core remains single `src/node/vfs/ownerfs/bind_mount.rs`; old `src/node/native_workspace/` absent, `src/node/native_workspace.rs` retained only as legacy runc adapter/lifecycle. No new merge or PR was needed.

G1 historical8/8, major G2 counts, defaultOFF and previous data subitem PASS remain unchanged. This adds diagnostic/failed evidence only; full G2.13/ON/POSIX/new public trial remain pending. Next **DFS one-writer/many-reader**, reusing historical evidence with identity and retaining three-sync-copy/3FS qualification limits. General drain/root lifecycle, mixed append/classic locks/watch, fuser official API migration and large/complex/backends remain separately pending.

Raw and independent collector/program archives stay outside source: [indexed artifacts](external-artifacts.json). Only text outputs/commands/hashes/index copied here; no Python/C/Rust snapshots, ELF/rootfs/archive/private-key copies. [Manifest](SHA256SUMS). Normal Lore main commit/push, no per-feature review/approval gate; remote publication receipt is stored under the external raw root.

The4,858,583-byte repetitive Node trace stays in the verified external results archive: [full-log recovery index and all warning/error categories/counts](runtime/service-logs/node-log-index.json). No failed log information is discarded; Git retains results, commands, structured failures and a hash/member mapping rather than another full trace copy.
