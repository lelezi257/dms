# G2.15: inbound WRITE decode-copy candidate rejected

**Function limited PASS; one measurement COMPLETE; performance retention FAIL; candidate REJECTED and production restored. Final matched MooseFS dual target PENDING.** Original G1 historical8/8 and G2 original27 items12/0/15 are unchanged. [Finite b80 ON trial](../20261008-workspace-bind-on-trial/README.md), [retained READ+31.84%](../20261008-owner-remote-read-bytes/README.md) and [DFS read+16%](../20261008-dfs-read-version-cache/README.md) retain their own identities.

Only `OwnerWriteRequest.data` was temporarily mapped by the public prost generator to owned Bytes to avoid the inbound Home protobuf decode copy. Outbound `data.to_vec()` remained; this is distinct from the earlier rejected caller-owned buffer direction. Wire schema/checksum/length, per-write authorization, killpriv, handle serialization, worker scheduling, fdatasync/O_SYNC, flush/error retention and RDMA admission remained. No dependency/vendor/bind-core changes. Actual measurement failed the predeclared exit, so all four production/fixture files were restored byte-for-byte to main68dc. The committed change is only a legacy-wire bidirectional regression in `common/protocol/tests/owner_write_payload.rs`; the candidate-only sharing test/patch is recoverable outside Git.

## Frozen conditions and one pair

[Contract](frozen-exit.json): ctl local-file Meta, B physical ext4 Home workspace bind ON, C remote Owner FUSE uid501, gRPC/mTLS, existing64MiB/C1/1MiB writes, one warmup+five formal per phase, baseline68dc then candidate, same inode/config/TLS/Meta. Each round resets the physical payload to byte98 and fdatasyncs outside timing, confirms physical+fresh-remote full SHA98/mincore64MiB hot, then actually overwrites to97 remotely with finalfdatasync and verifies physical+fresh-remote full SHA97/EOF. This avoids repeated no-op content. Native mutation/remote fresh open, EACCES/ENOENT and actual ext4 source→FUSE workspace mount identity passed. No FUSE-self bind or qualified MooseFS claim.

| Metric | main68dc baseline | inbound Bytes candidate |
| --- | ---: | ---: |
| Median whole-task throughput, MiB/s | 187.467638 | 184.904391 |
| p50, ns | 4926349 | 4709468 |
| Preselected p95, ns | 6915115 | 7207540 |
| p99, ns | 8674415 | 9113678 |

[Result](paired-result.json): throughput **-1.3673%**, p95 **+4.2288%**. The fixed own-baseline retention required >=1.05 throughput AND <=1.0 p95; both failed. Keep the negative data, do not change thresholds/percentile/order or rerun. This optimization gate is separate from final MooseFS>=1.2 throughput/<=.8 independent latency, still PENDING.

Latency is independently recorded CLOCK_MONOTONIC **pwrite+exact-count-check**; post-write content checking is outside the operation interval. The one final fdatasync is separate from per-write intervals and inside whole-task open/thread-create+join/IO/fdatasync/close wall throughput.320 raw operation samples per phase, pooled nearest-rank p50/p95/p99, p95 selected before startup. No latency inferred from throughput. The prior READ timing correction remains historical and unchanged.

## Identities, validation and closure

[Candidate](candidate-identity.json): base68dcff2d4a081678d13cf2ecad08bceca34cb6c9 + patchde662eec455dd42a97ba43eab35e9df3fb7dd9f4cc824df00ae2f8e08067a1b4,160 compiler inputs mapd951e3e0dd8338f8261652ceea3121684bbb67e74330b784b3c77e2da575cb3c, Node2c99df676252787dcbd65fb93d18754101631da96522c19e1a4df1a9ae56a0b0; baseline159-map3a22539290f20f24e3c6373f00c34eb78ab94245f90b6d436c42e9d7a001df36 /Node8e1216f4075deb48bc05c921dbfca1337a2180c78d693f5d9923736d83382a30. Same b80 Meta15648a8753bfe240fce15c24b673057dea6b97e04f82abee6eb6c5a348d086e2. Frozen source audit occurred before restoration. [Production restoration](production-restoration.json) preserves all159 baseline compiler bytes, with one retained wire-test file; final160 mapf704143e7da6364bc01bfe39ddcceaae86ee96ee7d388fea6a6a137de6f54e55. No rejected candidate package or extra release rebuild.

[Linux validation](linux-validation.json):112 disjoint tests PASS (2 retained READ+2 new WRITE protocol,86 Owner VFS,14 dispatch,3 peer RPC,3 real Home/mTLS,2 RDMA admission);1 privileged Owner FUSE test remains explicitly ignored, not a PASS. fmt/check default+RDMA/clippy root+protocol/release build passed. Two empty test filters are retained and not counted; correct dispatch14 ran before runtime. Existing2 peer warnings and1 integration unused constructor remain. After restoring production, protocol3 tests/fmt/protocol Clippy PASS. [Saved-data audit and final restoration](audit-summary.json), 59 checks passed.

Six actual child wait0 events, bound supervisors gone (no supervisor-wait claim), owned FUSE/bind/UDS closed; original complete mounts and3 affected-role protected processes unchanged, [independently rechecked](post-protected.json). A untouched. Final sampled aggregate459239424B<640MiB, not continuous peak. [Logs](raw-logs.json):10164B/3ERRO/30WARN/0TRACE retained, including bootstrap/missing-file expected lookup errors; no zero-error claim.

[Archive and actual Linux restore](archive-index.json):1532 raw files+27 nested guest records, 215818B, SHA34f6eed6863d3367196e4a11f7229cf1e7f1ad35c0fc18303527f99ba48e106d. Commands, RED, full candidate patch/map, helpers/adapters, timing/content/closure and restoration are source-external; no full source/tools/ELF/private TLS in this compact index. [Provenance](tool-index.json), [failures and limits](failures-and-limits.json). No VM repair or product rerun.

Next independent item: DFS one-writer/many-readers core write/publication path, one substantive cost; reuse unaffected read, three-replica and recovery evidence. Remote final performance remains open; do not retry either rejected write-copy direction or expand comparator qualification. Ordinary local FUSE, large/long/complex reliability, multi-Meta, etcd and Redis stay later.
