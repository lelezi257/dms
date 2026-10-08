# G2.21: retain one bounded readonly version/layout cache

**Function: limited PASS. Measurement: COMPLETE. Product improvement: retained. Formal matched 3FS performance: PENDING.** G1 historical8/8 remains closed; original27 G2 items remain12 limited complete /0 bind in progress /15 pending. This does not reopen the [closed0bd read-interval measurement](../20261008-dfs-read-intervals/README.md) or relabel historical standard/recovery results. The published [b80 bind ON trial](../20261008-workspace-bind-on-trial/README.md) keeps its identity.

## Product change and fixed exit

Only `src/node/vfs/dfs.rs` production code changed. The readonly, no-write-state path caches one successful immutable FileVersion/LayoutRoot pair keyed by exact FileVersionId. **Each read still calls GetInode and validates the current inode before any cache lookup.** A changed head loads its new version; Meta failures propagate without returning stale content. The bounded one-entry mutex is never held during an RPC. Write-state, authority, permissions, complete chunk checksum, error propagation and persistence paths are unchanged; no third-party source changes.

The four targeted regressions cover avoiding repeated immutable-version loads, changed head returning new bytes/length, GetInode failure despite cache, and changed-version load failure with untouched output and successful fresh retry. The same-head regression failed before implementation (expected1 version load, observed2), then passed. Cache hits avoid that RPC in this test; the runtime pair does not claim a separately observed RPC count.

Before startup, [retention](frozen-exit.json) required **both B and C median whole-task throughput >=1.05x their own baseline AND each independent pooled read p95 <= its own baseline**, with correctness and normal closure. One pair only; no fishing rerun. [Independent Linux audit](paired-result.json):

| Metric | B baseline | B candidate | C baseline | C candidate |
| --- | ---: | ---: | ---: | ---: |
| Median whole-task throughput, MiB/s | 45.581221 | 52.915640 | 45.610345 | 53.022073 |
| Independent read p50, ns | 21660727 | 18670602 | 21743094 | 18657753 |
| Independent read p95, ns — preselected | 23290800 | 19641373 | 23315101 | 19520037 |
| Independent read p99, ns | 23934885 | 20487195 | 24210342 | 20118651 |

B/C throughput improved **16.0909%/16.2501%**, p95 fell **15.6690%/16.2773%**. All four retention conditions passed, so main retains this bounded product change. It does not establish parity with3FS.

## Conditions, correctness and closure

POSIX/FUSE, ctl local-file Meta, A writer then synchronized B/C readers, counter-1m-v1 64MiB with16 distinct4MiB chunks and three synchronous durable copies, C1 per reader,1MiB logical reads. Writer fdatasync and directory barrier precede the read phases. Each phase has one warmup and five formal rounds per reader; each reader contributes320 independent formal read intervals. Timing uses CLOCK_MONOTONIC complete-read-v1, full_read_1MiB_excluding_content_oracle; report nearest-rank p50/p95/p99, with p95 frozen beforehand. Whole-task wall includes open/stat/read/EOF/close; content oracle is outside the timed interval. Both phases use the same unchanged probe, tools and owned configuration.

Serial baseline then candidate in one fixture, same original payload: FileVersion/LayoutRoot IDs, logical chunk/copy mapping and all48 physical chunk dev/inode/size/full SHA identities match before and after the pair. Only B/C reader Node binaries were replaced; Meta and A remained b80. The original R3 proof remains labeled b80; candidate reader ELF captures are independent, not a relabeled homogeneous candidate R3 recovery result. Fresh-open full SHA/length/EOF before/after and every timed round content check passed. Page-cache state was not explicitly mincore-frozen; one-warmup/order/noise limit the inference to this product pair, not a qualified external comparator.

[Closure](runtime-closure.json): six actual child wait0 records match their captured incarnations; supervisors disappeared, with no supervisor-wait claim. Owned FUSE/UDS and drivers closed; original complete mount inventories and12 protected historic processes remain unchanged. Sampled aggregate peak593960960B <896MiB, per-role allocation/free-space gates passed; this is not continuous peak. Logs8210B retain5ERRO/18WARN/0TRACE: A logged four missing-dentry lookups and one unsupported non-user.* xattr, all preserved; no zero-error claim.

## Identity, validation and retained records

[Candidate](candidate-identity.json): source base61c1931b50e42e8a308fdb18c2bcc4f97ddd72e2 + patch753b1bab0f0790d43aae26328f6ad352f9e87769e4e05fa6b310d0a72d89fe1f,158 compiler inputs/mapa3fac3b2ee9fabaf73d0842f2caf862e96c77556c42a8c5fb9bbd82d5360fd52, Node SHAc8bb82afef20f88918521eaeda4dadcc03a99862828835f767fc43bcd14cccfb. Baseline b80/map ea809fa7/Node SHAb3335fb2; unchanged Meta SHA15648a87. [Production reuse mapping](baseline-production-reuse.json) proves b80→61c committed compiler differences were test-only; no runtime conclusion is inherited by that mapping.

[Linux validation](linux-validation.json):148 DFS module tests,15 readonly filter tests (overlapping),13 full-chunk integrity/corruption/EIO/grant tests, fmt/check/affected lib-bins Clippy and one release Node build pass. Two existing peer fallback dead_code warnings remain. The frozen158 source input bytes were verified before startup and remain the measured product; documentation edits do not alter compiler inputs. No new fullPOSIX, crash recovery, RDMA, package or installation claim.

[Archive](archive-index.json) actually restored and SHA-verified991 raw files plus85 nested guest records in Linux. It retains the frozen contract, prototype patch/input map, exact commands, raw operation arrays, physical proof, lifecycle/log records and prestart corrections; ELF stays separately SHA-bound outside Git. [Tool index](tool-index.json) references maintained scripts and case deltas instead of copying them into the source tree. [Failures and limits](failures-and-limits.json) preserve the RED regression and prestart tool corrections; no runtime/performance rerun or environment repair followed.

## Next independent item

Return to G2.14 remote access: select one concrete remaining RPC/data-path cost from the current source, retaining bind ON and remote correctness. Do not retry the rejected READ dispatch or WRITE copy directions, polish local FUSE, expand3FS qualification, or repeat packaging for this isolated change. Ordinary Owner remote final1.2x-throughput AND0.8x-latency, and DFS final3FS parity, remain pending.
