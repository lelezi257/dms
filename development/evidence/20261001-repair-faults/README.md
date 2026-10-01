# Replica recovery and crash restart

## Scope and result levels

This candidate fixes retired repair-worker claims, repair debt without spare
capacity, stale multi-replica placement and disconnected FUSE restart. The
initialization policy remains N desired / M synchronous copies. No public
module, RPC, wire format, dependency or file durability contract changes.

The complete Linux source gate passes for v60. The affected A/B fault flows pass
as short development integrations. These establish a **stage gate**, not
formal acceptance: all69 release cases remain NOT_RUN and ENV PREPARING.

[Validation strategy](../../validation.md) selects original failures and related
regressions for small edits, affected integrations for related batches and the
complete source gate at batch/stage boundaries. The final shell-only repair
reuses the unchanged143 Rust compile inputs and qualified binaries; its own
Linux regressions and actual restart are fresh. Full POSIX,8GiB,performance and
long stability matrices remain scheduled release work.

## Source and controller qualification

[Linux gate](source/qualified-linux-clean-r2/report.json) and
[143 compile inputs](source/qualified-linux-clean-r2/compile-inputs.json) bind
ARM64 Linux6.8.0-106, formatting, strict all-target/all-feature workspace Clippy,
feature checks and binary build. Library375 pass with2 existing environmental
ignores; contracts58, shared errors4, local API9 and actual privileged FUSE5
pass. [Source audit](source/root-source-inputs-audit.json) matches host inputs.

[Original source diagnostics](source/source-diagnostics/) retain worker-reclaim,
expired-session,debt-persistence and cached-placement failures. The final
[Meta module run](source/source-diagnostics/meta-fixed-v59-r2.log) passes14
selected recovery tests. An intermediate RN refactor failed strict Clippy for an
unused engine provider; [failure](source/failed-linux-unused-field/qualified-linux-clean/clippy.log)
is retained. The final private engine refreshes its provider once per RN batch.
R1 stays local; N2/M1,N2/M2 and multiple chunks verify the fresh epoch and one
refresh. No extra per-chunk placement request is introduced.

[Controller qualification](source/controller-gate/report.json) runs the new
regressions against both old and fixed controllers on Linux. The old controller
fails; the fixed controller passes19 groups, retaining54 existing native CLI
command records. Recovery checks exact configured target,FUSE fstype and AFS
source, and requires bounded GNU stat rc1 with explicit ENOTCONN. Live mounts,
foreign mounts,timeouts and unknown errors are refused. Both configured mounts
are examined even without a PID record, before mkdir. GNU timeout is the
existing coreutils dependency; Shellcheck was unavailable and is not claimed.

Qualified unstripped binaries are bound to the actual debug-stripped runtime by
[artifact mapping](runtime/build-artifact-sha256.txt). Runtime Node SHA256:
`53da41fb54bcabbfe567bef91096891274ea36b3d28d989dba372fc6940c12ea`.
Runtime Meta SHA256:
`37218622c1cfd1cfadc696269f0ea01a2570052ec460528c748d525fe982ef9d`.
Controller SHA256:
`78949590bfdf12f6db0977ac5756702312e85e1d91c8f59903d4490222488f2b`.

## Actual Linux fault flows

All data resides on A/B guest ext4. The isolated runtime is repair-v60-a/b,
18480..18485, memory Meta,TLS/gRPC,N2/M1 and separate OwnerFs/DFS mounts. A/B
observed kernel6.8.0-142 differs from the build kernel; the formal environment
is still PREPARING. These runs do not establish persistent Meta recovery.

1. [Preparation](runtime/prepare.complete.json): A alone fsyncs1MiB with one copy
   and Pending work. B joins; two available copies and Completed work appear.
   Physical B Chunk and FUSE bytes match. Controlled B stop0/restart uses the
   same binary/config and retains exact bytes.
2. [Target outage](faults-v60/target-outage.complete.json): stop B, prove one
   available copy with Pending repair debt and no confirmed loss, read A's
   fsync watermark, then restart B and restore two available copies.
3. [Fresh source outage](faults-v60-r2/source-outage-run.json): create a distinct
   exclusive1MiB file,fsync and verify two copies; SIGKILL the exact A Node and
   prove /proc disappearance within2s. B's first read of this file,while A is
   down,matches the original checksum. Controller restart cleans both actual
   disconnected A mounts,starts a new Node incarnation with the same binary
   and config,and restores two copies. A Meta stays the same incarnation.

The source-outage file is repair-v60-source-loss-r2.bin; SHA256:
`5d03f17fd63f2eaa38f058d50bcb9174aa5cd3a3b6a41d739a1a486a13b8c26b`.
The [run inventory](faults-v60-r2/source-outage-run.json) binds one fresh UUID,
file bytes and SHA values. Probe preflight refuses an existing run directory;
source execution rejects prior completion markers. Original runs retain their
own identities and results.

## Independent verification and retained failures

[Final root audit](faults-v60-r2/root-independent-audit-v2.json) checks live
A/B process identity,config,binaries,mounts,physical B Chunk and FUSE bytes,
live REST,two-copy health,143 compile inputs and handoff SHA. It verifies the
B controller installation followed an absent old process,that B restart kept
binary/config and changed PID/ticks,and fresh controller hashes on both nodes.
[Two-node receipt](faults-v60-r2/root-independent-controller.2.json) is the final
controller audit; the unsuffixed earlier receipt covers only A. Read-only
[Meta](meta-review.json),[controller](controller-review.json) and
[proof review](proof-review.json) report no scoped blocker. Reviewers did not
build,test or run probes; root owns runtime verification.

- [Original v56 fault attempts](failed-v56/) and original source diagnostics
  remain unchanged.
- [Original v60 source restore](faults-v60/source-outage.restore-failure.json)
  remains FAIL: B served the bytes,but old processctl hit disconnected mkdir.
  It has no source-outage completion PASS.
- [Diagnostic controller recovery](source/controller-recovery/) restored A's
  mounts,but its120s two-copy wait failed because B had independently exited
  on expired NodeSession. It remains a failed diagnostic,not source-outage PASS.
- [Session observations](source/session-expiry-diagnostic/) retain new/older
  expired Nodes. The cause is unproven; host suspension is not asserted.
  [Separate B recovery](source/session-expiry-recovery/recovery.complete.json)
  restores B and two copies before the distinct fresh source-outage run.
- The older v51/v55 Nodes were already exited before fresh r2. The
  [unrelated snapshot](source/session-expiry-diagnostic/unrelated-before-r2.json)
  and final audit establish unchanged observed state,not historical liveness.
- [Reproducer syntax](source/reproducer-syntax.json) checks all11 actual Python
  source hashes on Linux. [Host preparation attempts](syntax-attempts.json)
  record an unavailable host share and AppleDouble archive metadata; neither
  qualifies product behavior. No metadata files are published as source.

[Manifest](manifest.json) covers every published evidence/reproducer byte.
Reproducers retain their research-workspace relative paths under reproducers/;
restore that layout and original evidence inputs before invoking them. The
publication flattens the nested source-gate directory only; original reports
and commands retain their original paths and bytes. docs/handoff.md is unchanged.

## Remaining qualification

Missing placement errors and retired Running tasks without replacement capacity
remain broader follow-ups. Unexpected idle session expiry needs isolation and
lifecycle investigation. Real claim/report ACK loss,corrupt copy repair,actual
RXE repair/security/resources,durable backends,complete deployment/POSIX,fair
MooseFS/3FS baselines and long stability remain required. These short successes
do not complete the delivery goal.
