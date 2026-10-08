# DFS one-writer/two-reader timing inventory and probe boundary

**Fact: evidence inventory and measurement-tool readiness only. No new AFS runtime or performance acceptance.** The immutable f03/7bfc same-file 64MiB packet was inspected on the existing Linux build VM. [Result](result.json) verifies the packet SHA and all 700 members. Two warmups and ten measured C read tasks, five coordinator windows and original successful writer/closure receipts remain bound to their original f03 identity. This is not a new current-main R3 run.

| Existing observation | Reuse / boundary |
| --- | --- |
| B five measured validated whole-file reads | Direct wall durations retained; retrospective nearest-rank p50/p95/p99 = 1412.428029 / 1425.107188 / 1425.107188 ms |
| C five measured validated whole-file reads | Direct wall durations retained; retrospective nearest-rank p50/p95/p99 = 1412.028190 / 1422.062812 / 1422.062812 ms |
| Task clock | Guest-local CLOCK_MONOTONIC; open/fstat, 64 complete 1MiB reads, content generation/memcmp, EOF and close. Buffer allocation/JSON output excluded. |
| Coordinator windows | One ctl clock, before START emission through both DONE receipts; includes relay/Lima/result overhead. |
| Logical read latency / actual overlap | **Missing in the old data.** 640 measured logical reads are counts, not 640 observed intervals; short-read retries make actual syscall count unknown. No verified cross-VM clock mapping. |
| Formal performance | Cache/RPC qualification and matched 3FS baseline absent. Five whole-task samples are diagnostic; p95/p99 both select the maximum. These are not retrospectively declared acceptance metrics or syscall/block tails. |

## Small tool change

Only maintained test tools changed: `probes/dfs_unique_io.c` emits an additive `read_timing` object for successful reads; `dfs_r3_small.py` validates any present schema. Existing uninstrumented historical records remain valid for their original scope. A present but partial, unordered, wrong-clock, bool-valued or inconsistent interval is rejected.

Schema `complete-read-v1` fixes **64 complete logical 1MiB full_read intervals excluding the content oracle**, plus separate open/fstat/EOF/close spans. Retries remain inside each logical interval; this is not a per-syscall claim. One guest-local clock and total task boundaries are recorded. Whole-task timing, full data comparison, EOF, close errors and generation behavior remain. Write JSON receives no read schema. The probe remains a test helper outside the ordinary product package.

[Checks](checks.json): Linux RED regression first found the actual missing field; then 31 targeted C/driver/relay tests passed, including real emitted spans, generation/content corruption, invalid CLI and malformed intervals. A separately pinned helper was compiled with `cc -std=c11 -O2 -Wall -Wextra -Werror`; [Linux ext4 helper validation](probe-validation-result.json) and [64 actual raw intervals](probe-read.json) passed. This is helper validation, **not DFS performance**, and no Rust/product rebuild or AFS service run occurred. [Tool versions](tool-inputs.json) distinguish new probe ELF from the old f03 probe.

## Evidence, failures and next

[Compact archive index](archive-index.json) references the new inventory/commands/RED/green outputs outside Git and the immutable original 700-member packet. No payload, ELF, full maintained source snapshot, private config or VM image is copied into Git. The original KeyError RED output remains; no historical failure or evidence link is removed.

**Decision:** G1 historical 8/8 stays closed; G2 counts and default OFF remain. Production/Rust compiler bytes are unchanged from main0bd. The completed subitem is **timing inventory + probe readiness**, not G2.21 performance.

**Next bounded item:** one small read-only synchronized B/C supplement using the pinned new helper, fixed candidate identity, raw logical-read p50/p95/p99 method declared before startup, full SHA/EOF, three distinct durable-copy proof and normal exits. Reuse confirmed writer data where admissible; do not rerun the recovery/full-standard matrix. Cache remains explicitly unqualified until observed; cross-VM actual syscall overlap, qualified 3FS comparison and large/complex cases stay separate.
