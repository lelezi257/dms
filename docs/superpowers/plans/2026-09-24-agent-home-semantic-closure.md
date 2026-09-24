# Agent home semantic closure implementation plan

> **For agentic workers:** execute each task with a failing Linux test first, then the smallest implementation, then Linux verification. The agreed scope is the Agent workspace preview contract, not full MooseFS POSIX or single-copy VM failover.

**Goal:** Make the five architecture invariants executable and close the confirmed P2P file-identity and unsafe-retry counterexamples without adding per-write center work to the local Home path.

**Architecture:** The center remains authoritative only for first-level root ownership; each Home's native filesystem remains authoritative for entries and bytes within its root. The requester FUSE keeps per-mount inode/handle state. A remote handle operation must reach the Home file descriptor captured at `OPEN`, never re-resolve the old path. Read-only P2P queries may reconnect; mutation and `OPEN(O_TRUNC)` must not be replayed after an uncertain transport result. Existing process-restart contract permits old remote handles to return `ESTALE`, while new opens recover from disk.

**Spec:** `docs/agent-home-preview-design.md`, `PRINCIPLES.md`, and the workspace [architecture feasibility report](../../../../outputs/reports/2026-09-24-agent-home-architecture-feasibility.md).

**Global constraints:** Build, test, and benchmark only on Linux. Preserve the original `source/` worktree. Do not enter S6, merge, or release. Keep P2P and NFS acceptance semantics aligned where their transport permits. Do not claim equal physical durability against default MooseFS.

## Task 1: File identity across rename, unlink, and same-name recreation

- [ ] Add Linux failing regression: B opens A's file, A renames it and recreates its old name; B `fstat`, `ftruncate`, read/write and metadata changes target the old open file. Repeat after unlink/recreation and against NFS.
- [ ] Add Home RPCs for handle-based attribute read/update and length change; bump P2P wire version.
- [ ] Route FUSE `getattr(fh)` and `setattr(fh)` through the opened local/NFS or P2P handle; path-only operations remain path-based.
- [ ] Run targeted Rust tests and real Linux FUSE/P2P and NFS behavior checks; inspect actual bytes and inode identity.

## Task 2: Unknown remote mutation result

- [ ] Add a Linux test that drops the response after Home executes `OPEN(O_TRUNC)` and verify the requester does not replay the truncating open.
- [ ] Restrict automatic reconnect/retry to read-only path queries. For mutation and truncating open, discard the broken connection and return an error whose operation result is treated as unknown; a new query/open can reconnect.
- [ ] Test before-execution failure, after-execution lost reply, and a subsequent fresh open. Include non-truncating read-only `OPEN` reconnect as a positive case.

## Task 3: Five-invariant acceptance matrix

- [ ] Extend `scripts/homefs/accept_three_vm.py` with one owner race, stable remote file identity, close-to-open after concurrent name change, Home process restart (old and new FD), and center restart/recovery checks for both backends.
- [ ] Add controlled root create/delete fault cuts and compare persisted center state to actual Home directories; record any unsupported power-loss claim explicitly.
- [ ] Save Linux raw JSON, commands, binary/script SHA, topology, and cleanup receipt in workspace `evidence/`.

## Task 4: Performance and handoff

- [ ] Run targeted W2 after P2P changes and W1 against same-run Native, thin FUSE, and default MooseFS if normal Home callbacks changed. Keep complete round samples and ACK/config differences.
- [ ] Update design, installation, stage review, and workspace `CURRENT/STATUS/NEXT/LOG` from actual results. Keep preview source branch reviewable; no S6, merge, or release.

**Success:** no open FD mutates a replacement pathname; uncertain mutating RPC is never silently replayed; new opens recover after Home process restart; owner remains unique; close-to-open holds in both backends; modified normal path preserves the already observed local W1 advantage or reports a valid failure. VM-loss availability, automatic migration, and cross-Home atomic operations remain stated architectural boundaries.
