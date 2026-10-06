# Owner remote small read/delete diagnostic

2026-10-07, unchanged product source6d51aeb/map66dbbe3e/157 inputs.
Linux ARM64 ctl/A/B guest ext4, local-file central Meta, Owner-only/gRPC/R1,
native OFF. New B Home/sole Moose chunk, A client. Stock MooseFS4.59.2/ac106b2
fixed ELF/tools reused; old services/state untouched. [Contract/admission](../20261007-owner-remote-bhome-preflight/README.md).

**Sampled correctness PASS; performance DATA_RECORDED; whole slice NOT PASS.**
Each read/delete has one warmup plus five alternating pairs, no repeats/tuning.
Owner/B independent64MiB read fullSHA PASS; B FUSE600 deleted paths and six
removed directories absent. Actual Home serving/B session/epoch and sole
Moose class CREATE/KEEP1copy/physical chunkB proved. Full formal performance
qualification remains NOT_QUALIFIED: cache UNOBSERVED and A/ctl old load
retained; no cold/hot/full-VM-fairness/strong durable-write claim.

| Five measured samples | Owner median | Moose median | Median paired Owner/Moose rate |
|---|---:|---:|---:|
|64MiB sequential read, C1/1MiB blocks|427.371MiB/s|14,854.399MiB/s|0.028803|
|100×4KiB namespace unlink|1,039.769unlink/s|1,241.516unlink/s|0.842255|

Read timer is the existing C open/thread/pread+content-check/close timer.
Delete preparation fdatasync/files+directoryfsync/fresh full content,
post-unlink directoryfsync/namespace checks and rmdir are outside100unlink
timer. Moose trash/physical reclamation excluded; default policy retained.
[All rates/pairs/boundaries/status](small-summary.json), [versions/input hashes](inputs.json).

**Cleanup FAIL retained:** A foreground mfsmount received validated TERM,
real parent wait exit1, exact mount disappeared. Original exit0 gate remains;
no rerun, KILL, lazy unmount, threshold change or environment repair.
[Raw stop failure](commands/a-stop-moose.stdout), [actual wait and logs](commands/a-moose-stop-failure-detail.stdout).
Other two Moose services and all three AFS services stop0. Independent ctl/A/B
postchecks prove no owned process/mount and unchanged prior process incarnations;
this limited observation does not convert nonzero stop into successful cleanup.
[ctl](commands/ctl-independent-postcheck-and-export.stdout),
[A](commands/a-independent-postcheck-and-export.stdout),
[B](commands/b-independent-postcheck-and-export.stdout).

Relevant raw commands include [Home](commands/home-location-direct.stdout),
[one B chunk](commands/moose-live-topology.stdout), [class](commands/moose-class-policy.stdout),
[physical file](commands/moose-file-info.stdout), [B read](commands/b-independent-owner-content.stdout),
[B absence](commands/b-independent-deletion-and-chunk.stdout),
[read](commands/small-read-r1.stdout), [delete](commands/small-delete-r1.stdout).
Guest per-sample results/config/launch/real wait/logs are under guest/.

The initial inherited-proxy HTTP502 query is kept, then explicit direct internal
route succeeded; it is not a product failure. Initial review found default
all-case missing-read falseDATA_RECORDED; narrow tool fix and8 final Linux
guards PASS, earlier4 unchanged fixture guards reused =12 unique methods.
Initial9-test receipt/review retained, without counting executions as new tests.
Two prior tool versions are restored byte-exact from current canonical tools
using reverse deltas [verified on Linux](commands/reverse-delta-linux-restoration.stdout);
no full Python source snapshot is stored here.

[Independent final review](review-r2.json) accepts the raw data and tool guards; it rejects a whole-slice PASS because the real cleanup exit1 is retained.

G1 history8/8 and G2 formal counts unchanged. G2.14/G2.16 gain independent
small data/correctness receipts; formal read target and full selected cleanup
remain open. Remote optimization and comparator-client shutdown correction
are independent TODOs; no more tuning/retesting this packet. Next selected
core data case is DFS small one-writer/multiple-readers. Strong3-sync/3FS
baseline remains independently blocked and cannot be substituted by R2 data.
