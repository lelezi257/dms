# G2.11 historical completion: reuse boundary, no repeat benchmark

**Fact:** [G2.11](../../trial-release-goals.md) already completed the e925 limited case: 100×4KiB/C1, five pairs, correctness and an ext4 comparison report, no hard ratio. [Original evidence](../20261006-e2e-current/r4/README.md) retains its version, criterion and conclusion. Do not reopen it or relabel its rates as f03 measurements.

**Fact:** [e925→6d impact audit](../20261007-owner-standard-reuse/code-impact-review.json) preserves the ordinary Owner unlink/permission/error path. The fixed 6d→f03 diff changes Owner module declarations/test visibility, while root/remote/native_home/localfs bytes stay identical. Shared FUSE unlink adds request counting and preserves dispatch/errno. The 7e6→f03 compiler-input change is only DFS create. [Exact versions, hashes and raw diff index](review.json).

**Fact:** [7e6 impact map](../20261007-current-trial-7e6/impact-map.json) asks for current OFF install/basic operations/sync/read/local-file recovery/closure after additive startup/counter changes. [f03 installed audit](../20261008-current-trial-f03/independent-stored-audit.json) supplies that scoped regression. Its Owner selfcheck actually includes two basic unlink calls; it is not an independent current 100-file absence check.

**Decision/inference:** keep the historical limited completion and reuse the unchanged ordinary OFF functional scope, backed by the existing impact reviews and current basic regression. No new full-suite or five-pair run; no current delete throughput claim. G1 historical 8/8 stays closed and G2 counts do not change.

Sequence correction: the planned next deletion run in the local-write slice was superseded by this evidence check before launch. Return to the already identified G2.12 workspace-bind necessary function gap: authorization changes and existing native FD/mmap reference drain. Existing normal host lifecycle and eight G2.13 performance cases are not reopened. The general revocation/drain gap is an implementation boundary, not something an extra PASS test alone can close. Ordinary remote missing-latency data remains independently pending.
