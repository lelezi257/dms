# G2.21 — one bounded missing-read measurement (2026-10-08)

| Claim | Outcome | Version and scope |
| --- | --- | --- |
| Function | PASS, limited current small case | 0bd66ac5 runtime, 158 compiler inputs/map866cd522; 64MiB counter dataset, A confirmed write, B/C one warmup + five formal reads, 16 distinct chunks × three synchronous Ready/Durable physical copies |
| Measurement | COMPLETE | Each reader 320 real logical 1MiB full_read intervals, CLOCK_MONOTONIC, excludes content oracle/open/fstat/EOF/close; nearest-rank pooled per-reader p50/p95/p99 fixed before running |
| Performance | PENDING | No qualified matched 3FS comparator. No hot/cold, network/syscall latency or actual cross-VM overlap claim |
| Delivery | NOT A DELIVERY RUN | f03 package only carries install/config/process tooling; pinned Meta/Node replaced before start. No new ON trial or ordinary release qualification |

| Reader | p50 ms | p95 ms | p99 ms | Whole-task median MiB/s (diagnostic) |
| --- | --- | --- | --- | --- |
| B | 21.598651 | 29.518704 | 35.177468 | 46.064920 |
| C | 21.601027 | 28.589659 | 35.514288 | 46.047017 |

Exactly one formal runtime attempt; no Rust/product change or before/after improvement claim. A fresh bounded fixture uses one initialization write because old persisted fixtures violate the fresh-only admission contract; prior functional/three-copy/recovery proofs retain their original identity and are not rerun as a recovery matrix.

Four actual wait0, eight owned service/supervisor PIDs gone, FUSE/UDS closed, all twelve protected process incarnations and original mounts retained. Peak 592,080,896 B below 896 MiB; INFO logs 5,370 B, five ERRO/ten WARN retained, no TRACE. Two read-only audit failures (unneeded import; incorrect close==task-end assumption) are retained in the external packet; corrected audit follows the existing duration/boundary contract and reads the same runtime records without rerunning the product.

Reproduce runtime on the admitted four-role environment with external `run.py prepare`, then `run.py run` **once**; commands, arguments, raw reader samples/coordinator tokens, identities and failed auditors are in [archive-index.json](archive-index.json). Compact outcomes: [function](function-audit.json), [measurement and raw intervals](measurement-audit.json), [pre-run contract](contract.json), [identity](identity.json). The inherited runtime flag `no_threefs_or_latency_or_actual_io_overlap_claim` means no qualified comparative latency/3FS/overlap acceptance; the separate raw-interval audit records the completed measurement.

G2.21 remains performance-pending; G1 historical 8/8 and original G2 totals unchanged. This slice is closed. Next: G2.12 current bind ON + remote FUSE necessary functional closure and ON scenario delivery; no more DFS timing/tool/matrix work before it.
