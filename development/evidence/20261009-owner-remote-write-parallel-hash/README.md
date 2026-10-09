# G2.15: client parallel checksum rejected after one pair

2026-10-09. Function **LIMITED_PASS**, measurement **COMPLETE**, optimization **REJECTED**, formal MooseFS write target/original G2.15 **PENDING**. Production restored to c16 logic; only a real large-payload grpc regression retained. No product improvement or trial package.

| Metric | Retained Node d14 | Candidate dee312e361 | Candidate/base |
| --- | ---: | ---: | ---: |
| Median throughput MiB/s | 185.936355 | 181.353319 | 0.975352 |
| Independent pwrite p50 ms | 5.139814 | 4.849489 | 0.943514 |
| Independent pwrite p95 ms | 7.244649 | 7.152447 | 0.987273 |
| Independent pwrite p99 ms | 9.363645 | 8.782456 | 0.937931 |

Preset retention throughput>=1.05 **and** p95<=1.0: FAIL (throughput−2.46%, p95−1.27%). Both original phase order and all samples retained, including candidate230.714MiB/s outlier; no retry/reordering/tuning. Single experiment split >=512KiB grpc client BLAKE3 into canonical public hazmat subtrees with at most one extra process-wide helper thread and serial busy/failure fallback. Exact same32-byte digest, server verification/dispatch/FIFO/auth/errors/durability/RDMA unchanged; no third-party/new dependency changes. Direction closed and production helper/import/constants/helper-only tests removed. [Before/after](paired-result.json),[restoration](production-restore.json).

Source c16b0267 + patch a97b3700ee260ed9e9f769f6a857d89ad9ab923b3e66156aaa4e741276a1d3bc; candidate dee312e361c252e367eaf1b72fadd1f00337432bfb3c511350135aa2aca14482,160 compiler map49c4db76. Candidate patch/ELF remain external and actual source patch recovery checked; tested ELF is rejected, not current product. Published Meta650 and d14 baseline, B real ext4 bind ON/C remote FUSE/ctl local-file. [Identity](candidate.json).

One warmup+five formal per phase, baseline then candidate exactly once; same existing64MiB inode/uid501/mode0600/C1/1MiB logical pwrite/fdatasync. Reset98 outside timing then fresh remote full SHA98/hot backing; timed overwrite97 and physical+remote full SHA97/EOF after every round. Independent pwrite+count-check intervals exclude oracle,320 samples/side pooled nearest-rank predeclared p95; p50/p99/raw samples saved. Whole-probe throughput includes open/thread scheduling/IO/fdatasync/close. [Frozen conditions](frozen-contract.json),[raw formal samples](operation-samples.json).

Linux candidate26 unique targeted tests (grpc7 includes hash2, plus dispatch15/flush2/fsync1/release1), fmt/all-target check/lib Clippy/release PASS. Restored5 grpc tests/fmt/check/lib Clippy PASS; two existing dead_code warnings retained. New regression sends deterministic1MiB−1 through actual OwnerFiles grpc, checks offset8192/kill_suidgid/exact digest and short-write2/payload metrics; passes after optimization removal. Candidate hash/concurrency helper-only tests remain recoverable in rejected patch, not dead tests in main. First fmt failure retained; no full POSIX/all-repository strict lint claim. [Validation](validation.json).

Five distinct actual wait0; same original device/inode/size/full bytes/permissions and helper/ELF identity, all protected boot/process/full mount inventories restored. Capacity gates pass across all sampled boundaries: ctl/B/C max118,415,360/240,873,472/173,428,736B; min free1,952,124,928/21,223,940,096/36,848,881,664B. No current capacity blocker. Cumulative logs64,703B/24ERRO/157WARN/0TRACE retained, not all classified as new failures. [Saved-data audit](saved-data-audit.json),[logs](retained-logs.json).

1457 raw/26 guest files/12 references actually recovered on Linux, candidate source patch reapplied to exact Git base and Meta recovered from published package; then five temporary ELF copies97,808,416B (~93.28MiB) removed. Final closure append-only streams separately archived/recovered; initial indexing assertion retained and corrected without product rerun. One rejected candidate ELF retained externally. [Archive/recovery](archive-index.json),[cleanup](cleanup.json).

G1 historical8/8 closed, original G2 12 limited complete/0bind in progress/15pending unchanged. This closes only the finite optimization experiment, not G2.15. Next return to Owner remote known gaps using saved read comparison and existing tools, select one different supported source cost; no parallel-checksum rerun or more qualification matrix. Strong durable Moose write comparison/full targets remain pending; DFS write/A-ctl maintenance remain user-deferred, no human decision pending.
