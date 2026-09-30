# Background writeback and identified v48 integration

## Verified source boundary

v49 adds a fair background writeback pass with at most64 examined inodes and one250ms admission/RPC budget. The cursor advances by the number actually examined, including clean/busy entries. Concurrent maintenance is skipped. Production lease renewal, file commit and metadata sync use the remaining budget capped by configured RPC timeout. Foreground sync and uncertain-commit replay keep their ordinary timeout.

Expired-before-send prepared work retains the complete `CommitFileVersion` (operation, version, layout, lease, CAS and receipts), releases only the busy flag, and can replay the identical request. Issued timeout remains unknown. A returned successful Meta result is applied even after budget expiry. A post-renew check prevents starting new prepare after the deadline.

Linux v49:350 library PASS/two explicit environment ignores;57 interface contracts,four shared errors,nine local API tests,five privileged actual FUSE tests; fmt,strict all-workspace/targets/features Clippy,none/OwnerFs/DFS feature checks and binary build pass. All143 compile inputs match. [Gate report](qualified-linux/report.json), [commands/logs](qualified-linux/checks.json) and [source manifest](compile-inputs.json) bind the result.

Node SHA `dbbf2ccd5eb47f06178bfc3140599865c1b7ad5f28851e244e728fbcdfc136af`; Meta SHA `917432057950380da208b057a4d94156841e1eab03e533f56675433d5184fd9c`. Artifacts are `/home/lzc.guest/afs-build/artifacts/v49-qualified` on the Linux build VM. These binaries are not yet deployed to A/B.

Three separate injections of old unbounded scan, sending after expired prepare and fixed configured RPC timeout fail their new regression. The production HTTP2 timeout fixture initially failed its overly narrow error-code assertion: the configured endpoint timer can return untyped tonic Cancelled/"Timeout expired" before the equal outer timer. [Original](initial-linux/target-rpc.log) and [diagnostic](rpc-error-diagnostic.log) remain unchanged. The corrected test preserves that exact transport identity for the configured-cap case; the three shorter caller-budget cases require `CLIENT_DEADLINE_EXCEEDED`. It does not accept arbitrary remote errors. No production error mapping was changed.

The initial orchestration shell lacked Cargo PATH before any test ran; [raw failure](format-first.log) is retained. The corrected gate runs entirely in Linux.

## Identified live runtime, separate binary version

A/B v48 memory Meta/R1/gRPC/TLS runtime has Node SHA `f84b641409d1f60de5e811122f0e7e2694d31b581823d75b3406996a1b875f39`, Meta SHA `f96b6a06047da02bf186ea8d89457ee74c454d6c437915f5c32a20a9b909f312`. A Node707590/Meta707560 uses17980..17983; B Node67588 uses17984..17985. Prior v45 services/data remain preserved.

Ten A/B consistency scenarios and seven distributed-lock steps per backend pass. Original35-second wait/55-second child bounds remain. [248 exact captured process records](runtime-v48/identity-verification.json) match stable incarnations and independent Linux kernels. Four `findmnt` observations on intentionally removed fixture paths have no target; they are retained explicitly and do not stand in for root mount qualification.

Full B DFS pjdfstest with strict A Meta/B Node process, TLS, endpoint, mount and guest-ext4 pre/post checks passes236/236 discovered and completed files,8819 TAP checks,zero unexpected failures/skips and28 upstream TODO. Duration1253.484s is within the unchanged1800s bound. [Full report](runtime-v48/full-dfs-remote-r2-report.json), [raw TAP](runtime-v48/full-dfs-remote-r2-raw/artifacts/std-01-pjdfstest/pjdfstest.stdout.tap), [retrieved artifact verification](runtime-v48/full-remote-artifact-verification.json). Seven worker/host manifest entries match exact bytes and SHA. The first attempt was BLOCKED before suite invocation because the guest evidence path did not exist; original output/proof remains in `full-dfs-remote-host` and `full-dfs-remote.stdout`.

This proves that specific v48 development matrix. It does not qualify v49, the OwnerFs full current matrix, durable backends, R=N/RDMA or all formal release gates. Earlier v40 full remote timeout remains its original BLOCKED result.

## Open boundaries

- Snapshot/sort of the inode table and already-started synchronous chunk preparation/disk IO are not bounded by250ms. Placement/R=N transfer deadlines and a shared whole-maintenance budget remain open. Workers are retained, not cancelled and mislabeled complete.
- [Linux native exit fixture](runtime-v48/processctl-exit-original-report.json) reproduces a controller defect: `afs-processctl stop` returns0 for child exit1 and124. Normal fixture exit0 passes. This is an original FAIL, not product shutdown acceptance. Controller exit-status propagation is the next independent fix.
- Full fault/backend/replica/RDMA/installation/performance/large-file/soak matrices remain unqualified. Formal69 cases are NOT_RUN and ENV PREPARING. Overall goal remains active.
- `docs/handoff.md` is unchanged; it refreshes only when explicitly requested.

[Artifact manifest](artifacts.json) records SHA256 and size for every raw source/runtime file in this directory; README and the manifest itself are excluded.
