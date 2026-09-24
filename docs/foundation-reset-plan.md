# Foundation branch reset plan

Base: `origin/main` at `7e210b4`. This branch is an isolated new start for two
Agent storage workloads. Its first deliverable is a truthful constitution and a
small reusable engineering base, not a running filesystem.

1. Remove the old KV/object service, SDK, protocol, SHM, benchmarks, release
   tooling, historical evidence, and design documents from this branch. Their
   Git history remains available on `main`; do not change other worktrees.
2. Retain only genuinely cross-cutting logging, metrics, tracing, and transport
   mechanisms. Remove old RPC names, object error catalogs, and wire-specific
   conversions from retained modules. Do not carry the old FUSE patch forward
   without its original lock-interrupt use case.
3. Write one concise constitution in `PRINCIPLES.md`, collaboration rules in
   `AGENTS.md`, workload contracts in `docs/workloads.md`, and a current-state
   handoff in `docs/status.md`. State accepted direction separately from
   implemented capability. Detailed architecture is the next stage.
4. Replace CI with checks for the retained Rust workspace. Run verification on
   Linux only: formatting, check, clippy, and targeted tests. Audit remaining
   tracked files and references for accidental old-product claims.
5. Commit the clean foundation on the new branch. Do not merge or release.

The source branch and its uncommitted changes are outside this worktree and
must remain untouched.
