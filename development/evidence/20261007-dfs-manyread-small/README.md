# DFS small one-write/two-reader diagnostic

2026-10-07. Fixed product6d51aeb/map66dbbe3e/157 inputs; Linux ARM64
ctl/A/B/C dedicated guest ext4, local-file Meta, DFS-only/gRPC/R2/native OFF.
[Contract](contract.json), [full identities and 15 Linux guards](inputs.json).
No product build or standard-suite rerun; unchanged passed evidence reused.

**PASS: bounded writer/independent readers/content/normal cleanup.**
**DATA_RECORDED: performance; formal G2.21/three-sync durable/3FS remains pending.**
A writes64MiB uniform byte97 with C fdatasync, parent/sample-directoryfsync,
fresh fullSHA verification; B/C validate the exact copied writer receipt,
fresh independently open/read full64MiB and each execute1 warmup+5 full reads.

| Data scope | Logical MiB/s |
|---|---:|
|One A confirmed write, one timing sample|241.424|
|B five measured reads, median|26.138|
|C five measured reads, median|50.222|
|Controller aggregate:12 C reads/768MiB over17.873s|42.969|

Aggregate includes both readers startup/prechecks, warmup and five measured
reads each; its one parent monotonic wall is not a cross-VM synchronized
read-only interval. Cache UNOBSERVED, retained old workload and unequal
ctl/node roles preclude a formal fair comparator claim. Pre-read fullSHA is
outside the C timer. [All rounds/rates/status](summary.json), [controller wall](controller-reader-window.json).

Uniform logical64MiB content deduplicates: observed one4MiB physical chunk on
A and C, none on B. This is not a unique64MiB physical corpus. Live Meta proves
exact two Ready/DurableReplica copies A/C, available2/Satisfied/no pending tasks;
full physical content and original catalog SHA recorded. [Live replication](commands/live-r2-replication.stdout),
[A physical](commands/a-physical-r2-content.stdout), [B](commands/b-physical-r2-content.stdout),
[C](commands/c-physical-r2-content.stdout). No fault-durability or three-sync qualification.

[Writer raw](commands/writer-r1.stdout), [B raw](commands/b-reader-r1.stdout),
[C raw](commands/c-reader-r1.stdout). Selected config/TLS/SHA/print-config,
live executable+mount evidence and actual exit receipts are retained under
commands/ and guest/. Private keys, ELF/rootfs, archives and full source tool
snapshots are excluded; maintained first-party tools stay at canonical paths.

All4 real processctl waits exit0, exact new mounts/processes absent, old
incarnations unchanged: [ctl](commands/ctl-postcheck-r2.stdout),
[A](commands/a-postcheck-r2.stdout), [B](commands/b-postcheck-r2.stdout),
[C](commands/c-postcheck-r2.stdout). Original [ctl checker failure](commands/ctl-postcheck.stderr)
retained: substring search of __supervise in its own source/args misclassified
checker+sudo; exact argv-token correction excludes them. No product restart,
IO remeasurement, KILL/lazy unmount, environment repair or gate relaxation.
[Tool review](review-tools.json), [independent runtime review](review-runtime.json) approve bounded content/data/cleanup only.

G1 history8/8 and G2 formal counts unchanged. Historical e925 one-writer/two
reader and core Meta reload remains original version; current6d evidence adds
only the bounded R2 branch. Next Owner remote small write diagnostic with
frozen barriers; strong Moose write and three-sync/3FS baselines stay independent
BLOCKED, targeted performance tuning and broad/long cases deferred.
