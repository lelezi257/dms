# Ordinary OwnerFs local read: raw latency and honest cache qualification

2026-10-08, G2.09 independent small item. Productf03dc2b3/map2b17, ordinary7bfc package unchanged. Both workspace switches OFF; existing B/client, Owner Home, official Moose4.59.2 master/chunk/client on B ext4. Local-file Meta/gRPC, one-copy CREATE/KEEP and one local VALID physical copy. [Predeclared contracts](contract-r3.json), [separate diagnostic declaration](contract-diagnostic.json), [Linux audit](independent-stored-audit.json), [raw commands/results/index](raw-index.json).

| Classification | Result and evidence |
| --- | --- |
| **Current tool PASS** | Optional C `samples` output preserves all sorted measured intervals; default CLI/output stays unchanged. Five real Linux integration guards cover legacy output, corrupted content/error propagation, hot residency, read/write sample quantile recomputation and invalid option refusal before data change. [Qualification](io-guards.command.json), [pre-edit baseline](baseline-regression.json). No product/Rust/vendor modification; all157 compiler-input hashes unchanged. |
| **Current formal comparison qualification FAIL** | First Owner hot prerequisite returnsrc3/0 resident bytes before timing. Zero completed warmup pairs or measured pairs; no throughput/latency verdict. Original command025/stdout/stderr and normal closure preserved. No hot→repeat relabeling or unchanged-score rerun. |
| **Current diagnostic data complete; formal NOT_QUALIFIED** | New, separately frozen repeat-buffered run:64MiB,1MiB blocks,C1,byte97,1warmup+5 alternating pairs, full content verification before every read and afterward;12 successful C payloads,320 measured intervals per target. Owner mincore before/after0, Moose64MiB: different client cache observations, not matched cache. Numerical ratios below are diagnostic only. |
| **Current closure PASS** | Each product run:2AFS actualwait0 +3Moose actualwait0,7 child/supervisor PIDs plus the test runner gone. Normal Moose/FUSE/UDS closure; original process/mount inventory unchanged. [Independent guest check](independent-closure.json). No VM installation/reset/repair, protected services untouched. |
| **Historical unchanged** | Prior e925 ext4 FAIL and6d remote diagnostics retain original versions/criteria/conclusions; this is not POSIX qualification or a change to G1 historical8/8. Full G2.09 stays pending under1.2×throughput AND0.8×p95. |

| Diagnostic metric | OwnerFs | MooseFS |
| --- | ---: | ---: |
| Median throughput MiB/s | 5937.148236 | 18687.489981 |
| Pooled p50 ns | 161458 | 45459 |
| Pooled p95 ns | 206000 | 54542 |
| Pooled p99 ns | 278834 | 60501 |

Diagnostic throughput ratio0.317707; pooledp95 ratio3.776906. Every five-round rate and640 measured values are preserved in the external archive. Operation timing is CLOCK_MONOTONIC around `pread + count + content-check`, independently measured from task `open→thread setup→reads→join→close`; no throughput-derived latency. Sorted floor(N*p/100) order statistic/p95 judging percentile were frozen before both runs. Warmup excluded.

Preparation failures remain: Lima directory-copy produced a nested tools directory, corrected only the path before launch; the first version query incorrectly used mfsmount-v (mount verbosity) instead of official-V, with no services/data run. The initial recipe's cleanup summary incorrectly demanded3waits when none had started; later check requires a receipt for every actually started process. Original failures/source and minimal reconstruction patches are indexed, never deleted or called environment failures.

Hot/diagnostic INFO logs6905/8225B retain3ERRO+3WARN and3ERRO+9WARN respectively; [grouped messages](log-summary.json), full logs retained. Maximum sampled allocations remain below2GiB and free above4GiB; these are phase snapshots, not continuous peaks. Guest data roots remain; text archive does not prove full user-data restore. No package rebuild/republication, no full standard rerun or third-party change.

Next: retain the cache-qualification gap for a separate comparison/tuning item; continue the next basic local-write case. Missing matched-cache formal read evidence remains open, not waived or demoted out of the goal.
