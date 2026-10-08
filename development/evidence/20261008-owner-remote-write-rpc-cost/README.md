# G2.15 remote WRITE cost diagnostic — 2026-10-08

**Whole run FAIL_BUDGET; diagnostic data available; no product gain or final performance acceptance.** Production Node839e14d9 / Meta15648a87 / probec87a5fd8, source main461407c4 and unchanged product map81c30c95. Reused stopped ctl/B/C local-file / Home bind ON / remote FUSE fixture. [Identity](identity.json), [frozen plan](frozen-plan.md). No A access, VM repair, new package or Rust build in this diagnostic.

One64MiB/C1 logical1MiB write+fdatasync, before/after existing metrics, full raw independent pwrite samples retained externally. [Result](rpc-cost-result.json):

| Observed method | Calls | Sum ms | Mean ms |
| --- | ---: | ---: | ---: |
| Home OwnerFiles.Write | 128 | 13.708106 | 0.107095 |
| Remote client write | 128 | 309.802673 | 2.420333 |
| Home OwnerFiles.Fsync | 1 | 47.279580 | 47.279580 |
| Remote client fsync | 1 | 47.646664 | 47.646664 |

Independent64-operation pwrite+count-check p50/p95/p99 = 4.728112/6.887328/9.573100ms; entire write-task wall371.032533ms, final fdatasync47.686957ms (12.85%). Home write-handler total is4.42% of client write-RPC total. The client includes checksum/encoding and other work; server timing starts after decode/checksum. Their difference is not pure network time.128 RPC does not establish individual request sizes; no percentile subtraction or latency inferred from throughput. Flush had no after-series and is explicitly NOT_OBSERVED.

The old read file's full content/inode/cache identity remained unchanged. New byte99 file full SHA, uid501/mode0600 and remote fresh-open EOF checks passed. [Saved-record audit](audit.json) verifies32 claims on Linux; this does not turn the run into PASS.

The preflight incorrectly omitted the new64MiB file from peak allocation. B rose from214,503,424B to281,620,480B, exceeding its fixed268,435,456B role budget by13,185,024B (12.57MiB); filesystem still had21,320,093,696B available. This was fixture-budget failure, not VM disk exhaustion. [Original failure](run-failure.json) remains; no threshold waiver or rerun. Original and diagnostic data remain in place.

All three real children exited0 with supervisors/PIDs gone and original mounts/protected processes unchanged: [B](b-lifecycle.json), [C](c-lifecycle.json), [ctl](ctl-lifecycle.json). These independent lifecycle checks are separate from the failed budget closure. [Logs](raw-logs.json) retain10,615B including4ERRO/23WARN across the reused fixture; [audit output-path failure](audit-first-failure.json) was corrected by emitting Linux stdout, without environment repair or product rerun.

[Archive](archive-index.json):242raw+27guest files actually restored and SHA-verified on Linux;70,784B, SHA4fd9404286b59b3ac45f3e7b7831b0fc389115263ffeafeb9fff56b97565a761. [Provenance](provenance.json). Raw commands, scripts, metrics and logs stay outside Git; only compact results/indexes are committed.

Next candidate investigates the Owner-only TCP server receive-frame limit, retaining TLS, fresh-open, permissions, errors and fdatasync. No formal write pair has started; the previously asked fixture-space choice remains pending. [Retained READ improvement](../20261008-owner-remote-read-frames/README.md) stays unchanged. DFS write is deferred by user choice. G1 historical8/8 and G2 original12/0/15 remain unchanged; G2.15 formal Moose1.2/.8 is PENDING.
