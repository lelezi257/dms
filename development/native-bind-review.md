# OwnerFs native bind review record

Issue: [#42](https://github.com/lelezi257/dms/issues/42).
Scope: isolated `feat/ownerfs-native-bind` branch. This records an intermediate
implementation review; full feature and independent PR review remain pending.

## Native flock retirement and kernel error interface

The retirement bug retained terminal/session history after global invalidation,
so the registry could not reclaim tables. Permanent invalidation now refuses
all replay and clears obsolete history after kernel cleanup. Failed unlocks
retain their descriptor pins and non-idle state. Session cleanup after global
invalidation retries pins without creating new session history. Waiting threads
wake into the permanent refusal and do not recreate outcomes.

The shared `LockError` enum gains `Kernel(i32)`. Only the native Home flock
coordinator emits it; the userspace lock table and DFS state transitions remain
unchanged. Acquire and unlock errors preserve errno rather than pretending a
mutex was poisoned. Existing stable domain/FUSE mappings cover ordinary kernel
flock errors; ENOSYS uses the existing Node VFS unsupported code. Neither the
wire protocol nor error catalog changes. Rust consumers with exhaustive matches
must account for the added enum variant; this is an interface change to review.

Reviewed evidence: five behavior regressions, full root-project all-feature test
command, strict workspace/all-target/all-feature Clippy, fmt and three feature
compilation checks. Current independent VM foundation17 PASS uses binary SHA256
`24edd642eb2d8dd46684a54613c8bd57e15049bd840c6fa9249943498f15fae3`;
parent namespace and temporary-data cleanup are independently inspected.
[Evidence](native-bind-evidence.md) preserves the prior failures, ignored cases,
source/binary/archive identities and exact scope. Feature-off/DFS-only builds
retain two descriptor-helper dead-code warnings; strict lint under those
individual variants has not been claimed.

## Open review gates

- Exact native lock Home authority revalidation and precise terminal outcomes
  when a normal owner release fails; source regressions do not replace network
  peer/session fault tests.
- POSIX process-owner arbitration and the pending old-FUSE/native same-process
  ownership boundary. Do not silently replace classic POSIX ownership with OFD.
- Node current-authority control feed, bounded hint/inventory worker, effective
  namespace/source/policy readiness and actual managed Agent start/stop/fencing.
- Full deletion/reclaim/switch and boot/namespace/daemon recovery. Detached or
  revoked alone is insufficient to delete or reuse backing.
- Applicable file/permission/mmap and actual P2P gates, followed by paired
  native-ext4 performance and the final no-merge PR for independent review.

No full-feature, production admission, performance or merge approval follows
from this intermediate review record.
