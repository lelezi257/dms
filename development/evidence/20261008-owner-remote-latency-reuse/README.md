# Historical ordinary OwnerFs remote latency reuse, 2026-10-08

Facts: existing Linux evidence audited on `afs-build` without compiling, measuring or starting services. Both immutable read/write packet manifests match: 415 + 409 files; 941 consistency assertions pass. These assertions are evidence-integrity checks, not 941 product tests. [Audit](audit.json), [exact reproducible command](audit.command.json), [Linux compact-index/protected-identity validation](validation.json). Original results, failures and links are unchanged.

Product `6d51aeb45c1ed8669d80f612b3817e6d1bdabe04`; ctl Meta/master, A client, B Owner Home/sole Moose chunkserver; 64MiB, 1MiB blocks, C1, one warmup/five alternating measured pairs. Each of the ten read and ten write measured payloads already records independently timed p50/p95/p99. The blanket description “missing independent latency” is corrected; source/tool identity and operation boundaries are part of this audit. None of the old timings becomes current `f03` data.

Helper source is recoverable at Git `b904c100cf51e70f8835ce6cee4b6aaaf1278ac9:development/acceptance/probes/ownerfs_native_closeout_io.c`, SHA `e32168dccaa0620698ff5aec77e47c403c9cd72e8166999eaf127cb84cabf364`. [Original build provenance](../20261007-container-perf/payload-provenance.json), source/build receipts and [remote deployed ELF identity](../20261007-owner-remote-small/inputs.json) bind ELF `70ac97c7634d406a177a74c783d446b62a2014ba198586132882a1d9228e55e8`. The product commit precedes the helper import; do not label the tool source as part of product6d.

Per-read latency uses CLOCK_MONOTONIC around pread, complete-count and content validation, excluding open/thread setup/close. Per-write latency similarly surrounds pwrite+count; fdatasync is separately recorded and included in task wall time. Original code sorts64 intervals and emits indices32/60/63 for p50/p95/p99. Only scalar quantiles were exported; it is impossible to reconstruct pooled320 quantiles from five quantiles. [Original read payloads](../20261007-owner-remote-small/guest/a/results/read-r1/read-samples/), [original write payloads](../20261007-owner-remote-write-small/guest/a/results/write-r2/samples/).

| Read round | Owner p50 / p95 / p99 (ns) | Moose p50 / p95 / p99 (ns) |
| --- | --- | --- |
| 1 | 2415197 / 2838687 / 2963893 | 54958 / 92789 / 100081 |
| 2 | 2368656 / 2921518 / 3108598 | 59457 / 127455 / 147288 |
| 3 | 2470779 / 2950101 / 2994600 | 56249 / 90289 / 119914 |
| 4 | 2385031 / 2785397 / 2975059 | 60332 / 123831 / 141580 |
| 5 | 2488653 / 2906811 / 3176513 | 55166 / 88748 / 98247 |

Qualification stays NOT_QUALIFIED: original contract froze diagnostic/cache UNOBSERVED, not a judging percentile; no per-operation interval arrays; old load retained; original remote-read mfsmount actual wait1 failure retained. Write additionally lacks strong durable-ACK qualification. These summaries are reusable historical diagnostics, neither a new 1.2/.8 acceptance nor current performance. No aggregate p95 or selected favorable round is manufactured. G1 historical8/8 and G2 counts remain unchanged.

Next preparation: stock Moose4.59.2/ac106b2 supports `mfscachemode=DIRECT` (kernel data cache bypass), distinct from NEVER (do not retain cache across opens). Existing A ELF/help identity verified read-only: [receipt](moose-cache-help.json). [Version-pinned parser](https://github.com/moosefs/moosefs/blob/ac106b2ec8661ff00def725d042cb67d3ca2184d/mfsclient/mfsmount.c#L1855-L1867), [public open response](https://github.com/moosefs/moosefs/blob/ac106b2ec8661ff00def725d042cb67d3ca2184d/mfsclient/mfs_fuse.c#L3842-L3854). This is preparation, not a qualified baseline. Direct mode does not prove Home/chunk backing-cache equality, disable Moose userspace readahead, or close the old exit failure. No global drop_caches, old service stop, mount or VM repair occurred. Freeze a separate current remote-read case and judge p95/raw arrays before any timer; retain ordinary default configuration data separately. Current helpers pin the old ELF/cache-unobserved interface and must not silently accept a replacement ELF. Comparator qualification remains bounded separate work.

Raw commands/receipts live outside source at `evidence/afs-delivery/owner-remote-latency-reuse-20261008-r1/`; Git carries this compact audit/index and commands, no copied helper source, ELF or data files. Productf03/7bfc unchanged; no Rust build/test is needed for this documentation/evidence-only correction.
