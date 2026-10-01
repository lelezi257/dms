# Delivery implementation rules

The release contract is [acceptance.md](../docs/acceptance.md). Architecture documents describe the accepted design; fill missing implementation design without weakening acceptance. A conflict that changes a public contract requires an explicit decision, not a silent implementation shortcut.

## Architecture boundaries

Keep existing core directories, modules and public interfaces where possible. OwnerFs and DFS are submodules of afs-node with independent mounts and state. Share the FUSE module and transport resources. File I/O enters Node, while management REST may enter Meta. File bytes never transit Meta.

Keep rpc/control.rs, data.rs, meta.rs and peer.rs responsibilities. Meta owns authority, Peer control coordinates nodes, Peer data transports bytes, peer.rs owns shared connection resources. gRPC and RDMA carry the same identity, authorization, completion and error contracts. R=1 and R=N branch below ChunkStore, not in file layout.

Only first-stage scope is required: do not pre-build Meta HA, DFS SDK, VerifiedCache/Seed or Spill. Preserve useful existing implementations. Add fields or focused interfaces as needed; avoid unrelated refactoring and speculative abstractions.

## Whole-system convergence

Follow the four rounds in [plan.md](plan.md): healthy whole-system use, major faults and recovery, performance/resources followed by persistent backends, then formal delivery acceptance. Each round visits deployment, management, OwnerFs, DFS, replication, transport and observability. Existing task IDs are work labels and dependencies; they do not require completing every edge of one module before entering another.

Use [issues.md](issues.md) as the single active issue list. Record the affected scenario, consequence/severity, whether it blocks the current round, temporary handling, planned round and acceptance cases. At round end review whole-system evidence and reorder remaining issues. Do not select the next task merely because it was discovered most recently.

Immediately repair mainline blockers, erroneous success, silent corruption, permission failures, broken commit ordering and resource exhaustion under normal use. Defer only issues whose consequences permit normal correct use and current integration; calling an issue an edge case does not justify deferral. A workaround must preserve the accepted contract and have explicit limits.

Use small fixtures early to expose major functional and performance problems. Keep costly file sizes, full matrices and long runs in their scheduled acceptance rounds. Memory Meta is the primary functional lane; its restart does not promise namespace persistence. Validate durable Meta restart with etcd/Redis in round 3.

## Slice design

Before each implementation slice, identify acceptance cases, current code, missing behavior, design and a short failing test or reproducer. Define user-visible behavior before selecting abstractions. Include RPC count/roles, durability boundary, identity, error propagation and bounded resource ownership where relevant.

Choose validation scope using [validation.md](validation.md): small edits use the original failure, related module regressions and necessary compilation; related batches add affected core integrations; stage completion or batch integration adds the full Linux source gate. Expand earlier for public interfaces, data formats and cross-module contracts. Reuse valid frozen evidence with exact identities; deployment-script-only work does not repeat unchanged Rust gates. Keep local regression, stage gate and formal acceptance results distinct.

Record significant directory/module/interface/data-format changes in the execution change log: problem, alternative, decision, affected callers, compatibility and evidence. Consolidate these changes for the final human review. Update final-design architecture pages when design changes; execution progress belongs outside product architecture pages.

## Autonomy and stopping conditions

Complete routine design details, fixes and optimizations without intermediate approval. Record reviewable decisions for the final delivery. Do not change acceptance thresholds, required coverage, consistency or architecture direction without explicit authorization. Report genuine external blockers with evidence and continue independent authorized work.

The delivery is complete only when required acceptance gates pass, an installable release package is reproducible, and final changes/evidence are reviewed. A skeleton, successful compilation or a few demonstrations are not delivery completion.

See [validation.md](validation.md) for staged validation.
