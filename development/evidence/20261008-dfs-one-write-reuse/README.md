# DFS one-writer evidence reuse audit

## Plan and boundary

Audit the immutable f03 saved packet on existing Linux, associate each A-written file with both B/C reads and physical R3 observations, preserve sequential/concurrent and pre-read/post-stop observer boundaries, then update the active ledger. No product rebuild, runtime retry, service, VM or historical acceptance changes. This is a new evidence audit of an old run, not a new product test.

## Result: limited functional reuse

**Fact:** [Linux result](result.json) explicitly links two different A-written 64MiB files to independent C and B reads: round00/generation1/SHA46a900… and round03/generation4/SHA757ac8…. Each write has successful fdatasync, close and parent/root directory fsync, complete SHA/EOF; each reader has matching path/generation/content identity, pre/post full SHA/EOF and successful read/close. Both reader routes run in sequence, C then B. The same-file control windows do not overlap. Their existing aggregate wall values remain observations, without derived latency percentiles or a new common-reader throughput claim.

**Fact:** each file has16 different4MiB chunks and48 physical observations across A/B/C, with three Ready/Durable nodes per chunk. Original R1 observers incorrectly label old7e6 Meta/Node ELF; the preserved R2 correction is a **post-stop read-only observation**, not a replacement pre-read snapshot. Chunk arrays are identical, but index00 catalog/selection revisions and both Meta snapshot metadata differ. The audit records all changed top-level fields. Actual original live captures separately bind f03 Meta/Node ELF, and four same-incarnation wait0 receipts/owned PID closure are preserved. Existing saved protection flags and unchanged live-to-final protected process inventories are checked; this does not reobserve today's VMs.

**Fact:** product f03dc2b3679c31daa51caee275fb2087413e949c, compiler map2b17fad77c87b4977d79e14809b6eada755f648767dd29b5503e49556f8ce7b4. The original run used package **e622fdef35ca3857edfa39aba931f050a27b15c1ceb740e1108c9be1e8bebdf3**, not the later7bfc guide/manifest package. Full Meta/Node/probe SHA is in result.json. No current main runtime identity is implied by this audit.

**Decision:** reuse this f03 run for the G2.21 subclaim “one A writer, after confirmation, two independent B/C reader views return the same file.” Historical931 simultaneous-reader timings and7e6 recovery retain their original versions. Full G2.21 stays pending: current f03 simultaneous same-file readers, independent per-operation arrays/p50/p95/p99 and prospectively fixed judgment percentile, cache/RPC qualification and a comparable3FS reference are absent. G1 historical8/8 and G2 aggregate11 limited/1bind in progress/15unaccepted remain unchanged; default OFF.

## Reproducibility and failures

[Original immutable packet index](../20261007-dfs-r3-multinode-current/raw-archive-index.json) remains unchanged: raw archive4ff33303… /1,217,143B /2039 members and member index1f9b1e8…. This Linux audit reads directly from that archive and checks every member's bytes/SHA before interpreting records. The6378 assertions include6117 member type/bytes/SHA checks; **they are not6378 product test cases**.

[Command](command.json) records the audit SHA and actual Linux exit0. First attempt exit1 is retained: the oracle compared an active protected mount inventory, including owned FUSE mounts, with the post-stop inventory. R2 compares protected process incarnations, validates old mounts retained while active and exact final restoration using the saved flags. Only the audit changed; no runtime or environment retry. The original run's23 ERRO/60 WARN, previous product/observer failures and archive mappings retain their original conclusion.

[New compact archive index](archive-index.json) pins7 small raw audit members outside the source tree. [Actual Linux restore](restore-result.json) extracted all7 and checked every size/SHA, then reverse-applied the audit delta and proved the exact first failed script SHA. No Python script copies, payload, ELF or VM image are added to Git. To reproduce, recover audit.py from that pinned archive and run the recorded argv against the original packet; the original frozen evidence is required.

**Next:** the narrow current-candidate gap is same-file simultaneous B/C readers. Prepare/check that small item first;3FS qualification, broad optimization and large/long cases stay deferred. Already passed standard, recovery and workspace core performance results are not replayed.

[Source identity/path check](source-check.json) retains a host bookkeeping failure: the check assumed the adapter was below ownerfs; the existing adapter actually is src/node/native_workspace.rs. Correcting the assertion verified the real path; no file was moved or behavior changed. Protected handoff/acceptance-lock SHA remains unchanged.
