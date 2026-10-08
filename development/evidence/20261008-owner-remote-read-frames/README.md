# G2.14 OwnerFs receive-frame improvement — 2026-10-08

**Function LIMITED_PASS; measurement COMPLETE; improvement RETAINED; final MooseFS dual target PENDING.** G1 historical8/8 stays closed; original G2 stays12 limited complete/0 bind in progress/15 pending. This closes this bounded optimization and its functional/measurement subitems, not the entire G2.14 or delivery milestone. Existing b80 ON trial retains its own binary and support scope; no new package.

The [preceding existing-RPC diagnostic](../20261008-owner-remote-read-rpc-cost/README.md) recorded matching128 RPCs/64MiB: Home mean0.068953ms, client mean1.252815ms. Home is5.50% of measured client RPC time; subtraction is a matched-count mean, not wire time or percentile subtraction. Current windows were already2MiB, so no default65KiB-window premise or window change. Request-size split remains unproven; no probe-alignment change or cache relaxation.

## Product change

`src/node/rpc/peer.rs` adds an OwnerFiles cached-channel profile advertising256KiB maximum received HTTP/2 frames via the public tonic0.14.6 `Endpoint::max_frame_size` API. `src/node.rs` selects it for OwnerFiles; ordinary DFS and no-deadline lock channels retain their existing profile. Both profiles share the bounded pool and highest-epoch map; epoch advance evicts all stale profiles. The setting permits larger frames; this experiment did not capture actual frame sizes and does not prove that framing alone accounts for every observed gain. It applies to the OwnerFiles channel, including its metadata/write RPCs; this slice establishes a READ performance gain only.

mTLS/CA/server-name, timeouts,2MiB flow windows, message limits, per-operation grants/Home/root/epoch checks, handle and release ordering, checksum/shape validation, direct-I/O/mmap negotiation, freshness, permissions and errors remain. No third-party, wire schema, dependency, bind core, container adapter or durability change.

## Frozen small pair

[Contract](frozen-contract.json), [plan](frozen-plan.md), [identity](candidate-identity.json): source d79bf7c4+patch7bb394a3,160 compiler inputs/map81c30c95; candidate Node839e14d9. ctl local-file Meta/B real ext4 Home workspace bind ON/C remote FUSE uid501, gRPC/mTLS,64MiB/C1/logical1MiB. Retained8e1216f4 baseline; one warmup + five measured rounds each, serial baseline→candidate. Same payload/inode/config/certificates; physical mincore64MiB hot before every round and complete physical/remote SHA. Client cache remains its existing direct-I/O policy; this is not a qualified MooseFS equal-cache comparison.

| Metric | Baseline | Candidate | Candidate/baseline |
| --- | ---: | ---: | ---: |
| Throughput, median MiB/s | 389.558922 | 444.926519 | 1.142129 (+14.21%) |
| Independent operation p50, ns | 2371784 | 1811901 | 0.763940 |
| Independent operation p95, ns | 4564690 | 4368813 | 0.957089 (-4.29%) |
| Independent operation p99, ns | 6926307 | 5887752 | 0.850056 |

Each phase has320 actual operation intervals. CLOCK_MONOTONIC `pread+count-check` stops before content memcmp; whole-task throughput still includes oracle/open/worker setup/join/close. p95 selected before execution, pooled nearest-rank p50/p95/p99 and full arrays retained. Numeric retention rule>=1.05 throughput AND<=1.0 p95 PASS; no rerun, no post-result metric selection, no final MooseFS1.2/.8 claim. Previous oracle-inclusive measurements retain their original boundary and conclusions.

[Saved-data audit](result.json):46 checks independently recompute intervals/throughput/gate; full SHA/EOF/errno/permissions, native mutation→remote fresh open, bind source≠target on ext4, same physical inode, six actual child wait0, disappeared PIDs and full original mounts/three protected processes PASS. No A activity, capacity repair or affected background service interruption.

[Linux validation](linux-validation.json):8 distinct tests PASS including profile isolation/cross-profile epoch fencing, existing eviction fencing, real Home mTLS full1MiB read/write, permission/identity/release, long wait beyond ordinary5s deadline and malformed reply/write contracts. fmt, default/RDMA lib+bin check, affected lib/bin/integration Clippy and release build PASS. Two historical fallback dead_code warnings and one existing fixture-ctor warning remain; not strict all-target clean/full POSIX/device RDMA/new recovery/new delivery.

[Archive](archive-index.json):1319raw+27guest actual Linux recovery PASS;191817B archive SHA9a0768ad5ccbe87ceb5b4933e4a7d82cdd865f9aac2aaba67970c6af11ba7030. Full map/patch/ELF/tools/raw measurements stay outside Git, compact identity/result/index only here. [Logs](raw-logs.json) retain8105B/3ERRO+20WARN; semantic success does not erase logged failures. [Provenance](provenance.json). Earlier [owned-output rejection](../20261008-owner-remote-read-payload/README.md) and [inline rejection](../20261008-owner-remote-read-dispatch/README.md) remain and will not be rerun.

Next independent item: G2.15 remote WRITE/barrier request cost using retained data and existing metrics; choose one product issue, preserve persistence. DFS write comparison remains deferred at the user's choice due A capacity; no new qualification matrix, ordinary local FUSE optimization, packaging cycle or complex reliability prerequisite.
