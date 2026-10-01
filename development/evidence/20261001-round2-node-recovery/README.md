# Round 2 Node interruption and recovery

**Scoped fault integration PASS; ROUND2 remains incomplete.** Formal acceptance
is still **69 NOT_RUN / ENV PREPARING**. This batch tests actual current Node
process interruptions, not memory Meta persistence or disk destruction.

## Identity and flow

Use the [corrected archive async baseline](../20261001-round1-corrected-package/README.md):
central memory Meta, N2/M1, independent OwnerFs/DFS FUSE mounts, Auto with real RXE,
4 MiB +17 bytes acknowledged by data/full sync and close. Node and Meta bytes
remain unchanged from the qualified source gate. No Rust, protocol, format,
module or product-interface change is made here; exact-input stage evidence is
reused. All operations run on the locked ARM64 Linux/ext4 guests, with build off.

| Fault flow | Evidence and consequence |
| --- | --- |
| Cold remote Node, then Home/source A SIGKILL | [B initial storage](b/physical-before.json) has no durable Chunk; B is normally restarted before reading. [A kill](a/home-kill.json) binds `/proc` binary, boot, PID and start ticks before pidfd SIGKILL. Meta stays running |
| Owner unavailable, DFS surviving-source read | [Owner open](b/owner-home-down.json) returns EHOSTUNREACH/zero bytes. [DFS cold read](b/dfs-source-down.json) returns exact original bytes and EOF while A remains stopped, rather than an incorrect success or mixed version |
| Source failure repair | [Expired Home and copy state](ctl/source-outage-repaired.json): A unavailable; live copies B/C satisfy N2, both repair tasks reach Completed attempt2. [B physical data](b/physical-source-outage.json) matches expected Chunk hashes. Counters and C log prove actual RXE repair, with zero DFS gRPC file payload |
| Original-disk A recovery | Installed controller removes stale PID and recovers disconnected mounts in [restart](a/home-restart.log), without manual UDS removal. [Physical Home/chunks](a/physical-restarted.json) are unchanged. [REST](ctl/snapshot-after-home-restart.json) keeps Home A and advances its session/lease; remote Owner/DFS reads match acknowledged bytes |
| Actual replica C interruption | [C kill](c/replica-kill.json) binds its process identity. [Authority observation](ctl/target-outage-qualified.json) shows C unavailable and live A/B copies satisfying N2. The first fault already created an extra retained copy; therefore this second fault does **not** create new under-replication debt |
| C original-disk rejoin | [Restart](c/replica-restart.log) and [physical copies](c/physical-restarted.json) match; first Owner/DFS reads after rejoin pass. [Final authority](ctl/snapshot-final.json) serves Home and reports sufficient live copies |
| Authority continuity | Meta identity [before](ctl/identity-before-qualified.json), [during](ctl/identity-during.json) and [after](ctl/identity-final.json) is identical, including boot/start ticks/binary. No memory-Meta restart durability claim |

## Verification boundary

[Linux semantic audit](audit.json) passes 48 checks and [negative checks](audit-regression.json) pass eight checks; both
bind fault identities, cold source read, expected errors, physical hashes,
repair records, Home session transition, process recovery and authority continuity.
The production source gate is reused; no unrelated full Rust gate, complete
POSIX, performance matrix, 8 GiB or soak is repeated for probe-only changes.

One initial Meta identity constant in the probe omitted characters; its
pre-injection guard refused execution. The corrected guard identifies the actual
Meta executable. An initial observation asserted that both chunks must still be
under-replicated when polled after Home expiration: repair had already completed
on B. Its [raw observations](ctl/source-outage-observations.json) are retained;
that transient was not observed by this late polling and is not claimed as
qualified. Actual dead A, live B/C, task attempt2, matching physical copies and
first cold surviving-source read independently establish the scoped recovery.
An export-only chown initially used the guest home-directory name as a username;
export was corrected using the real numeric uid/gid. These preparation errors
are separate from product results.

Still required in ROUND2: product network interruptions, physical ENOSPC/EIO,
data corruption, exact unknown-commit replay across relevant integrations and
remaining business RDMA completion/cancellation/teardown faults. Persistent Meta
recovery is ROUND3; full fault/transport/backend matrices and stability are not
qualified by this slice. Active priorities remain in [issues](../../issues.md).
AGENTS and handoff are unchanged. Data and earlier failed observations remain
available; all four current baseline processes are running after recovery.
