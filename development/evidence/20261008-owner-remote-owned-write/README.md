# G2.15: reject the second-copy remote WRITE trial

**Function: limited PASS. Measurement: COMPLETE. Optimization: REJECTED, production restored. Formal MooseFS performance: PENDING.** G1 historical8/8 stays closed; original27 G2 items remain12 limited complete /0 bind in progress /15 pending. The published [b80 bind ON trial](../20261008-workspace-bind-on-trial/README.md) retains its identity and support scope.

## One issue and one formal pair

The prototype moved the FUSE-owned write buffer through first-party Backend/OwnerFs/RemoteFiles into the gRPC request, eliminating its second whole-buffer copy. It retained the first copy needed by queued FUSE work, checksum, authority, per-handle ordering, killpriv, errors and persistence; RDMA still borrowed the same bytes. No third-party code changed. This was a cost hypothesis, not a claim that copying dominates remote writing.

Before running, retention required **>=5% median throughput improvement AND no independent pooled-p95 regression**, with correctness and normal closure. One warmup and five formal rounds per phase were run exactly once. [Raw result](paired-result.json):

| Metric | b80 baseline | owned-write prototype | Candidate / baseline |
| --- | ---: | ---: | ---: |
| Median throughput, MiB/s | 190.407079 | 187.685204 | 0.985705 |
| Independent write p50, ns | 4752797 | 4928215 | 1.036908 |
| Independent write p95, ns — preselected | 6861693 | 6873420 | 1.001709 |
| Independent write p99, ns | 8136771 | 7911533 | 0.972921 |

Throughput declined **1.4295%**, p95 increased **0.1709%**. The trial fails its retention line: all production changes were removed, without another performance run. Main retains only one meaningful borrowed-path gRPC request-contract regression and its existing test-stub request recording. It verifies offset, length, bytes, checksum, killpriv and short-write success metrics; malformed-reply coverage already exists. This round claims **no product performance improvement**.

## Fixed conditions and correctness

New ctl local-file Meta / B physical Home workspace bind ON / C remote FUSE fixture, uid/gid501; one existing64MiB file, C1,1MiB sequential in-place overwrites, fdatasync. Serial baseline then candidate, same physical dev/inode/size, exact configs/TLS/resources and unchanged Meta; only B/C Node ELFs replaced. Order/noise remains a limit. Before every round, B reset the whole file to byte98 and fdatasync outside timing; C fresh-open full SHA98 and B physical mincore64MiB hot passed. Timed C write changed it to byte97, followed outside timing by B physical full SHA97 and C fresh-open full SHA97/EOF. This is actual content transition, not writing the same bytes repeatedly. mtime/ctime were recaptured after intentional mutations; inode/device/size remained fixed.

The unchanged C tool records **pwrite+count-check** intervals independently of throughput. Each phase has320 formal operation intervals, nearest-rank ceil(p*N/100)-1 for p50/p95/p99, with p95 selected before timing. Wall time includes open/thread create+join/IO/fdatasync/close; full content checks are outside it. Native4KiB mutation/restoration visible on C fresh open, uid502 write EACCES13, uid501 missing ENOENT2, length and EOF passed. This limited fixture is not full POSIX or crash recovery.

[Closure](runtime-closure.json): all six owned children actually waited0; supervisors disappeared, with no supervisor wait claim. Bind/FUSE/UDS closed, complete original mount inventories and protected processes retained. Sum of maximum sampled case allocation408862720B <640MiB; per-role ceilings/reserves passed. This is sampled allocation, not continuous peak. Exact self-owned transfer uploads94141286B were SHA-verified against retained host originals and removed; stopped guest state/data/logs remain. Logs10161B contain3ERRO/30WARN/0TRACE, preserved with no zero-error claim.

## Identity and verification

[Prototype identity](candidate-identity.json): base df26b1c3e52cb8561aa4b71a8617f56367a31b18 + patch dbbf8f2d538104fe587fbc12b6d2789e7765d5b6a78bcfeb3711e9ba1305ff35;158 compiler inputs/map2efdb8fb44f3a6f5aafb2eab2d3164e2ca9c173d81ca3e75642a6b68026ab67b; Node SHA5104ffe9824d4715801be960e74bf1f8d1f5faacb584f1d26ebe7e577a4c0994. Baseline b80 Node SHAb3335fb2 and unchanged Meta SHA15648a87 retain their original identities. Raw closure source labels distinguish baseline and prototype. Final test-only main is not relabeled as the measured prototype or automatically granted its runtime conclusions.

[Linux validation](linux-validation.json): prototype OwnerFs affected filter87PASS/1 pre-existing ignored, actual gRPC filter3, default owned fallback1 and FUSE same-handle ordering1; overlapping individual filters are not extra tests. Compiler identity, fmt/check, affected lib/bins Clippy and one release Node build passed. After rejecting the prototype, final borrowed gRPC filter3 (including the one new test), fmt/check/lib-bins Clippy passed; two pre-existing fallback dead_code warnings remain. No new release build/package/install cycle followed the test-only restoration. [Saved-data/source audit](audit.json) recomputes timings and confirms production bytes before the test module exactly match the base, and the other four production files are unchanged.

[Failure and limit index](failures-and-limits.json) preserves the initial fmt failure and three closeout-tool errors: wrong raw closure schema, attempted output on the read-only Linux share, and missing same-case temporary admission destination. They were corrected only for saved-data audit/collection; no AFS performance rerun or environment repair followed. [Archive](archive-index.json) actually restored and verified1320 raw records plus71 nested guest files in Linux; patch, commands, all failures and negative data remain recoverable outside the source tree. Prototype ELF stays separately SHA-bound; no whole tool/source snapshot was added to Git.

Historical [6d write data](../20261007-owner-remote-write-small/README.md) retain their original OFF/A-reader conditions, cache/strong-durability baseline and latency qualification gaps. They are not this pair's comparator. Current **>=1.2x throughput AND <=0.8x independent latency versus qualified MooseFS** remains PENDING. No new MooseFS or3FS run was made.

## Next independent item

Return to DFS one-writer/many-readers. Reuse the closed [single per-read measurement](../20261008-dfs-read-intervals/README.md), content/three-copy/normal recovery evidence by identity; inspect one concrete product read-path cost and attempt only a bounded product optimization. Do not reopen the missing measurement, fish for this rejected WRITE result, polish local FUSE or expand the deferred3FS qualification matrix.
