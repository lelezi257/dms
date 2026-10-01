# OwnerFs native bind review record

Issue: [#42](https://github.com/lelezi257/dms/issues/42).
Scope: isolated `feat/ownerfs-native-bind` branch. This records an intermediate
implementation review; full feature and independent PR review remain pending.

## Focused architecture/performance delivery checkpoint (2026-10-01)

The user has assigned generic reliability to another AI. This branch now focuses
on bind architecture, actual ext4/native/MooseFS performance data, and changes
necessary to validate or implement that feature. Do not resume the separately
parked drain/reaper/retry WIP or expand shared-module repairs. A no-merge PR is
the delivery vehicle after concrete conclusions/data, not evidence of acceptance.

E13 closes retained-object/close-to-open A2 on actual authenticated VM peers;
E14 proves the selected nonblocking flock combinations; E16 proves two reopen
cases with a native-eligible Home and ordinary remote constructor. E15 is a
qualified semantic failure: a single remote append can be interleaved with native
writes and report the wrong file position. The unchanged standard FUSE/P2P
boundary cannot provide the required syscall-wide arbitration/append endpoint.
Neither generic retry nor a userspace per-request mutex closes that conflict.
Classic POSIX process ownership also remains unresolved. No new semantic
exemption has been approved. The architecture stage has not passed and native
performance has not yet been measured.

Subsequent E17 provides optimized actual P1 timings: absolute-path task ratios
19.276/4.161(c1/c8) decisively miss the native goal; native-cwd relative
0.992/1.036 is close to ext4 without qualifying every strict gate. The full
timing run retains its Actor-timeout FAIL; forensic timing checks and the short
clean supplemental run are distinguished. Stock goal1 MFS data is visibility
diagnostic only, with excluded pre-configuration samples; B001 durability and
P2/P3/P4 remain open. The report supersedes the earlier “not measured” checkpoint
for P1 only. Generic reliability is still outside current work.

Review overlap explicitly: older bind groundwork touches Root grant/command
APIs, Meta owner_roots/store/RPC, Node rpc/meta, shared FUSE adapter and LockError.
These are included historical foundations, not permission to keep expanding
general reliability. Production Node still constructs ordinary OwnerFs; the
native network driver is Linux-test-build-only. Final independent review must
distinguish experimental mechanisms, required production glue, and work owned by
the other AI. See the current validation plan and evidence for scope/provenance.

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

## Xattr call lifetime and native directory authority

Four actual original failures cover both path-returning xattr helpers, revoked
empty-directory reads and retired directory capabilities borrowing recovery.
Private xattr helper tuples now carry RootUse to all eight syscall callers.
Native-eligible Home directory wrappers store the complete opening grant;
local readdir admits that captured authority and retains its guard through
enumeration/response, with an additional peer-handler guard through final row
conversion. Directory sync/close cleanup is preserved. Ordinary directory policy,
wire format and DFS are unchanged; ordinary local xattr bookkeeping also retains
its already-admitted operation guard through completion.

Four focused regressions and independent ext4 VM replay,414 library tests and
strict Clippy/fmt pass. These do not establish complete public-call/network
stress or path-object race coverage, whole-handle/dirty-work drainage, actual
Node/Agent fencing or normal detach/ACK. Native POSIX/mmap and performance remain
open. Exact identities and all original failures are in the evidence record.

The prior opendir inspection is corrected: its successful local path already
retained a second RootUse through physical open/publication. No publication
counter bug was reproduced. The actual fixes are xattr caller lifetime and
directory captured-authority/readdir lifetime. Final independent review remains
pending.

## Home native lock request lifetime and drain interface

The two original blocked-call counter failures are reproduced with actual kernel
locks. Local and Home-side peer get/set lock calls retain a RootUse for the full
native-authority operation lifetime. Ordinary keys and remote dispatch retain
their existing paths. OwnerFs adds native-only refusal and lock-drain methods
with a current RootRefusal, full grant-key matching, attempted cleanup of every
matching table, error/pin retention and final table/route/operation rescan. Failed
unlock remains failed; terminal fenced peer routes no longer wait for an ACK
that cannot be admitted after refusal. Other roots and recovered authority
retain their locks. No new RPC or native Node bootstrap is enabled.

Six focused cases and their independent ext4 VM replay,410 passing library
tests, strict Clippy/fmt, root-project all-feature tests, three feature builds
and fresh17-case actual VM foundation support this slice; exact source/binary/archive and
original failures are in the evidence record. True covers current admitted
operations and matching lock tables/routes only. It does not drain open handles,
dirty work, native Agent processes/exports or declare shared scope history fully
reclaimed. Applicable native POSIX ownership, network P2P, Node consumer/cursor
durability, actual fencing and normal detach/ACK remain required. Final
independent review is pending.
The subsequent checkpoint repairs the xattr-helper gap and corrects the initial
local-opendir publication assessment; the actual further directory gaps were
captured authority and readdir lifetime. A retained-guard count is still not a
whole-filesystem syscall/reference drain proof. Full audit and actual handle
cleanup remain required; green lock/lifetime slices do not waive that gate.

## Command-scoped Home refusal interface

RootManager adds `begin_command_refusal` and `refused_operations_drained` with
an opaque process-local `RootRefusal`. A Refusing phase retains the exact command
and full captured grant; it rejects new local/peer admission under the same lock
as operation counting. Delayed/foreign targets fail before mutation. Duplicate
identity is exact, including revision; a changed command cannot reuse a previous
barrier. Drain checks compare the cached Arc identity as well as command/grant,
and fail on control loss or unrelated invalidation. The old RootId-only
invalidation API is not repurposed as a command processor. DFS is unchanged.

Five missing-feature behavior REDs,404 passing library tests, strict Clippy/fmt,
root-project all-feature tests, three feature builds and five independent VM
ext4 passes support this interface slice. Fresh17-case actual VM foundation
replay also passes with independently checked mount/data cleanup. Existing
ignores and separate vendored-fuser higher-ABI failures remain outside these
passing scopes. The original
API compilation failures and unavailable stubs are retained separately. This
does not prove open-handle/lock/process drainage, normal unmount, persistent
command/cursor state or ACK honesty. Node consumer wiring, actual Agent fencing
and full semantic/performance qualification remain open. See the evidence record
for exact binary, snapshot and archive identities. Final independent review is
still required.

## Open review gates

- The additive command poll and exact ACK slice passes14 source controls and
  their VM replay, with4 retained lock regressions and39 Meta contracts separately
  passing on VM.399 library tests, strict Clippy/fmt, feature compilation and45
  native contracts pass; WSL project-wide expiry remains failed and later suites
  unqualified. The public read selector gains RootCommand, the pinned read view
  gains global revision/prefix reads, RootMeta gains a default-unsupported poll,
  and OwnerRoots adds a unary RPC/message without changing legacy wire fields.
  Legacy watch semantics and ordinary filesystem/DFS policy remain unchanged.
  ACK replay compares exact payloads; existing tonic timeout/unknown-outcome
  semantics are preserved. Scope, all failures and raw identities are in the
  evidence record. Final/independent review remains pending.
- A valid ACK identity cannot prove truthful drainage. The Node producer must
  still wire the exact-generation refusal, persist pending work before cursor
  advance, fence actual managed users/references, normally detach and only then
  issue success. No runtime native admission follows from the new poll.
- The subsequent native lock authority/release slice passes its four original
  behavior failures and all385 all-feature library tests (two existing ignored).
  Native keys distinguish recovered Home authority even at the same epoch;
  old open capabilities cannot borrow a recovered grant, post-wait validation
  retires the old table, and a failed normal unlock preserves its granted
  outcome and pin for retry. Ordinary keys remain epoch-based. Exact source
  snapshot and RED/GREEN logs are in the evidence record. Strict lint, root-project
  checks,17 VM foundation cases and all4 new regressions on VM ext4 PASS. Exact
  binaries/archive hashes and independent namespace/data cleanup checks are
  indexed in the evidence record. These fixtures do not replace network
  peer/session fault tests or full review.
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

## E18: core container Agent mechanism and P1 performance

Actual OCI containers resolve the original verified ready export inside the Node
mount namespace, then bind only that workspace at `/ownerfs/agent1`. Native and
ext4 controls have the same physical source inode; final native mountinfo has
ext4/no FUSE. C0/C1 checks cover distinct namespaces, empty capabilities,
NoNewPrivs, readonly parent containing only agent1, known host/sibling/backing
data inaccessible via direct/parent/symlink/proc paths, and own read/write.
This proves bounded host-data isolation for the tested spec; it does not hide
mount-root metadata or qualify arbitrary UID mappings/hostile code.

Run `network-probe-20261001T125205-fc4c5860` completed72 P1 tasks, normal
container/Node/Meta teardown and independent checks of frozen artifacts/specs/
actual runtime replies. Absolute native/ext4 paired medians1.050/1.019 and
relative1.003/0.995 remove the host's order-of-magnitude ancestor penalty;
16 of24 phase/shape medians exceed1 and uncertainty crosses1, so strict native
performance is not passed. Stock MooseFS remains visibility-only/B001
unqualified. All samples and precise identities are in the performance report.
The zero-timing observer failure is retained separately, not promoted to PASS.
E19 now adds30 local sequential/random IO shapes/360 tasks, with cache conditions
checked before timing, bytes verified and independent raw-artifact verification.
Median paired ratios0.834–1.135,17/30 above1, do not pass strict native performance.
Original normal detach returns Detached while the independent native container
still reads/writes the same source: final container mounts require tracking and
stop/unmount before reuse. Both containers then stop/delete and Node/Meta exit0.
The initial cache-premise FAIL is preserved; four same-file cases are finite
attribution, not replacement. Full three-way/remote timing and production daemon
publication/Agent READY/fencing remain unqualified.

Read-only origin/main audit at `16e855468f2ef36405044b4136e7febe646a5dd4`
finds overlap with the pre-existing bind foundation in meta.proto, Meta RPC/store,
Node constructor/meta RPC, OwnerFs/root and architecture/operations/status docs.
The tested feature candidate remains based on7824577, not the later main
runtime. No rebase/merge or generic reliability edits are performed to hide this
integration boundary. A draft PR must disclose these paths and source identity;
the other AI's review must resolve integration and rerun any affected proof.

## Bounded container evidence review and draft delivery

A read-only reviewer inspected the new OCI/IO/observer code, cache/timing/content
checks, source namespace/spec, full360 paired results and lifecycle wording.
No blocking issue was found for draft evidence delivery. Optional provenance
hardening now directly links the rootfs benchmark/probe/IO SHA to frozen input
SHA; original archived copies already matched, so no timing sample was changed.
Final observer rechecks the completed container runs and rejects archive/read/
timing/source/isolation/cache/shared-file mutations. Six portable raw packages
include the E15 semanticFAIL, E17 runnerFAIL plus supplement and E18/E19 original
artifacts/frozen source, retaining original verdicts.

Declined-to-judge scopes are explicit: production integration/merge readiness,
generic reliability, inherited foundation, complete POSIX owner/append repairs,
hostile-container security, host physical cold cache, power-loss durability and
remote/MooseFS full performance. They remain open requirements, not exclusions
or approval. Another AI's integration review is still required; this branch
and full architecture/performance goal are not ready to merge/complete.
