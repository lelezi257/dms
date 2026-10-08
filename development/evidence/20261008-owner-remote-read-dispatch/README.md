# G2.14: reject a remote READ scheduling trial below its retention line

**Function: limited PASS. Measurement: COMPLETE. Product optimization: REJECTED, production scheduling restored. Formal MooseFS performance: PENDING.** No original G2 task is newly complete: 12 limited complete / 0 bind in progress / 15 pending, 27 unchanged; G1 historical 8/8 stays closed. The [b80 ON trial](../20261008-workspace-bind-on-trial/README.md) retains its original identity and support scope.

## One principal product hypothesis and result

The proposed first-party change ran GrpcInline READ work via `block_in_place` on a multi-thread Tokio runtime, with panic-to-domain-error mapping and current-thread `spawn_blocking` fallback. It tried removing one queued blocking-worker handoff; it did not change remote caches, authority, permissions, FUSE, RDMA, persistence or third-party code. A direct runtime-blocking draft was corrected before the identified candidate build. This is a hypothesis about one scheduling cost, not proof that it dominates the remote gap.

Before measurement, retention required **at least 5% median throughput improvement and no pooled-p95 regression**, plus correctness/freshness/errors and normal closure. [Actual results](paired-result.json):

| Metric | b80 baseline | proposed scheduling | Candidate / baseline |
| --- | ---: | ---: | ---: |
| Median throughput, MiB/s | 298.373605 | 305.725474 | 1.024640 |
| Application read p50, ns | 2833957 | 2725867 | 0.96186 |
| Application read p95, ns — preselected | 5772631 | 5393811 | 0.934377 |
| Application read p99, ns | 7217069 | 7893537 | 1.09373 |

Throughput improved only **2.464%**, below the fixed retention line. The direction is closed without rerunning to fish for a result; the production change is removed, and useful read/error regression tests remain. This round leaves **no claimed product performance improvement in main**. Sample ranges overlap; the lower p95 is an observation in this one pair, not a general latency guarantee.

## Frozen conditions and actual correctness

One newly initialized fixture: ctl local-file Meta, Home B physical workspace bind ON, C remote FUSE, uid/gid501; serial baseline then candidate with only B/C Node executables replaced. Same Meta, exact configs/TLS and physical 64MiB inode/device/size. Each phase used one warmup plus five measured C1 rounds, 1MiB sequential application reads, byte97 and close barrier. Every sample first read the full C file SHA then the physical B SHA; mincore confirmed the entire physical range hot with stable identity during measurement. The phases intentionally did not alternate, so order/noise remains a limit.

The unchanged C tool independently measured 320 intervals per phase. Wall timer includes open, thread creation/join, reads and close; latency intervals are **pread + count + content check**, not bare syscalls or RPC intervals. Reported pooled p50/p95/p99 use nearest-rank `ceil(p*N/100)-1`; p95 was selected before timing. Client residency used the tool's unchanged `repeat` observation policy, without changing the product cache policy or treating it as physical hotness.

Native 4KiB mutation and restoration were seen by remote fresh opens; full64MiB SHA, length/EOF, uid502 EACCES13 and uid501 missing-path ENOENT2 passed in both phases. [Runtime closure](runtime-closure.json): six owned child lifecycles actual wait0; their supervisors' incarnations disappeared (no supervisor wait claim), bind/FUSE/UDS vanished and original complete mount inventories/protected processes were retained. Final sampled case allocation totaled409178112B below640MiB, with ctl512MiB and B/C4GiB reserves; this is sampled allocation, not continuous peak. Temporary transfer files and logs are separately indexed in the raw closeout.

## Version and verification boundaries

[Candidate identity](candidate-identity.json): base35ae8943 + patchc29a3cc7, 158 compiler inputs/map163d1c4a, proposed Node SHA6e2a8521. Baseline b80 Node SHAb3335fb2 and unchanged Meta SHA15648a87 are linked to the existing [b80 evidence](../20261008-workspace-bind-remote/README.md). The rejected patch and ELF remain outside Git and recoverable through the archive index. The final source contains tests but restores the original production READ dispatch; it is not relabeled as the tested prototype.

[Linux checks](linux-validation.json) retain six prototype unit tests, two real gRPC payload tests, two real Home handle/session tests, fmt/check/affected lib-bins Clippy and the one release build. Strict lib/tests Clippy fails on unchanged peer.rs `dead_code` and `items_after_test_module`; the failure and two existing default lib/bins warnings remain. Final post-revert tests are separately recorded. No full POSIX, G2.12 recovery rerun or new package is claimed.

Raw candidate closure observers kept a legacy b80 `source_commit` label. That label is explicitly not the proposed source identity: the observed executable SHAs, parent input map and patch above bind the prototype. Original raw records are preserved, with a correction in the closeout index. Two pre-service path/transfer errors and their ineffective stop attempts occurred before any AFS service existed; they are preparation failures, not measured rounds. All initial unit/compile/pipeline/strict-Clippy failures are retained. [Failure/limit index](failures-and-limits.json) records8147B logs with3ERRO/20WARN/0TRACE; no zero-error claim. [Independent saved-data/source audit](audit.json) recomputes both medians and pooled percentiles and confirms the production prefix is unchanged.

The historical [f03 MooseFS comparison](../20261008-owner-remote-read-backing-hot/README.md) remains throughput0.026616x / p95 28.332518x, FAIL under its original OFF/A-reader conditions. It is not the comparator for this ON/C-reader product pair. The current >=1.2x throughput **and** <=0.8x independent-latency MooseFS target remains pending; no new MooseFS or 3FS run was made.

## Next independent exit

G2.15 remote small writes: reuse the existing functional and delete evidence by version/scope, inspect one actual write-path bottleneck against retained measurements, and optimize only a concrete product issue. The previous write barrier/comparator qualification gap stays explicit; do not expand an unbounded matrix or return to this rejected read scheduling trial. DFS one-writer/many-readers follows, ordinary local FUSE remains later. Archive/restore and raw command identity are in [archive-index.json](archive-index.json).
