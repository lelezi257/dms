# Node shutdown owns FUSE callbacks and blocking work

Date: 2026-10-01. Candidate v48, Linux ARM64. Focused memory-Meta, R=1, gRPC development evidence; no full acceptance or performance claim.

## Results

| Check | Result | Evidence |
| --- | --- | --- |
| Busy inode holds global table / background waits for active commit | Two regressions FAIL with original behavior | [Original log](original-background.log) |
| FUSE queue drop returns before accepted jobs finish | One regression FAIL with original behavior | [Original log](original-fuse-drop.log) |
| Final coherent Linux gate | 343 library PASS / two explicit environment ignores, 57 contracts, four shared errors, nine local API and five privileged FUSE tests PASS; fmt, strict Clippy, feature checks and bins build PASS | [Log](linux-qualified-gate.log), [commands](linux-gate.sh) |
| Input identity | All143 host inputs match Linux capture and final product code | [Host](host-source-hashes.json), [Linux](compile-inputs.json) |
| Watchdog child process | Blocked Tokio runtime destruction exits124; explicit completion stays alive and exits0 | `runtime::shutdown_deadline_tests` in the Linux log |
| Actual healthy Node stop with an open writable fd | Exit0 in15.719ms; restart's first read returns all14 accepted bytes | [Report](runtime-v48b-report.json), [Node log](runtime-v48b-logs/logs/node-normal.log) |
| Actual paused Meta during dirty drain | Verified SIGSTOP; Node exits1 in5031.857ms with explicit drain/shutdown errors | [Report](runtime-v48b-report.json), [Node log](runtime-v48b-logs/logs/node-normal-restart.log) |
| Resume Meta and same-disk Node restart | First read retains acknowledged `seed`; prior14-byte watermark unchanged; final Node and Meta exit0 | [Report](runtime-v48b-report.json) |

The five privileged FUSE tests are first discovered as explicit ignores by the unprivileged contract run and then executed as root. They are not counted twice. The two library environment probes remain ignored. Child-process test output is nested inside the library log, not additional library-suite passes.

## Lifecycle contract

Background writeback snapshots inode references while holding the inode table, releases that table, and uses nonblocking admission to skip an occupied inode. It does not wait for an already active commit. Foreground inode modification/commit ordering and the exact unresolved Meta request remain intact.

Shutdown stops the session receive loop using a session-owned Unix socket, joins the FUSE session, drains accepted dispatch jobs and completes lock-session cleanup before the final DFS dirty drain. `MountedFuse::join` propagates saved cleanup errors. The existing vendored fuser receives a stop wakeup and retains its receive-error behavior; a busy mount uses its existing lazy-detach fallback. A session-owned wakeup does not close a possibly reused descriptor. An old application fd is disconnected after Node shutdown (the probe observes ENOTCONN); it cannot continue accessing a terminated process.

The production `afs-node` binary arms one monotonic15-second native watchdog when service stop begins. It remains armed across FUSE teardown, blocking drain, Tokio runtime destruction and observability guard destruction. A normal error can exit1 earlier. If teardown cannot finish, the watchdog exits124 without waiting for logging or destructors. Forced exit is failure, not graceful drain or cancellation of a blocked worker. Embedded callers of the existing `node::run` do not receive an implicit process watchdog.

## Failures retained

- v46/v47 source gates passed, but v47 real healthy stop failed: an open fd made plain unmount return EBUSY, leaving the receive thread blocked. The watchdog exited124 after15053.057ms. [Original runtime report](runtime-v47-original/report.json) and [Node log](runtime-v47-original/logs/logs/node-normal.log) are retained. v47 is not qualified for healthy shutdown.
- The first v48 fixture incorrectly required every paused-Meta failure to reach the watchdog. The product instead returned the per-RPC timeout as exit1 in5024.962ms, after a successful healthy stop/readback. [Original fixture](https://github.com/lelezi257/dms/blob/e925c5bcf0408851ebfa08a59df29953374da9e9/development/evidence/20261001-node-shutdown/runtime-v48-fixture-assertion/probe.py), [failure report](runtime-v48-fixture-assertion/report.json) and [stderr](runtime-v48-fixture-assertion/stderr) remain FAIL. The corrected [probe](https://github.com/lelezi257/dms/blob/e925c5bcf0408851ebfa08a59df29953374da9e9/development/evidence/20261001-node-shutdown/shutdown-probe.py) accepts exit1 only with both drain and shutdown error records, or exit124 after the watchdog interval. Both must finish within the unchanged19-second outer limit.
- Initial fmt/Clippy failures and earlier draft gates remain separate logs. None substitutes for the final coherent gate.

## Identities and scope

Node SHA256: `f84b641409d1f60de5e811122f0e7e2694d31b581823d75b3406996a1b875f39`.

Meta SHA256: `f96b6a06047da02bf186ea8d89457ee74c454d6c437915f5c32a20a9b909f312`.

Immutable binaries: `/home/lzc.guest/afs-build/artifacts/v48-qualified`. Actual fresh runtime: `/mnt/lima-afsadata/afs-delivery/p2-shutdown-v48b`, guest ext4, ports17980–17983. The report records executable hashes, exact PIDs/start ticks, boot ID, config hashes, actual mounts and fault state. The fixture terminates only its own child processes and restores paused Meta. Existing A/B v45 runtime is unchanged; its earlier consistency and lock evidence does not qualify v48.

The timeout write `next` has an unknown outcome: the probe permits only the complete acknowledged head or the complete later head, never a mixture. This run reads `seed`. Meta memory state stays live, so no Meta restart durability is inferred.

## Remaining validation

Nonblocking inode admission does not yet bound candidate count or every background RPC/put across a whole writeback pass. A poll-to-read race or physical I/O can still block; the process deadline reports failure rather than promising cooperative cancellation. Mount-path reuse by an unrelated process is outside the private runtime-path trust boundary. Full OPS/DEP stop status propagation, current A/B consistency/locks, whole lifecycle/backend/replica/RDMA matrices, fair performance and final long tests remain open. Formal69 cases remain NOT_RUN and ENV PREPARING. `docs/handoff.md` is unchanged.
