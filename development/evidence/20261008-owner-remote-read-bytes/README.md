# G2.14: retained Owner READ decode-copy reduction

**Function: limited PASS. Measurement: completed diagnostic pair. Throughput improvement: retained. Original pure-read latency gate: NOT_PROVEN. Final matched MooseFS dual target: PENDING.** G1 historical8/8 and original27 G2 items12/0/15 are unchanged. The [b80 finite bind ON trial](../20261008-workspace-bind-on-trial/README.md) and previous [DFS cache improvement](../20261008-dfs-read-version-cache/README.md) retain their original identities.

## Product change

The public tonic/prost generator maps only `OwnerReadReply.data` to owned `prost::bytes::Bytes`, avoiding the protobuf payload Vec allocation/copy at decode. Home converts its existing owned Vec without copying. RDMA keeps an owned buffer moved into the blocking worker; its lifetime, capacity, session, poison and checksum checks remain. Client reply shape/checksum checks and final copy-out, worker scheduling, per-read authorization, freshness, permissions and error propagation remain unchanged. No protobuf schema, dependency, third-party source, bind core or container adapter change.

The code is in `common/protocol/build.rs` and `src/node/rpc/data.rs`; client/integration fixtures only adapt the payload type. Two new regressions in `common/protocol/tests/owner_read_payload.rs` prove bidirectional legacy wire compatibility and full1MiB decode sharing/owned lifetime. Sharing failed with the previous Vec implementation, then passed. The generator uses its authoritative second node-data pass. [Official public Config::bytes API](https://docs.rs/prost-build/0.14.1/prost_build/struct.Config.html#method.bytes); exact installed tonic-prost-build0.14.6/prost0.14.4 decoder sources were read without modification.

## One actual pair

[Saved-data result](paired-result.json), same fixture and physical inode; ctl local-file Meta, B real ext4 Home workspace bind ON, C remote Owner FUSE uid501, gRPC/mTLS,64MiB/C1/logical1MiB. One warmup + five formal rounds per phase, serial current-main baseline then candidate. Physical full SHA and mincore64MiB hot before every round; remote full SHA before every round, EOF/length, native4KiB mutation followed by remote fresh-open visibility, EACCES/ENOENT and complete content passed. Default client cache remains0 resident bytes in the probe; this is not an equal-cache or qualified MooseFS comparison. Only B/C Node ELF changes; same Meta/config/TLS and original physical file remain.

| Metric | a734 baseline | Bytes candidate |
| --- | ---: | ---: |
| Median complete I/O task throughput, MiB/s | 277.005041 | 365.215247 |
| Actual interval p50, ns | 3821283 | 2426548 |
| Actual interval p95, ns — preselected | 6012909 | 4969932 |
| Actual interval p99, ns | 7187724 | 5933076 |

Throughput improved **31.8443%**. Actual wider-interval p95 fell **17.3456%**; each phase has320 independent operation intervals, pooled nearest-rank p50/p95/p99. CLOCK_MONOTONIC whole-task timer includes open, worker setup/join, read/content checks and close; allocation/cache preparation and result serialization are outside. No latency is inferred from throughput.

**Timing correction and decision:** the frozen plan incorrectly said the read interval excluded content oracle. The unchanged, SHA-fixed probe actually times `pread+count+content-check`; its source and every raw record agree. The raw harness's numeric >=1.05 throughput / <=1.0 p95 result is preserved, but **the original pure-read latency retention gate is NOT_PROVEN**. We retain the directly measured throughput improvement and tested correctness; the wider-interval latency is diagnostic only. This corrects the scope, without choosing another percentile, threshold, sample or order after seeing results. No rerun to polish data; no pure-read or final MooseFS1.2/.8 PASS. [Original contract plus correction](frozen-exit.json), [boundary error](timing-boundary-correction.json).

## Version, affected validation and closure

[Candidate identity](candidate-identity.json): main basea734091cb866869e6ead0cde553e6129be88250b + full patch951e9c760e9bef1b3a0f723f260dfa1c4a0f8f692566b5dadf0be6aae10bf368,159 compiler inputs map3a22539290f20f24e3c6373f00c34eb78ab94245f90b6d436c42e9d7a001df36,Node8e1216f4075deb48bc05c921dbfca1337a2180c78d693f5d9923736d83382a30. Baseline current a734 Nodec8bb82afef20f88918521eaeda4dadcc03a99862828835f767fc43bcd14cccfb matches all158 original main compiler inputs. Meta remains b80 SHA15648a8753bfe240fce15c24b673057dea6b97e04f82abee6eb6c5a348d086e2. Full patch identity was expanded to include the new test before startup; no built input bytes changed.

[Linux validation](linux-validation.json):2 protocol regressions,5 read-service,3 real RPC payload/malformed/write-contract,3 real Home+mTLS and2 RDMA admission tests:15 disjoint tests PASS. fmt --check, default/RDMA lib+bin check, affected lib+bin Clippy, protocol-test Clippy and release Node build PASS, with2 unchanged fallback dead_code warnings. No full POSIX, new Meta recovery, device/RDMA runtime, new comparator or packaging claim. Wire/schema unchanged does not automatically relabel historical acceptance to this ELF.

[Closure](runtime-closure.json):6 exact child wait0 events, bound supervisors gone (no supervisor-wait claim), owned bind/FUSE/UDS closed. Complete original mount inventories and3 protected processes on affected roles remain unchanged, independently [rechecked](post-protected.json). A was untouched. Sampled final aggregate459419648B <640MiB; per-role ceiling/floor checks passed, no continuous-peak claim. [Raw logs](raw-logs.json) retain8099B,3ERRO/20WARN/0TRACE: one unregistered-root lookup during bootstrap and two expected missing-file lookups; no zero-error claim.

## Recoverable evidence and next item

[Archive and actual Linux restore](archive-index.json):1353 raw files +27 nested guest records,208686B, SHA2be026a5f807308d55b75e978fd5677670d09e89e2829cdff876cbadd5a1764a. Raw commands, first failures, original contract, compiler identity, patch, case adapters, measurements and lifecycle records are outside Git at `evidence/afs-delivery/owner-remote-read-bytes-20261008-r1`; archive at `evidence/archives/20261008-owner-remote-read-bytes/records.tar.gz`. No ELF/private TLS/full source or repeated tool snapshots in this compact source index. [Tool provenance/reused dependencies](tool-index.json), [failures and limits](failures-and-limits.json). Original sharing RED, assertion compile failure, read-only formatting/auditor output mistakes and timing error are preserved; no VM repair or product rerun.

Next independent item: G2.15 remote WRITE, inspect one actual RPC/barrier cost in the bind ON flow, avoiding the rejected owned-buffer-copy direction. Reuse unaffected finite bind/standard/recovery evidence; final remote read/write MooseFS dual targets and DFS3FS parity remain pending. No new package cycle, ordinary local FUSE optimization, baseline-qualification matrix or broad reliability work in this slice.
