# G2.21 saved-input impact review — 2026-10-09

**Decision: reuse the historical c8bb B/C function/measurement/closure evidence within its original scope; no newly affected DFS behavior requires repeating that pair. Current d14 B/C concurrent performance remains unmeasured; formal 3FS parity remains pending.** This is a read-only review of saved compiler inputs and Git source, not a new runtime PASS. G1 historical8/8 stays closed. No VM, product build/test, dependency or runtime operation was performed for this review.

[Compact reuse map](reuse-map.json) records every differing input, its hashes, exact source anchors and evidence references. Full compiler maps and scripts are not copied into this directory.

## Exact identities

| Identity | Historical cache pair | Published current candidate |
| --- | --- | --- |
| Source | `61c1931b50e42e8a308fdb18c2bcc4f97ddd72e2` + patch `753b1bab0f0790d43aae26328f6ad352f9e87769e4e05fa6b310d0a72d89fe1f` | `d47eec28360ba60e92d95c9f0775c8ecce53ef2e` |
| Input map | `a3fac3b2ee9fabaf73d0842f2caf862e96c77556c42a8c5fb9bbd82d5360fd52` /158 inputs | `ec7e2de5ea5e5b83883725fe4175b819af75cd4713c25e0479bb275ff5bab300` /160 inputs |
| Node SHA256 | `c8bb82afef20f88918521eaeda4dadcc03a99862828835f767fc43bcd14cccfb` | `d14e3182260e0f7ab51c82c4f18b9e2deab6446cd2980a6787fa43c22e9361b4` |
| Meta SHA256 | `15648a8753bfe240fce15c24b673057dea6b97e04f82abee6eb6c5a348d086e2` | `650bd9714da0293c61b6aff80ce86aede1152a233edb1506ed2d97d5b53f3e58` |

Historical runtime replaced **only B/C reader Nodes**; A and Meta remained `b80dab66e9d819cad821c9b30edbf97c795c0c3e`. Do not relabel its writer/R3 evidence as homogeneous c8bb or d47. Current map metadata records base_main `461407c4439f247f1e4abdd5fcd99cff2ec19f50`; published candidate identity is d47. The four changed production files were checked against Git at both respective source revisions and match their saved map hashes.

External source records remain under the original host delivery root:

- `evidence/afs-delivery/dfs-read-version-cache-20261008-r1/compiler-inputs.json`, file SHA256 `0ab1be7324825a3b7b06c59cdd0d23c1edb886223a72025581c1e8a251bbe76a`.
- `evidence/afs-delivery/workspace-bind-on-trial-d47-20261009-r1/compiler-inputs.json`, file SHA256 `75ca5790a8d73f3e1a1a97fe0b4cdafa10c6858c9f2f841028f65858542081fd`.

These file hashes bind the JSON records; the input-map identities above are distinct recorded identifiers. Exact absolute external paths and candidate/identity record hashes are in the compact map.

## Differences and DFS impact

**Facts:**153 inputs are byte-identical;5 changed,2 added,0 removed. Four changed files contain production deltas; the other three are OwnerFs tests. Source line anchors below refer to **d47**, rather than a later working tree.

| Changed production input | Delta and DFS boundary |
| --- | --- |
| `common/protocol/build.rs:26` | Only `OwnerReadReply.data` uses Bytes. DFS protobuf source and wire schema are unchanged. |
| `src/node.rs:44,50,87,1636` | Server frame change applies only when `ownerfs && !dfs`; DFS-only/combined retain original configuration. OwnerFiles factory selects its own channel profile. Startup/shutdown ordering is unchanged. |
| `src/node/rpc/data.rs:2326,3137,3201,3560` | OwnerReadReply construction and Owner RDMA adapt to Bytes. DFS read/replica handlers, authorization, checksum and errors are unchanged. |
| `src/node/rpc/peer.rs:817,861,874,885,940,1059` | Shared pool key gains an Owner-profile boolean. Ordinary DFS channel and long-wait use false and retain their endpoint settings; DFS read_ranges still selects the ordinary channel. Epoch high-water fencing and stale-profile eviction remain shared. |

The other differences are changed `tests/ownerfs_peer_contract.rs`, added `common/protocol/tests/owner_read_payload.rs`, and added `common/protocol/tests/owner_write_payload.rs`. These are test inputs, not DFS product changes.

Byte-identical scopes include `src/node/vfs/dfs.rs`, `src/dfs.rs`,10 Meta inputs,2 storage inputs, `src/node/fuse.rs`,14 public transport inputs,5 protobuf source inputs, `Cargo.toml` and `Cargo.lock`. The retained cache loader (`src/node/vfs/dfs.rs:1178`) and each-read GetInode/validated-head call before cache lookup (`:1682`) are identical. R3, full-chunk checksum and close/drain behavior have no newly identified production delta.

**Inference:** these deltas provide no newly affected-scope reason to repeat the historical DFS B/C pair. This does not prove identical new-ELF timing or constitute a current B/C runtime result.

## Existing evidence and decision

- [Original cache pair](../20261008-dfs-read-version-cache/README.md): c8bb B/C synchronized readers,64MiB/C1/1MiB, one warmup + five formal rounds and320 independent intervals per reader;48 physical identities and original FileVersion/LayoutRoot preserved; normal closure. Historical throughput improvement16.0909%/16.2501% and p95 reduction15.6690%/16.2773% retain that original identity and comparison conditions.
- [Original production reuse mapping](../20261008-dfs-read-version-cache/baseline-production-reuse.json): b80→61c differences were test-only. This preserves the original writer/Meta identity and does not create a current runtime conclusion.
- [Owner channel regression evidence](../20261008-owner-remote-read-frames/README.md), [Linux receipts](../20261008-owner-remote-read-frames/linux-validation.json): profile isolation, cross-profile epoch fencing and existing eviction fencing. Relevant d47 tests are `peer_pool_owner_profile_is_separate_but_shares_epoch_fencing` (`src/node/rpc/peer.rs:5234`) and `peer_pool_never_regresses_epoch_even_after_channel_eviction` (`:5269`).
- [Server regression evidence](../20261008-owner-remote-write-server-frames/README.md), [Linux receipts](../20261008-owner-remote-write-server-frames/linux-validation.json): real HTTP/2 SETTINGS test for Owner-only, DFS-only, combined and disabled profiles; d47 test at `src/node.rs:411`. The earlier packet retains its own premeasurement status and failures; this review does not rewrite it.
- [Separate current G2.22 evidence](../20261009-dfs-local-read-current/README.md): d47 homogeneous R3 A sole-reader function/independent measurement. It does not measure B/C concurrency.

**Decision:** retain the G2.21 closed historical function, measurement, replica preservation, normal closure and cache-retention decision. Do not assign c8bb performance values to d14. Current d14 B/C concurrent numbers are **NOT_MEASURED**; qualified same-condition3FS performance is **PENDING**.

Next action: do not launch a duplicate B/C pair solely because the ELF differs. Continue the existing independent-task order. A bounded current run becomes necessary if actual DFS behavior, comparison conditions or judgement change, or an explicit delivery exit requires current-ELF concurrent numbers. No new matrix, write/recovery rerun or 3FS qualification was introduced here.
