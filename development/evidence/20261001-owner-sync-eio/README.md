# OwnerFs fatal sync errors and physical EIO recovery

**Scoped fault integration and final Linux source gate PASS. ROUND2 remains incomplete.**
Formal status stays **69 NOT_RUN / ENV PREPARING**. The implementation changes only
`src/node/vfs/ownerfs.rs`; protocols, persistent formats, public interfaces,
dependencies and process/module boundaries are unchanged. AGENTS and handoff are unchanged.

## Defects and contracts

[Original physical fault](original/a/evidence/fault-ownerfs.json) admits an 8 KiB
Owner append, then returns EIO from fdatasync and fsync, but incorrectly returns
success from close. [Cold reload](original/a/evidence/verify-old.json) loses that
unconfirmed append. DFS correctly returns EIO from all three barriers.
The original failure and its old production identity remain intact.

The open handle now retains its first fatal storage error from local or remote
write/sync. Later write, handle resize, flush and sync return that error; reads and
release remain available. A consumed native writeback error cannot make close
successful. Capacity, timeout and unavailable errors retain their existing retry
behavior. Recovery requires repaired storage, a new handle, explicit rewrite of
unconfirmed data and a successful barrier. Mutable Owner files do not promise
rollback after a failed sync.

[The original resize-only regression](build/resize-original/test.log) separately
proves a remote caller skipped Home flush after successful handle resize. The
caller now marks that handle dirty. Its next flush invokes Home; a hard Home error
is retained. This adds no RPC type or state-machine abstraction.

## Actual product flow

The new candidate runs separate memory Meta on ctl:20280 and a Home Node on
A:20282, with independent FUSE mounts, mTLS and gRPC. An exclusively owned **32 MiB
logical dm/loop/ext4 image** on A's locked guest ext4 volume backs its data.
Logs and evidence remain outside the fault volume. Only A participates in the
initial DFS fault, with the unchanged N2/M1/local-required policy; this is not
proof of two synchronous copies or the complete fault/transport matrix.
B:20284 is a separate Owner-only ingress for the subsequent remote flow.

| Check | Observed result |
| --- | --- |
| [Seed](fixed/a/evidence/seed.json) | Both 8 KiB files complete write/fdatasync/fsync/close and exact content/length/EOF; backing volume is synced |
| [Physical fault](fixed/a/evidence/fault-observed.json) | UUID-bound dm table switches to `error`; kernel reports actual buffer and ext4 I/O errors. Owner and DFS ordinary writes admit 8 KiB; fdatasync, fsync and close each return EIO |
| [Cold reload](fixed/a/evidence/verify-old.json) | Identity-bound SIGKILL, restore/remount and installed-controller start preserve the acknowledged Owner prefix and original DFS content. Owner's unconfirmed tail happens to survive this attempt; rollback is not implied |
| [Recovery](fixed/a/evidence/recover-after-session.json) | New handles explicitly rewrite 8 KiB and complete both syncs/close. [Normal restart](fixed/a/evidence/verify-new-final.json) retains exact new content/length/EOF |
| [Remote physical fault](fixed/a/evidence/fault-remote-observed.json) | Same owned device faults again. B's accepted ordinary append later returns EIO from both syncs and close; its O_DSYNC write itself returns EIO, followed by EIO from sync/close |
| [Remote resize close](fixed/a/evidence/resize-close.strace) | A separate healthy resize/close captures Home `fdatasync` on the actual `remote-size.bin` file. [A/B normal restart and fresh remote read](fixed/b/evidence/cold-read-final.json) verify 24 KiB, exact acknowledged prefix and zero tail, plus the recovered 8 KiB remote file |
| [Cleanup](fixed/a/evidence/cleanup-completed.json) | A/B/Meta stop normally; owned mapper and loop detach, image remains inspectable, outer data reserve exceeds 4 GiB |

[Meta identity](fixed/ctl/evidence/identity.json) equals its initial identity across
all Node interruptions. Final [A](fixed/a/evidence/binding-final.json),
[B](fixed/b/evidence/binding-final.json) and [Meta](fixed/ctl/evidence/binding-after-remote.json)
bind actual executable hashes, config paths, health and independent mounts.
[Candidate hashes](build/artifacts/binary-sha256.json) come from the Linux build;
all guests use the existing installed process controller.
The older round-1 async cohort was observed healthy and retained, not upgraded or
requalified by these new runtime results.

## Retained unsuccessful observations

- [First source fixture failure](build/local-original/owner.log): `fsync` injection
  hit root-catalog setup. A no-I/O test catalog isolates the target explicit-sync
  path. That unit test does not prove native O_SYNC pwrite EIO.
- [Immediate DFS recovery](fixed/a/evidence/recover.json) returned EBUSY while its
  previous owner lease was still fenced. An explicit later retry succeeded on
  the same cold Node; no lease threshold was shortened.
- [Remote fixture failure](fixed/b/evidence/remote-result.json) incorrectly required
  resize-only close to return EIO. That resize preceded the fault by about 21 s;
  [cold length/zero tail](fixed/a/evidence/resize-cold-first.json) remain correct.
  Closing already journaled metadata can succeed. The ordinary/O_DSYNC error
  observations stay valid; the original outer FAIL is not relabeled PASS.
- B's first config combined Owner-only mode with a DFS mount and was rejected.
  [Original config](fixed/b/evidence/invalid-node-original.toml) and startup logs
  remain; [corrected config](fixed/b/evidence/prepare-corrected.json) removes that
  mount. The first cleanup predicate also incorrectly required removal of the
  stopped PID file; corrected cleanup verifies the process is gone.
- [Initial audit](audit-initial.json) incorrectly required a readonly remount as
  an immediate EIO side effect. Fixed local raw logs prove I/O errors without that
  asynchronous side effect. The final audit keeps this unproved limit explicit;
  it does not qualify a forced-readonly case or change acceptance.

## Validation and scope

[Local regressions](build/local/owner.log): **69 Owner tests PASS**, formatting and
all-target/all-feature compile PASS. [Final source gate](build/full/gate/lib.log):
**421 library PASS / 6 explicit environment ignores**, 65 interface tests,
4 shared-error tests, 9 local-API tests and 5 privileged FUSE tests PASS, plus
strict all-workspace/all-target/all-feature Clippy, five feature builds and
Node/Meta builds. Original and intermediate gates keep their original inputs.
[Before/after compiler inputs](build/compile-inputs-after.json) match for 143 files.

[Final Linux semantic audit](audit-final.json) passes **63 checks and 8 negative
variants**, including rejection of false fixed close, no physical fault, changed
Meta, wrong cold bytes, bad executable/config and hidden original failure.
A source gate is required for this production batch; script-only audit corrections
reuse it. No full POSIX, paired performance, 8 GiB, soak or formal case PASS follows.
Native O_SYNC pwrite, forced readonly and remaining backend/transport/device axes
remain outside this evidence. SDK/cache/spill/Meta HA remain outside first-stage scope.

[Final publication audit](verification.json) binds all143 product compiler inputs
to the executed candidate, proves only ownerfs.rs changed relative to the prior
round-1 source gate, checks112 local file links and records242 artifact hashes.
Protected AGENTS/handoff hashes are unchanged. [Read-only review](review.md)
reports no new blocking finding and retains its independent-test limitation.
