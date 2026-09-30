# Read-only shutdown review

Reviewer: native code-reviewer, 2026-10-01. Reviewed runtime, Node binary/assembly, shared FUSE and vendored channel/session/unmount lifecycle. No edits, Cargo, Git or service operations.

No concrete correctness/security/lifetime blocker found. Per-session wakeup, stop-before-callback-drain, final lock sweep and process guard preserve the targeted boundary. Root owns all Linux/runtime evidence.

Non-blocking follow-ups: vendored join currently unwraps underlying errors and MountedFuse maps unexpected panic to generic I/O error; lazy-detach helper does not report when both unmount attempts fail. Do not interpret process disappearance as clean status. Poll-to-read/physical-I/O blocking remains possible and is bounded by failed process termination rather than cooperative cancellation.
