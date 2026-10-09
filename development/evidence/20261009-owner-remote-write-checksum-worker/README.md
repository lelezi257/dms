# G2.15: one inline-checksum scheduling experiment, rejected

2026-10-09. Function **LIMITED_PASS**, measurement **COMPLETE**, optimization **REJECTED**; formal MooseFS write target and original G2.15 **PENDING**. No new product improvement or trial package.

| Metric | Published Node d14 baseline | Candidate abdecaa448 | Candidate/base |
| --- | ---: | ---: | ---: |
| Median throughput MiB/s | 177.476873 | 177.590363 | 1.000639 (+0.06%) |
| Independent pwrite p50 ms | 4.963867 | 5.031991 | 1.013724 |
| Independent pwrite p95 ms | 7.524260 | 7.294252 | 0.969431 (-3.06%) |
| Independent pwrite p99 ms | 10.188698 | 8.097937 | 0.794796 |

Preset retention required throughput>=1.05 **and** p95<=1.0: **FAIL**. The sole change moved nonempty inline checksum verification from the async RPC worker into the existing blocking closure, still before Home write. It removed no checksum work; neither checksum CPU nor reactor contention had been proven the bottleneck. Product logic fully reverted, direction closed without rerun. Only the meaningful real-RPC regression remains in main. [Results](paired-result.json),[production restoration](production-restore.json).

Source base6ee30416 + patch f9d22dfa10d0915c7afcd78bc1b73465b88ef128d60417dfc7c3543fd492c8e0; candidate Node abdecaa44849a0599e0657a5cf7c68c9d0f51d7c96d2a374ad4ca23b3aeabd03,160input map4ae7a372. Published Meta650 on both phases; Owner-only/B real ext4 Home workspace bind ON/C remote FUSE/ctl local-file. Baseline Node d14 remains the retained published product; current source adds cfg(test) coverage, not a deployed new runtime. [Candidate](candidate.json).

One warmup+five formal writes per phase, same uid501/0600 original inode64MiB/C1/1MiB logical writes and frozen c87 probe; reset same physical file to98 outside timer, verify fresh remote SHA98/hot backing, then timed overwrite97+fdatasync and full physical/remote SHA97/EOF. Real pwrite+count intervals exclude oracle;320 samples/side pooled nearest-rank preset p95, p50/p99 retained. Whole probe throughput includes open/thread scheduling/IO/fdatasync/close; no derived latency. mtime/ctime necessarily change on writes and are recorded, not asserted invariant; original device/inode/size/bytes/permissions preserved. [Contract](frozen-contract.json),[all formal samples](operation-samples.json).

New real RPC regression passes against both original and candidate implementations: wrong32-byte checksum→CORRUPT_DATA; malformed checksum length/length-data mismatch→TRANSFER_INVALID; rejected requests never invoke Home handler or count successful payload, subsequent valid and empty legacy checksums succeed. Candidate8 affected tests/fmt/default-all-target check/lib Clippy/release PASS; restored4 RPC tests/fmt/check/lib Clippy PASS. Existing two dead_code warnings retained; no full POSIX/all-repository strict lint claim. Server handler histogram includes checksum in the candidate only; benchmark uses unchanged syscall timing, not that changed diagnostic boundary. [Validation](validation.json).

Five distinct actual wait0; original helper/ELF inode, protected process/boot/full mount inventories restored. Capacity gates pass at each sampled write boundary; no environment blocker. Cumulative retained fixture logs46,073B/19ERRO/107WARN/zeroTRACE preserved, not classified all as new failures. Initial format failure and saved-audit floating-expression mismatch retained; only tools/format corrected, product pair not repeated. [Logs](retained-logs.json).

1469raw and26 actual guest files recovered on Linux; candidate source patch reapplied to exact Git base and hashes matched, published Meta recovered. Only after that, five temporary ELF copies97,666,208 logical bytes (~93.1MiB) removed; one candidate Node remains externally with patch, original payload/logs/history retained. Git stores compact results/index, not ELF/archive/full source snapshots. [Archive and restoration](archive-index.json),[cleanup](cleanup.json).

G1 historical8/8 closed; original G2 12 limited complete/0bind in progress/15pending unchanged. Next: one bounded remote-write CPU/transport cost diagnostic using the existing probe and retained file before selecting another product change; no repeat of rejected checksum/copy/decode/dispatch directions. DFS write/A-ctl expansion remain user-deferred. Moose strong durable-ACK comparison, full final performance, large/long and complex reliability remain pending.
