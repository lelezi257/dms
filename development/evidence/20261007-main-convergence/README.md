# Main convergence inventory

2026-10-07. **Decision:** main is the sole daily development/delivery entry under the user's standing commit/push authorization. No feature PR/MR or per-feature human approval gate; normal affected checks continue, overall project review follows the overall goal. No history rewrite, force push, worktree cleanup or historical acceptance rewrite.

## Included work

Initial main/origin main: f09185e74229e267b0a5db3d8e32ec267e3b8783. [Before inventory](inventory-before.json) records refs, uncommitted state, worktrees and the eight existing fix-branch commits. Their complete valid change set was fast-forwarded together, including the additional completed uncommitted Node slice committed as [73842cd](https://github.com/lelezi257/dms/commit/73842cdf2a14ed09112bb16dfc35b07d3f5761c7). [Actual fast-forward output](fast-forward.log). No selected-feature-only merge, cherry-pick or Rust conflict resolution was needed.

| Included commits | Result and original evidence |
| --- | --- |
| 2a55b02 | Recovery repair and initial failure/version ledger: [initial evidence](../20261007-native-orderly-recovery/README.md) |
| 033e7c2 | [Scoped Owner standard reuse audit](../20261007-owner-standard-reuse/README.md); no extra suite run |
| 5596f3b, 08458e0 | [Verified VM archive and authorized removal](../20261007-vm-archive/README.md); source/cache/baselines protected |
| 3cc10a2, 6e57374 | [Final source checks](../20261007-native-orderly-recovery-source/README.md), [build](../20261007-native-orderly-recovery-build/README.md), [4KiB orderly-recovery runtime](../20261007-native-orderly-recovery-runtime/README.md) and checkpoint alignment |
| 165d13f | [Workspace bind naming/ownership migration and affected tests](../20261007-ownerfs-bind-remediation/README.md) |
| df56e48 | [One real OwnerFs FUSE core covering-bind test](../20261007-ownerfs-bind-core-fuse/README.md) |
| 73842cd | [Node startup failure cleanup](../20261007-ownerfs-bind-node-startup/README.md): src/node.rs, two targeted regressions, maintained Linux runtime tool, plan, docs and all raw/index evidence |

The uncommitted Node slice was complete before inclusion: Linux fmt, affected test compilation, Owner-only/DFS-only checks, strict Clippy and release build; five Node tests; one real rejected startup with28 checks, Node actual wait1 and Meta wait0. Its first driver-oracle FAIL remains in the packet. No partial code was presented as complete.

## Other branches and worktrees

| Refs | Judgment and disposition |
| --- | --- |
| arch/unified-node-runtime, feat/native-filesystem, mem-kv; corresponding origin refs | Already main ancestors. mem-kv preserves the prior product's Git history; do not restore deleted old product code |
| local perf/native-fs-vs-moosefs | Already integrated as an ancestor |
| origin perf/native-fs-vs-moosefs; local/origin perf/native-fs-small-hot-path, perf/native-fs-namespace-mutation, perf/native-fs-write-through, perf/native-fs-peer-first-read | Old server/protocol/Arena/block P1–P4 architecture, explicitly superseded by AFS reset; no direct current AFS completed omission |
| feat/local-owner-durable-preview | Old range-owner/KV/block product; S5 execution complete but performance FAIL, S6 unstarted. Preserve26 modified tracked files and one untracked plan; do not import |
| feat/agent-home-fs-preview, corresponding origin ref | Superseded separate server/homefs/P2P/NFS preview, different permission contract and old vendor changes; retain history |
| origin feat/ownerfs-native-bind, fixed80b0bca | Issue42/PR43 investigation/provenance already in main. Current mechanism was selectively reimplemented; old production journal/manager lifecycle was unfinished. Do not blindly merge abandoned code. [Original fixed issue/PR/payload provenance](../20261007-container-perf/README.md) and [current implementation boundary](../../native-workspace-slice.md) remain distinct |

This judgment uses code and product decisions, not just ancestry: [AFS reset3cad952](https://github.com/lelezi257/dms/commit/3cad952) explicitly rejects the old KV/block state machines and preserves unfinished worktrees; [history preservation3abc7f7](https://github.com/lelezi257/dms/commit/3abc7f7) retains the previous main as mem-kv. No other valid completed current-goal code omission was found.

All15 worktree records remain. The agent-home preview and foundation worktrees are clean. Eight old dirty worktrees (local-owner, r45, r7, r7fix, r7fix2, r7fix3, r7fix4, sv4) remain untouched. Their binary patches total1,257,452 bytes and were additionally saved **outside the source repository**, alongside the37,709-byte untracked private-local plan. Every patch was checked against its recorded HEAD using a separate temporary Git index; copied untracked content was hashed. Exact original paths, HEADs, diff hashes, status and recovery locations are in [inventory](inventory-before.json). Original worktree files remain the primary copy. The four missing/prunable metadata entries were not removed. The r7fix4/sv4 duplicate patch identity is recorded without deleting either workspace.

Recovery archive root: `/Users/lzc/workspace/code/agentruntime/rust-distributed-memory-store/evidence/afs-delivery/main-convergence-20261007-r1/`. Apply a retained tracked.patch to its exact recorded HEAD in a separate checkout; restore the indexed untracked path from its recovery copy. These old drafts have no current AFS completion proof and are explicitly excluded. Full old source snapshots, ELFs, TLS keys and archive payloads were not added to Git. [Inventory tooling failures](inventory-tool-failures.json) preserve the missing-metadata/default-buffer errors and narrow corrections; no product test or environment was changed.

## Candidate and acceptance identities

[Read-only live identities](running-identities.json) record actual executable SHA, PID/starttick and AFS mounts in all six existing Linux VMs. ctl/A historical Meta/FDB processes, build-VM system FDB and A's two historical FUSE mounts were preserved. B/C/micro have no matching services or mounts. The owned Node startup slice has actually finished; main convergence does not deploy or replace these processes.

| Evidence class | Version and scope |
| --- | --- |
| Historical complete | G1/g1.5 remains8/8; does not become full POSIX or performance certification |
| Historical tested |3cc/e15c map orderly recovery;165d/fabab map migration checks;df56/6dd63 map real FUSE core;6d package and recorded standards/remote/DFS baselines retain exact links and limits |
| Current-source tested |73842cd,157-map196a4177141fb837d3e5b155438e894ab9d632bfceb97a9ea3ab20bf2ce57c6d; six Linux gates, five Node tests and rejected-startup process scope only; Meta ELF f4d423fc…, Node ELF de1d1689… in linked frozen proof |
| Current-source pending | Accepted workspace lifecycle/full ON, general drain/host visibility, applicable changed-candidate runtime regressions, new installed trial/performance qualification. A Git merge does not automatically inherit all runtime PASS |

Unchanged code/environment/criteria evidence is reused with identity; no standard, ordinary performance or prior core test was rerun for the merge. G2 remains10 bounded complete/2 ordinary performance failures/2 bind in progress/13 pending. Append/classic-lock/watch failures, comparison qualification and original fuser API migration blocker remain. Next remains G2.12 accepted Node/workspace lifecycle, then G2.13 performance; DFS one-writer/many-reader, broad reliability/etcd/Redis keep their agreed order.

## Module and publication checks

The old src/node/native_workspace/ subdirectory is absent. Its mount.rs moved to the **single src/node/vfs/ownerfs/bind_mount.rs**. WorkspaceBindMount owns mount identity verification and normal detach; it has no native_workspace/container_mount/runc naming. Remaining src/node/native_workspace.rs is the separate runc adapter: container startup/exec/stop, private rootfs copy, secondary-reference drainage, legacy adapter configuration compatibility. Node owns lifecycle wiring. Default is still OFF.

The actual bounded core test mounted a physical Home ext4 directory onto the corresponding first-level workspace below a real OwnerFs FUSE root, in the controller/test private namespace. It did not bind FUSE to itself. This is not a standalone host-visible Node feature; naming migration/build success does not close G2.12.

[Convergence checks](convergence-checks.json) bind source map, complete included ancestry, core location, adapter retention, protected vendor/handoff and portable links/manifest identities. [Manifest](SHA256SUMS) covers this inventory packet. This document is included in the normal main push. Post-push GitHub ref/tree verification is recorded in the source-tree-external recovery root as remote-verification.json; final reporting names the actual remote main commit only after that verification. The outer research workspace is not a Git repository; its current/status/log/next files are updated locally, not claimed as GitHub source files.

The preliminary metadata check is retained as convergence-checks.pre-links.json: its overescaped link regex selected zero links and was not accepted as link coverage. Final convergence-checks.json uses the corrected selector and checks160 local links with no missing targets, plus617 payload hashes across nine included evidence packets. Only publication checks repeated; no product test rerun.
