# DFS synchronized small read phase — 2026-10-07

Current **6d51aeb /157 inputs/map66dbbe3e**; DFS-only/local-file Meta/gRPC/R2/native OFF, ARM64 Linux ctl/A/B/C and guest ext4. [Summary](summary.json), [fixed inputs](inputs.json), [raw quantitative snapshot](sync-quantitative-r1.json), [actual lifecycle receipt](worker-execution-receipt-r1.json).

One A writer confirms 64MiB with fdatasync/close plus directory fsync and full content. B/C each verify full SHA, perform one warmup, then five synchronized reads and final full SHA/EOF. All 12 C read samples and three stdio processes pass. ACK0 gates initial preparation; every dual DONE closes the ctl Linux monotonic window before ACK advances the next stage; ACK5 gates final SHA/EOF.

| Measurement | Median logical MiB/s |
| --- | ---: |
| Two-reader common window | 45.149364 |
| B C-tool wall | 50.326047 |
| C C-tool wall | 22.716080 |

The common window contains START send through both DONE receipt, including Lima relay/control/scheduling, C process launch and sample result writes. Preparation, warmup, ACK and final SHA/EOF are outside it. Five windows total 14.067568 seconds/640MiB, pooled 45.494716MiB/s. Host only routes stdio; it supplies no performance timestamps. Per-reader clocks are never added to construct the common elapsed time.

Actual Ready/Durable copies are A+B; B has a local durable copy, C has none at the pre-read proof. Cache and per-read RPC paths are UNOBSERVED, so this is not a two-remote-reader or cold-cache claim. Sixteen uniform4MiB extents deduplicate to one unique4MiB chunk; logical64MiB is not a unique physical64MiB dataset. [Raw replication](commands/live-r2-replication.stdout) and per-role physical content receipts retain this distinction.

Four AFS processes have real wait0, all three stdio processes have real rc0; exact fresh mounts and owned processes are gone, previous process incarnations/mounts unchanged. Final allocated199,868,416B is below the predeclared1GiB cap. No environment repair or repeated performance run occurred. The original quantitative closure PENDING is unchanged; final lifecycle/postcheck records establish closure PASS. [Final allocation](four-role-allocated-final.json).

[Tool review](tools/review.json) approves the bounded tool; [independent runtime review](review-runtime.json) validates the actual data/timers/closure. The original tool review retains its earlier protocol-document SHA; runtime review records the completed ACK protocol document, with driver/test SHA unchanged. Neither review upgrades a product performance gate. Current Linux evidence is 6 affected sync guards and11 fixture guards; prior guard failures/ResourceWarning/superseded protocol records remain in [tool provenance](tools/tool-provenance.json). Some overwritten intermediate source identities are explicitly UNKNOWN. The source/map/ELF are unchanged; unrelated Rust and historical standard suites were reused in their original scope.

[Local-only index](local-only-index.json) records SHA/bytes for transport archives and embedded full-source argv omitted from this packet. Originals remain local; TLS private keys, ELF and repeated Python snapshots are absent. Canonical Python is maintained in development/acceptance and is not hidden by attributes.

This closes the bounded content/timing/normal-lifecycle supplement. **Formal G2.21 remains pending matched three-sync durable3FS comparison.** G1 stays8/8; G2 counts unchanged. [Previous diagnostic](../20261007-dfs-manyread-small/README.md) remains immutable; its startup/precheck/warmup parent window is not the synchronized read-only metric and is not a before/after improvement comparison. Next independently assess DFS small delete; retain comparator blockers and native ON semantic failures.
