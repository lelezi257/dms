# Agent Home Preview Implementation Plan

> **For agentic workers:** Implement each task against the adjacent [design](../../agent-home-preview-design.md); keep the tests and evidence with the change. The user has selected inline end-to-end execution, so do not pause at intermediate design approvals.

**Goal:** Deliver a Linux-installable 2–4 node Agent workspace filesystem with home-local native files, a management location API, and NFS/P2P selectable remote access.

**Architecture:** A new preview binary and control plane coexist with the legacy DMS Block filesystem. One durable center owns first-level root placement; every node mounts a common FUSE view and serves its local ordinary file tree. Remote operations select kernel NFS or P2P at startup.

**Tech Stack:** Rust 1.95, Linux FUSE (`fuser` already vendored), kernel NFSv4, existing Cargo workspace and package scripts. Reuse dependencies already present in Cargo.lock.

**Spec:** `docs/agent-home-preview-design.md` and `PRINCIPLES.md`.

## Global constraints

- Preserve the existing dirty checkout; develop only in this isolated branch from `main`.
- Build, test, mount, and benchmark only inside the Linux VMs.
- No push, merge, release, S6, or long-stability claim.
- Native FS is a reference, not a pass threshold. Local W1 uses two independent rotated six-round sessions, each p50 ≤ 0.80 × default MooseFS.
- A passing package must be reproducibly built from source and installed for real 2–4 node tests in both backend modes.

## Review focus

- A root reservation crashes between center persistence and physical `mkdir`: restart or retry completes or safely reports pending, never silently routes to missing data.
- A node process restarts with ordinary files already present: root lookup and exact bytes recover without per-file center metadata.
- A home VM disappears: remote operations fail clearly; no other node starts writing the same root as home.
- A closes a write then B opens it: both NFS and P2P return new bytes without whole-machine cache drops or timing sleeps.
- Path traversal and concurrent root creation: cannot escape a node's data root or allocate the same root to two homes.

---

### Task 1: Architecture guidance and branch

**Files:** `AGENTS.md`, `PRINCIPLES.md`, `docs/agent-home-preview-design.md`.

- [x] Create isolated worktree from exact `main` and keep the old checkout untouched.
- [x] State product scope, backends, location API, recovery, ACK, and measurable value target.
- [ ] Reconcile legacy AGENTS wording and README/decision links so the new preview is unmistakably a separate current architecture candidate.

### Task 2: Durable root control plane and location API

**Files:** `server/homefs/src/center.rs`, tests in the same module.

- [ ] Define node registration and root states, durable persistence with atomic replacement and directory sync; recovery must reject incomplete/corrupt state rather than invent a home.
- [ ] Expose root reservation/activation/query/list operations and documented management read API returning home node and endpoint.
- [ ] Test concurrent first create, restart, pending create, corruption, and center-down behavior.

### Task 3: Common home file and P2P transport

**Files:** `server/homefs/src/p2p_rpc.rs` and focused tests.

- [ ] Forward bounded file operations to the home file tree; preserve native file handle and error semantics through close/fsync.
- [ ] Prevent path escape, reject unsupported file types and overlarge messages, authenticate trusted peer connections.
- [ ] Test operations, false success, reconnect, restart, traversal and close-to-open bytes.

### Task 4: Unified FUSE route and NFS backend

**Files:** `server/homefs/src/home_fuse.rs`, tests in that module.

- [ ] Maintain local kernel inode/handle state; route first-level roots to local ordinary files, pre-mounted NFS, or P2P according to home and startup backend.
- [ ] Handle common create/open/read/write/flush/fsync/release/readdir/mkdir/rename/unlink/rmdir/truncate; EXDEV across homes and EHOSTUNREACH when backend unavailable.
- [ ] Use conservative cache flags and NFS mount options until close-to-open tests pass; no timestamp-only coherence claim.

### Task 5: Daemon/configuration and root lifecycle

**Files:** `server/homefs/Cargo.toml`, `server/homefs/src/main.rs`, root `Cargo.toml`, `Cargo.lock`.

- [ ] Wire center/node/manage commands and startup config with explicit data, peer, mount, endpoint and backend paths; do not silently select another backend.
- [ ] Verify node registration, root create/delete, center restart and home process restart with real Linux FUSE.
- [ ] Keep legacy dms-node/meta behavior unchanged.

### Task 6: Linux installation package

**Files:** `scripts/build.sh`, `scripts/package.sh`, new `scripts/homefs/*`, `docs/agent-home-preview-installation.md`.

- [ ] Build and package the new binary with configuration and startup scripts; do not rewrite the v0.1.0 limitations as if it gained durability.
- [ ] Build from clean Linux source, inspect archive and hashes, install the archive, start center and 2–4 nodes with both backends.

### Task 7: Product E2E, performance, and verdict

**Files:** new `scripts/homefs/accept.py`, `evidence/` outside the source tree or a documented self-contained evidence directory, handoff docs.

- [ ] Run real file/dir, close-to-open, root mapping, center/home restart and outage cases on Linux in both backend modes. Record actual bytes and failure returns.
- [ ] Run W1 and distinct remote workload with fixed hashes and two independent six-round same-host comparisons to stock MooseFS, plus thin FUSE and Native reference.
- [ ] Triage any failed gate by phase timing and correct the implementation; deliver an honest PASS/FAIL with package path, source build steps, remaining limits and raw receipts.
