# G2.14: current binary pair, one small remote DIRECT read comparison

2026-10-09. **Function LIMITED_PASS; measurement COMPLETE; throughput FAIL; independent p95 PASS; overall performance FAIL.** Closes this selected comparison/measurement subitem, not the original G2.14 or final G2.27 performance gate.

| Metric | OwnerFs | MooseFS | Owner/Moose | Preset gate |
| --- | ---: | ---: | ---: | --- |
| Median throughput MiB/s | 389.863367 | 426.480835 | 0.914140 | >=1.2: FAIL |
| Independent pread p50 ms | 2.052085 | 0.250838 | — | Report only |
| Independent pread p95 ms | 5.394651 | 10.012459 | 0.538794 | <=0.8: PASS |
| Independent pread p99 ms | 6.278341 | 55.068156 | — | Report only |

One warmup and five alternating formal pairs; per side320 actual pread intervals, nearest-rank pooled p95 fixed before measurement. uid/gid501,empty supplementary groups,0600 files,64MiB,1MiB operations,C1,fresh O_RDONLY pread+close. Content comparison is excluded from pread timing but included in the disclosed whole-probe wall throughput. No write/durable-ACK assertion: one B authoritative Home versus one VALID B Moose copy. Both direct FUSE and physical ext4 fully resident before/after each read; client residency UNOBSERVED. Owner mTLS/Moose plain and stock within-handle prefetch disclosed. [Frozen contract](contract.json),[all operation samples](operation-samples.json),[separate results](paired-result.json).

Current published d47 binaries: Meta650bd9714d /Node d14e318226,160compiler-input map ec7e2de5. **Actual profile Owner-only**, B real ext4 Home workspace bind ON/C remote FUSE/ctl local-file. Existing combined-profile functionality remains separately proven by [the previous regression](../20261009-workspace-bind-remote-d47/README.md), not retested or upgraded. [Linux exact input proof](linux-inputs.json).

Official Moose4.59.2-1/build2106 server/client SHA fixed. New root-owned0755 copies identical to old uid501 official installation; originals unchanged, public CLI only. Test-only adapter uses isolated paths for both admission and actual launch, checks root-owned non-writable path/ELF/dependencies/live child and inherits foreground pidfd/true-wait lifecycle. [Stock identities](stock-identities.json),[9 Linux root contract tests](linux-targeted-tests.json). No product or third-party source changes, no performance-improvement claim.

Earlier root-owned payload was not uid501-readable. That unmeasured run stopped normally; a unique new baseline file owned501/0600 inherited one-copy policy for the sole formal comparison. Original baseline file and original Owner64MiB data/inode/checksum preserved; all failures retained. Each read proves Home application payload exactly64MiB or dedicated mfsmount→B socket payload64–66MiB with address/inode/cookie/owned PID checks. Full SHA/EOF passes. [Failures and boundaries](failures.md).

Six actual wait0 in the formal run plus six from the unmeasured failure; protected process/mount/boot inventories and original helper/Meta inode restored. [Closure](closure-summary.json),[94 independent Linux saved-data checks](audit.json). Two raw directories archived as2005 files+one alias+index,763600B; actual Linux extraction/checksums and published Meta recovery pass. [Archive](archive.json),[restoration](archive-restoration.json),[compact index](index.json). After restoration, six exact temporary Meta copies reclaimed94,850,208 logical bytes; logs, historical records and reusable baseline payload remain. [Cleanup](cleanup.json).

Next: one source optimization for the demonstrated remote-throughput gap, with the same correctness and frozen comparison; no qualification expansion or repeat until a substantive candidate exists. Remote write's qualified Moose durability comparison remains pending; DFS write/user-deferred A/ctl maintenance remain deferred. Historical G1 8/8 and original G2 12/0/15 unchanged; eight bind>=.90/ext4 results retain original versions; no new package.
