# G2.09 local inline READ scratch — 2026-10-09

**Limited function PASS; measurement COMPLETE; optimization RETAIN. Candidate MooseFS acceptance remains PENDING.** The historical f03 local comparison remains FAIL under its original version, cache policy and timing boundary. G1 historical8/8 and original G2 counts12/0/15 stay unchanged. No new trial package or release was produced.

## Product change and scope

Only `src/node/fuse.rs` changes. Existing local Owner O_RDONLY inline reads reuse initialized per-mount working storage up to1MiB, then synchronously reply with this read's returned prefix. Requests above1MiB retain the original temporary-allocation path. Every request still invokes the fresh backend read; this is not file-content caching. Offset/handle/inode ordering, root authorization, errno and short/EOF behavior remain. No unsafe, dependency or third-party changes.

The noninline queued read closure is byte-identical to base9fe4. Eligibility still requires `ownerfs.is_some()` and `is_local_inode()` at open; remote Owner and DFS do not select it. Physical workspace bind accesses bypass this FUSE callback. Their prior evidence remains under its own identity; this is source-impact reuse, not a current candidate remote/bind/DFS runtime PASS. [Published-current DFS reuse map](../20261009-dfs-manyread-reuse/README.md) remains specifically d47/c8bb.

## Frozen comparison and data

B only, existing stopped local-file/R1 combined fixture, both bind switches OFF; same original64MiB inode,0644/root ownership and SHA. FUSE/POSIX/O_RDONLY,1MiB,C1, one warmup plus five formal reads per phase. Exactly one formal baseline→candidate pair; no outcome-based retry. Full physical preload/checksum outside timers, physical64MiB mincore snapshots before/after; client default residency reported separately. Probe c87 measures `pread+count-check`; content oracle is outside each operation interval, while whole-wall throughput includes application overhead. It is not DFS complete-read-v1 or the old f03 oracle-inclusive timer.

Predeclared retention: median throughput≥1.05 baseline **and** pooled nearest-rank p95≤1.0 baseline, with correctness and closure. Each phase has320 raw operation intervals. Full arrays and all rounds remain in the external archive, not duplicated tools in Git.

| Metric | Baseline d14 | Candidate b62 | Candidate/baseline |
| --- | --- | --- | --- |
| Median MiB/s |1509.863775|1762.704160|1.167459|
| p50 ms |0.515708|0.433666|0.840914|
| p95 ms |3.735040|0.824708|0.220803|
| p99 ms |5.395538|1.508457|0.279575|

**Limits:** fixed phase order and one small fixture; baseline formal1 was482.899229MiB/s versus1426–1601 in the other four rounds. It remains included in pooled latency and all statistics. The results support the prospective retention decision, not a stable universal improvement or a new MooseFS1.2/.8 verdict. No comparator was run.

## Linux validation and closure

Original-allocation semantic tests3/3 passed before optimization. Candidate affected FUSE tests23/23, fmt and release afs-node build passed on existing ARM64 Linux builder. Five new regressions cover fresh short/smaller/zero/EOF reads, errors after success, reported-length clamping, initialized reuse/bounded growth and oversized success/error without scratch retention. Strict Clippy failed on the two existing `peer.rs` dead-code warnings; the supplemental run allowing only dead_code passed. The strict failure remains a gap, not a strict-lint PASS.

Runtime full SHA/short/EOF/zero-read and12 probe content checks passed. Current Meta, baseline Node and candidate Node each had actual wait0; original26 mounts, stopped process inventory, binary/config/data inode+SHA+ownership and original run/log records restored. Meta local-file state may normally advance. Snapshot peak301,535,232B<512MiB; minimum free21,180,289,024B>4GiB; logs below64MiB. Snapshot observations are not continuous peak/cache pins.

Four admission failures preceded all product launches: staged execute bits; host JSON rounding of nanosecond integers; invalid ps `--ww`; and ldd's Bash interpreter identity. Each failure remains separately archived with zero product launches/measurements and closurePASS. Corrections were limited to own staging and runner; exact product identity checks and criteria were preserved. This is a disclosed repeat of preparation admission, not four product runs. One saved-data audit parser failure is also retained in the audit record; it did not rerun products.

## Identity, recovery and external evidence

[Compact exact summary](summary.json) binds source, compiler maps, candidate patch, receipts, all aggregates, failures and cleanup. Base source9fe4eec8492cd8ed01aa71ddf6bd98d6f410fcf7; compiler map47de5b82f0e9d25becfed092bc721254eba0564fcb3cbd270d51f61a6387ee08; Node b62ade56fb2b06e3110b9c762c2393c5f1ef5c23b0a64123f2fa7702f11bf50b. Meta remains650bd9714da0293c61b6aff80ce86aede1152a233edb1506ed2d97d5b53f3e58.

External host root: `evidence/afs-delivery/owner-local-read-scratch-20261009-r1/` beneath the research workspace, outside source Git. Runtime archive `owner-local-read-scratch-20261009-r1.tar.gz`,23,033,751B,SHA256 `9af266312f4a47484d67d5b07ef6097776d3276255e63f36962bc7a618784d35`. Actual Linux extraction verified528 files and exact byte/hash manifest before cleanup. Four this-run ELF/probe copies66,124,832B and the VM archive copy were removed only after host archive verification; original fixture/data/history remain. B free after cleanup21,246,324,736B. No A/ctl/C runtime, VM restart or environment rebuild occurred.

Reproduction command (original staged paths and immutable inputs are in archive `results/exact-inputs.json`):

```text
sudo /usr/bin/python3 -B STAGE/run-linux.py --run --identity-sha256 74467d0276f7f8b7c770ca546c8a4c1ee1977f52f55e083da61a91d08e440c8d
cargo test --release --locked --offline -p afs --lib node::fuse::
cargo build --release --locked --offline -p afs --bin afs-node
```

Do not rerun a consumed results directory or restore old product runtime just to reproduce these numbers. Restore the archive first and establish a new explicit case if a later real change requires measurement. Next return to the existing Owner remote/bind priority; retain this finite local result without expanding its matrix or triggering another complete package cycle.
