# Container workspace C1 performance diagnostic — data recorded

2026-10-07, ARM64 Linux `afs-g2-micro`, guest `/opt` ext4, local-file Meta/gRPC, official runc1.5.2. Product Rust6d51aeb/map66dbbe3e/157 compiler inputs and release package unchanged. Maintained Python and two exact first-party PR43 C payloads only. G1 historical8/8 and G2 counts unchanged; default OFF.

**Collection result:** OFF and ON each complete one warmup plus five paired measurement rounds, all fixed payload checks and normal cleanup pass. This records diagnostic data, not G2.13 or production ON qualification. Mixed locks, append offset and cross-path watch [FAIL remain](../20261007-managed-semantics/README.md).

| Case | OFF elapsed/ext4 median [range] | ON elapsed/ext4 median [range] |
| --- | --- | --- |
| write | 1.535 [1.290, 1.679] | 1.023 [0.809, 1.120] |
| read | 3.879 [2.697, 6.056] | 1.031 [0.512, 1.232] |
| create_write_close | 62.986 [60.980, 75.959] | 1.097 [0.864, 1.307] |
| stat | 36.439 [30.877, 37.687] | 1.158 [1.025, 1.341] |
| read_close | 58.785 [55.152, 61.933] | 1.112 [1.059, 1.284] |
| readdir | 64.800 [61.283, 66.289] | 1.048 [0.992, 1.119] |
| rename | 19.700 [18.684, 20.343] | 1.104 [1.080, 1.215] |
| unlink | 23.370 [22.419, 23.852] | 1.114 [1.043, 1.178] |

Ratios are paired experiment wall time / reference wall time; lower is faster. They are medians of five individual ratios, not ratios of medians. [Fixed analysis policy](analysis-policy.json), [all pairs and times](paired-statistics.json). Small n=5, cache residency unobserved; no noise/confidence, cold/hot or universal performance claim. No tuning follows this diagnostic.

## Scope and identity

- Write: C1,64MiB,1MiB blocks,pattern90, fsync API return and close inside payload timer. Read: same bytes/content, close. This is not an independent physical durability claim.
- Metadata: absolute path,1000×4KiB, six phases; close visibility only, durability unqualified. Each sample uses fresh owned paths.
- Timing runs inside actual OCI processes via runc exec. OFF uses FUSE in ordinary OCI; ON uses current Home permit and controlled native exec. Reference is ordinary OCI on the same guest ext4 volume. Host procfs reads only identity; they are not benchmark timing. Separate OFF→ON cohorts do not claim one simultaneous container incarnation.
- UID/GID501,read-only rootfs,zero capabilities,noNewPrivileges,restricted workspace flags; rootfs cloned from [eight pinned regular inputs](rootfs-inputs.json) each time.
- Exact FUSE request counts are **NOT_OBSERVED** by current Node metrics, not zero. Raw metrics/load/memory/disk are retained; this remains a formal gate gap.

[Exact outer command](driver.command.json), [exit/elapsed](driver.exit.json), [result](results-r1/result.json), [preflight](results-r1/preflight.json), [commands](results-r1/commands.json), [tool SHAs](tool-inputs.json), [source-map reference](source-map-reference.json). Package SHA `ee25d5892c4e884e67c86d4e5b9c6ab551af4c46649edab6a05da06659d1aff9`; Meta SHA `2c7b7d088b759e3b9375080002182aa484a424b4fa216da1fb79a1004e96168e`; Node SHA `2cf1f538fe7af332a711f3c66a074ace140c00773826a182709a6445b8ae2645`.

[OFF live identity](results-r1/off-running-identity.json), [ON live identity](results-r1/on-running-identity.json), actual container identities and per-round JSON under `results-r1/`; raw controller receipts/logs retained. [OFF cleanup](results-r1/off-cleanup.json), [ON cleanup](results-r1/on-cleanup.json), [independent postcheck](postcheck.json): exact mounts/sockets/locks gone, no selected AFS/runc processes, both runtime lists empty. No force/lazy cleanup.

## Tools and reproduction

[Predeclared slice](../../container-workspace-perf-slice.md), [canonical driver](../../acceptance/container-workspace-perf-linux.py), [targeted guards](../../acceptance/test_container_perf.py). [Linux guard output](linux-guards.stderr): five methods; tools passed, not POSIX. [Exact C provenance](payload-provenance.json) pins original Git80b0bca bytes; source lives only in maintained probes. [Build/ldd commands](payload-build/build/compile_io.cmd) and raw outputs under `payload-build/`; positive controls, corrupt-content rejection and reused-directory rejection under `payload-build/results/`. ELF/rootfs/package/source snapshots remain outside Git.

[Issue42 snapshot](issue42.json) and [PR43 snapshot](pr43.json) retain original requirements and open-state/head identity. No full PR merge, third-party change or Rust rebuild. Read-only final tooling/evidence review is recorded in `final-review.json`.

## Next bounded item

Retain this complete diagnostic without remeasuring or tuning ordinary performance. Return to one independent required ON functional gap: append/SEEK_CUR first, bounded current-path adaptation and targeted regression; preserve the lock/watch failures and do not reopen G1. Production READY/drain/restart, broad concurrency/reliability, large/long cases and etcd/Redis remain separately pending.
