# DFS small delete — 2026-10-07

**Current product6d51aeb /157 inputs/map66dbbe3e**, fixed release Meta/Node ELF; DFS-only/local-file Meta/gRPC/R2/native OFF on four ARM64 Linux VMs, guest ext4. [Summary](summary.json), [fixed inputs and canonical Git](inputs.json), [actual final receipt](worker-execution-receipt-r1.json).

The bounded candidate functionality and quantitative collection are complete. **Formal G2.25 remains pending a qualified same-case3FS comparison.** There is no new delete-ratio threshold; candidate throughput is not a parity result. G1 stays8/8 and G2 counts stay10 limited completed/2 ordinary performanceFAIL/2bind inprogress/13pending.

Each of six samples creates100 distinct4KiB files with O_EXCL, per-file fdatasync/close and directory fsync, then freshly verifies full contents/EOF. One warmup and five measured samples run once. Only the100 unlink calls and Python loop lie inside each Linux monotonic timer; preparation, validation, directory fsync, namespace checks and rmdir are outside. Cache is UNOBSERVED. [Original quantitative snapshot](delete-quantitative-r1.json) preserves its checker guard gap and measurement-time closure PENDING.

| Measured round | Unlink seconds | Operations/second |
| --- | ---: | ---: |
| 1 | 1.930810 | 51.791747 |
| 2 | 2.641363 | 37.859243 |
| 3 | 3.348827 | 29.861202 |
| 4 | 3.838227 | 26.053699 |
| 5 | 4.537034 | 22.040829 |

Median29.861202ops/s; pooled30.681886ops/s over500 operations/16.296260403 seconds. The declining samples are retained without selecting a best sample, proposing an unverified cause, retesting or tuning.

B/C independently check the same exact writer manifest, each with600 actual lstat calls requiring ENOENT and real process rc0. This proves final complete namespace paths absent, including when the sample directory ancestor has been removed. It does not prove every individual unlink's physical replica ACK, positive-cache invalidation, interrupted deletion or synchronous chunk reclamation.

Post-delete observations find600 distinct retained4KiB chunk identities, each two equal-content copies: A+B279, A+C321. One representative actual Meta REST response is satisfied R2. These are retained-content/placement observations after deletion, not complete pre-unlink ACK or garbage-collection acceptance. [Physical pairs](post-delete-physical-pairs-r1.json).

[Original tool review failure](tools/review-r2-blocked.json) identified acceptance of unrelated safe sample names. The writer d001 timing remains valid and unchanged; corrected380c validators alone reran B/C against the same59005d manifest. [380c review](tools/review.json) and [independent actual runtime review](review-runtime.json) approve that limited scope. Final canonicale087 adds only literal .. root rejection: [separate final review](tools/review-r3b.json), six affected Linux guards and offline validation of the same real manifest qualify this tool; e087 is not relabeled as the measured writer/checker. Two affected named-fixture guards pass; unchanged default/sync and standards are reused in their original scope.

[Tool provenance](tools/tool-provenance.json) records all identities, including an explicitly UNKNOWN overwritten r1 test source. Fixed Git e68e99e plus [one r2 recovery delta](tools/r2-narrow-diff.patch) restores d001 writer/1435 test on Linux. [Small380c→e087 delta](tools/r3a-380c-to-r3b-e087.patch) and its Linux reverse proof recover the actual checker from canonical61e36fe. No complete Python copies are stored here.

Four official processctl stops yield realwait0; no owned service or exact fresh mount remains, old9 process incarnations and2A mounts unchanged. Final allocated206,835,712B is below the predeclared1GiB cap and role budgets/floors pass. No environment repair, Rust rebuild, unrelated suite or repeated performance run occurred. Final receipt supplements the original PENDING observation without changing it.

[Read-only baseline survey](baseline-survey/report.json) found four39-entry3FS prefixes identical and no missing ELF dependencies, but no live qualified deletion comparator. Existing22fca045 upstream reference has an explicit ARM64 compatibility patch; old physical/restart evidence and stopped artifacts do not establish current fair namespace performance or stock/unmodified3FS parity. The deletion namespace qualification can be independent of the still-blocked three-sync read/write barrier proof. Next assess that small comparison's own artifact/resource/mount/timer contract; do not reconstruct an environment or run a large durability matrix as a prerequisite.

[Local-only index](local-only-index.json) preserves hashes/bytes for transport archives, private material and redundant full canonical diffs omitted from Git; original local artifacts remain unchanged. Publication preserves raw results, failures, commands and SHA indexes. Canonical maintained Python remains visible in development/acceptance. DefaultOFF trial release remains available; ON semantic failures, officialfuser API gap and later large/complex/etcd/Redis work are unaffected.
