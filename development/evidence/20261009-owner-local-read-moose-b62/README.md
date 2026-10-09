# G2.09 — one current b62 / MooseFS local READ comparison

**Limited function PASS; measurement COMPLETE; selected-case performance FAIL.** Both throughput and independent p95 fail the unchanged 1.2/.8 conjunction. This closes the one measurement attempt, not G2.09 as a whole. G1 historical8/8, original G2 counts12/0/15 and the published d47 ON trial stay unchanged. No product change or new package was produced.

| Metric | OwnerFs b62 | MooseFS 4.59.2-1/build2106 | Owner/Moose |
| --- | --- | --- | --- |
| Median MiB/s | 5706.058291 | 19695.507455 | 0.289714 — FAIL |
| p50 ms | 0.142544 | 0.027375 | reported |
| Preset p95 ms | 0.226087 | 0.035667 | 6.338829 — FAIL |
| p99 ms | 0.520050 | 0.087752 | reported |

## Frozen scope and limitations

One formal run on existing ARM64 Linux B/ext4, kernel6.8.0-142-generic; existing stopped local-file/R1 combined fixture, bind OFF. Main f57bacc7 has all161 compiler inputs identical to retained product53e/map47de5b82. Node b62ade56 and Meta650bd971 reuse their accepted Linux release builds. Original Owner64MiB inode and sole local VALID Moose chunk are reused, never recreated. Their payload SHA remains fae972222d455a2eaee1661ad9625502ec3bfc5ec38b87a6eec5afd5107331b5.

Same c87 probe, FUSE/POSIX/O_RDONLY/pread,1MiB,C1, pattern97. One warmup each; five formal pairs alternate Moose→Owner, Owner→Moose, Moose→Owner, Owner→Moose, Moose→Owner. Each side records320 actual `pread+count-check` intervals. Content oracle is outside operation latency; whole wall throughput includes application/open/thread/oracle/close. Median five wall rates and pooled nearest-rank p95 were fixed before launch; p50/p99 and all individual intervals remain available. Owner formal4 slower3443.856741MiB/s is retained.

Full mounted-file and physical-payload reads preload each sample. Physical payload mincore snapshots are fully resident before/after all12 samples. Client residency is observed separately: Owner0/0 and Moose64MiB/64MiB each sample. These are different default implementation policies, explicitly frozen and disclosed; this compares their actual default behavior and does not establish equal internal caches, pure backing-filesystem performance, or all cache regimes. No DIRECT/drop_caches tuning. The separate prior d14→b62 retention pair keeps its own cache preparation/order; do not infer before/after improvement from its numbers and this comparator.

Both applications run as root; mounted and physical modes/owners are recorded. This is not a new ordinary-UID permission suite. Existing preparation barriers are reused; no new write-durability claim is made.

## Verification and failures retained

Full mounted SHA, EOF,17-byte tail returned from32-byte request, and zero-length reads pass before/after on both sides. Five actual child waits are0. Original physical files, six AFS/Moose configs, fixed assets and official stock installation retain inode/SHA/mode/owner/time identity; original run/log directories, mounts and protected-process inventory close. Local-file service metadata may advance normally.

Linux validation: three behavior guards (registered child survives identity-check failure in mocked Popen; bad independent runner pin starts no Runner/child; actual ORDER/nearest-rank boundary) plus three source guards pass; py_compile passes. These are tool checks, not product acceptance. Parent independent Linux saved-data audit verifies66 assertions for raw arithmetic, full identities, read checks, five waits and gone owned PIDs. No Rust rebuild/full standard rerun: the161 compiler inputs match the accepted build exactly. Its prior strict Clippy dead_code failure remains disclosed.

Four preparation failures occurred before product launch: host-only input path; ldd incorrectly applied to a script; official CLI aliases treated as nonregular; stopped fixture incorrectly required an Owner mount. All raw failures remain. r5/r6 passed44/64 checks; the final pinned r7 passed88. Parent rejected the initial launch path before services started; the final runner uses root-owned fixed-SHA isolated official copies. Product/formal measurement ran once, not once per preparation check. No environment rebuilding or official-source change occurred.

## Preservation and space

[Exact compact result and index](result.json) binds every version, threshold, interval aggregate and recovery receipt. Full scripts, ELFs, raw arrays/logs and all preparation failures remain outside source Git at `evidence/afs-delivery/owner-local-read-moose-b62-20261009-r1/` under the research project.

Canonical archive: `preservation/owner-local-read-moose-b62-preservation-linux-20261009120657.tar`,54,026,240B; SHA256 `d1e4d2286521407650121f85a7f69eda164559762cce506df64d17b61c5ea93b`. Actual Linux extraction verified155 files/53,853,633 payload bytes by SHA before cleanup; this is not merely tar listing. Archive metadata restoration beyond the documented checks is not claimed. Original fixture identity checks are separate and passed. Reproduction uses the exact archived runner/contract, launch command, official binary hashes and probe; do not reuse the consumed run directory.

Cleanup removed only this-run guest assets and temporary build/archive/restore scratch:341 files/216,100,864 allocated bytes total, including14 asset files/53,145,600 allocated bytes. These totals include temporary archive copies, not216MB of historical data. Formal results/logs and original fixture remain. B free21,245,726,720B; no fixture mount/protected process remains; scoped logs ERROR/ERRO/WARN/TRACE all0. Formal5 allocated289,062,912B<512MiB and free21,192,654,848B>4GiB are samples, not continuous peak proof.

## Next independent item

Stop this local comparison; no local tuning/matrix or repack follows. Return to bind/remote. A single short remote READ CPU profile was selected to identify a dominant source cost before another product change, but its dependency check stopped before launch: C's `/usr/bin/perf` is only a wrapper and the kernel6.8.0-142 perf executable is missing (exit2). Blocker output is preserved; no install/service/profile was attempted. User input is pending only for this environment dependency, while this completed result is published independently. Existing remote DIRECT throughputFAIL/p95PASS, strong-write-ACK/3FS topics, R2 migration and user-deferred A/ctl/DFS-write scope remain unchanged.
