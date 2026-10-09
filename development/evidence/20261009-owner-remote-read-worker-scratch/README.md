# G2.14: queued Owner READ buffer candidate rejected

2026-10-09. One frozen small comparison, no retry. Function **LIMITED_PASS**, measurement **COMPLETE**, optimization **REJECTED**; no retained production change or new trial. G1 historical 8/8 stays closed, original G2 counts stay 12 limited complete / 0 bind in progress / 15 pending.

## Result and decision

| Metric | Baseline b62 | Candidate ea225 | Candidate / baseline |
| --- | ---: | ---: | ---: |
| Median MiB/s, five formal rounds | 406.196257 | 406.470567 | 1.000675313 |
| Independent pread p50, ms | 1.950704 | 1.881079 | — |
| Predeclared pooled p95, ms | 5.403029 | 5.257613 | 0.973086208 |
| Independent pread p99, ms | 6.830692 | 6.611860 | — |

The pre-run retention gate required throughput >=1.05 and p95 <=1.0, plus correctness/closure. Throughput improved only 0.0675%, so the candidate was rejected despite p95 improving 2.69%. All five rounds, 320 formal operation intervals per side and 128 excluded warmup intervals are preserved. Fixed baseline-then-candidate order is disclosed; no general stable improvement claim.

The [official Moose READ result](../20261009-owner-remote-direct-pair/README.md) remains its original d14 identity: throughput 0.914140415 FAIL, p95 0.538793817 PASS, conjunction FAIL. This experiment did not run Moose and must not be spliced into that comparison. WRITE strong-ACK qualification and DFS 3FS qualification remain separately deferred.

## Frozen scope and identities

Base source `53e33587ab4dbce92c4a62ace608fb2137acbd9b`; B physical ext4 Home workspace bind ON, C remote Owner FUSE, independent ctl local-file Meta. B b62 and Meta650 stayed the same live PID/start/executable/config incarnation throughout; only C switched to candidate ea225. Six boundary checks prove the fixed endpoints.

Same original 64MiB pattern97 file, inode/device/mtime/ctime, uid/gid501 and mode0600, SHA256 `fae972222d455a2eaee1661ad9625502ec3bfc5ec38b87a6eec5afd5107331b5`. O_RDONLY/pread, concurrency1, 1MiB blocks, one warmup plus five formal rounds per side. Throughput uses whole probe wall time including open/thread/oracle/close; operation latency independently measures actual pread plus returned-count check, excluding content oracle. Nearest-rank pooled p95 was fixed before execution; p50/p99 retained. Physical backing is hot by full SHA preload and mincore pre/post each sample; client residency is explicitly unobserved, default direct FUSE, no cache tuning/drop.

Full ELF/source/compiler maps and frozen contract/controller hashes are in [result.json](result.json). Candidate ELF `ea225122f6868de9a6b5121f2ada68f981f8934ab6d824d13ebd3b5e5d230a74` is rejected and never enters the trial. Restored production FUSE SHA `bf622b05130b2f3a446087659834d875b8e837942f3087bca40f19793499d7d1`, 161-input map `47de5b82f0e9d25becfed092bc721254eba0564fcb3cbd270d51f61a6387ee08`, identical to 53e. Existing [G2.09 local READ improvement](../20261009-owner-local-read-scratch/README.md) remains. DFS, RPC, backend, vendor and published d47 ON package unchanged.

## Verification and preserved failures

Linux ARM64 only: unchanged baseline 23/23 affected FUSE tests; candidate 27/27 including fresh read/short/EOF/error, nested buffer fallback, separate OS threads and oversized/caller-context constraints; fmt/release Node build exit0. Initial test-only unused_mut warning and original outputs retained, then fixed before final candidate. Strict Clippy exit101 from existing peer.rs dead_code (336/1252); supplementary `-D warnings -A dead_code` exit0 is distinct, not a strict-lint PASS.

One formal runtime passed full content, EOF, uid502 EACCES, missing ENOENT and real remote 64MiB counters each round. Four actual lifecycle wait0, original data/permissions, mounts and protected processes restored. Original executable/helper inode/mode/uid/gid/mtime/SHA and run/log inode/content restored; executable ctime naturally changes on rename and is not claimed unchanged. Candidate source and its four extra tests survive in the external patch only; all production inputs restored exactly, unchanged baseline tests reused without rerunning.

Parent's first saved-data identity check confused canonical compiler-map SHA with serialized-map artifact SHA and failed before runtime. Corrected explicit map_sha256 plus all161 input bytes passed. Both records remain; no VM repair or product retry. Validation receipt is intentionally a build-time snapshot with runtime NOT_RUN; later runtime/closure records prove the single execution, without retroactively rewriting the earlier receipt.

## Raw evidence, recoverability and capacity

Canonical raw root outside Git: `evidence/afs-delivery/owner-remote-read-worker-scratch-20261009-r1/` under the research workspace. Git stores this index, compact result and checksums only; no script/source snapshots, ELF, raw sample copies or archives.

- `preservation/current-evidence.tar.gz`: 47,878,684B, SHA256 `86eb5eccde4647d472a3507fb70bb9c7ade61379c358746f9656e4a9a458d87f`, 1,426 regular files / 75,348,176 uncompressed bytes. Includes negative patch/source/ELF, frozen runner/contract, all raw results and original failures. Seventeen existing dependencies remain pinned by reference instead of copying previous trees.
- `preservation/final-receipts.tar.gz`: 286,994B / 89 regular files, SHA256 `ece3075a7a66fd88dff5facb8fd8a72817f14d2385c0d7a9d85f18273e2b688b`, closing restore/cleanup commands and receipts.
- Actual Linux extraction checked all1,426 regular files plus nested58 guest files / 91,308,169B, hash and mode/uid/gid. Linux audit from extracted data independently recomputed 640 formal intervals, excluded128 warmup, fixed B/Meta identity and four normal exits. Only then removed four current-run guest ELFs totaling91,159,864B; retained54 compact guest files /148,305B. Removed redundant host Meta15,808,368B after preservation; rejected Node and patch remain archived. Historical evidence and services untouched.

| Role | Final allocated fixture bytes | Free bytes | Ceiling / free floor |
| --- | ---: | ---: | --- |
| ctl | 105,086,976 | 1,963,778,048 | 256MiB /512MiB |
| B | 216,104,960 | 21,246,214,144 | 512MiB /4GiB |
| C | 148,799,488 | 36,873,433,088 | 512MiB /4GiB |

Host free83,480,555,520B above predeclared51GiB reserve. Product logs3,588B /0ERROR /0TRACE /5WARN in this run: one unknown opcode52, four unimplemented ioctl21505; not a claim of global zero errors. A and historical memory Meta untouched; A/ctl expansion and DFS WRITE remain user-deferred.

## Next independent exit

Stop this buffer direction and repeated remote copy/checksum/scheduling micro-tuning. Next only the existing small Moose throughput/p95 comparison for the retained G2.09 local READ change, if its existing comparator and environment are ready; no new matrix or qualification project. Remote target stays open. R2 upstream API migration stays incomplete with existing lock/cancellation semantics preserved; no pending capacity decision, no feature downgrade or new private patch. Historical standards, eight bind cases and unchanged DFS results retain their original limited identities.
