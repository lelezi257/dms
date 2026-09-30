# Recover durable reads after owner process restart

Date: 2026-10-01. Candidate v45, Linux ARM64. These are bounded development checks with memory Meta, R=1 and gRPC file data. They do not qualify the complete release matrix.

## Verified results

| Check | Result | Evidence |
| --- | --- | --- |
| Old serving-epoch rejection and lost timeout debt | Two new regressions FAIL with old behavior | [Injection log](linux-original-injections.log) |
| Coherent source gate | 337 library PASS, two explicit environmental ignores; 57 contracts, four shared errors, nine local API tests and five privileged actual FUSE tests PASS | [Linux gate](linux-qualified-gate.log), [exact commands](linux-gate.sh) |
| Formatting, strict all-target/all-feature Clippy, feature checks and build | PASS | [Linux gate](linux-qualified-gate.log) |
| Compile inputs | All143 host/Linux inputs match | [Host](host-source-hashes.json), [Linux](linux-source-hashes.json) |
| Restart probe selftests | Four PASS, including immediate first-open error; Linux ext4 probe selftest PASS | [Probe log](linux-probe-selftest.log) |
| Actual owner SIGKILL, retained Meta and same-disk restart | 64MiB durability watermark survives; first fresh B read matches length and SHA256 | [Fault report](v45-recovered-fault/report.json), [qualification](qualification.json) |
| A/B consistency after restart | Ten of ten PASS | [Report](v45-consistency/report.json) |
| DFS / OwnerFs cross-node locks | Seven of seven PASS each; original35s wait and55s interrupt bounds | [DFS](v45-dfs-cross/report.json), [OwnerFs](v45-ownerfs-cross/report.json) |

All248 captured product-process identity records in the consistency and lock reports match the candidate hashes and exact live PIDs. Workers run on distinct Linux kernels with actual independent AFS mounts. Assertions do not poll until stale data disappears.

## Original failure

The preceding v44 runtime passed consistency and DFS lock short checks but failed the real owner crash slice. The original restart exited because its Unix socket survived SIGKILL. After identity-checked manual socket removal for diagnosis, the first fresh B read still returned EHOSTUNREACH: Meta rejected the recovered disk copy because its receipt recorded the old process epoch. This failure and the manual intervention remain [raw evidence](v44-original-fault/report-failure.json), including the [first-read result](v44-original-fault/guest-state/diagnostic-once-result.json). The diagnostic restart is not a clean recovery PASS.

An initial controller trigger had a syntax error, and an earlier diagnostic checker polled read errors before timing out. Both raw attempts remain in the original directory. The revised checker performs one open/read under a deadline and reports the first error. Initial compile errors, the stale-socket inode-reuse test fixture failure and the zero-test filter attempt are preserved separately. None is counted as a passing test.

## Recovery boundaries

### Durable receipt and serving process

Persisted `CopyRecord` and receipt authority remain unchanged. Both Meta source issuance and receiver validation use one read-only serving projection. It requires a Ready durable copy, current live matching Node, non-regressed process epoch and the same persistent device ID/epoch. Across process epochs, the recovered catalog revision must cover the copy receipt. Within the same process, later writes may exceed the startup descriptor and remain readable. A fresh grant signs the current serving epoch; an old grant is denied by the replacement process. Actual disk reads still verify the chunk digest.

Four regressions cover real R1 store reopen, fresh/old grant outcomes, unchanged receipt/placement, stale or replaced devices/catalogs and invalid sessions. This adds no RPC or wire field, and changes no write fencing. Future catalog deletion/quarantine must keep authoritative copy state consistent; a catalog revision floor is not a blanket proof that every object remains present after an unreported deletion.

### Stale socket and startup ownership

Node holds a nonblocking sidecar lock for the local socket's lifetime. Only a socket that refuses connection and still has the observed device/inode is removed. Active listeners, regular files, symlinks, unknown connect failures and replaced paths remain intact. Cooperating Node startups use the same lock inode; the sidecar is not unlinked. Eight focused tests and the existing nine local API tests cover these boundaries. The private runtime path remains a trust boundary; this is not an atomic compare-and-unlink guarantee against unrelated processes replacing that path.

### Provider timeout and cleanup ownership

A two-second provider idle timeout reports the close error but queues the exact owner handles together with the shared provider lifetime. Reserved capacity becomes pending debt. Same-owner maintenance waits for admitted I/O to drain before Release; authoritative owner replacement or expiry retires only the old identity. Unknown Meta results retain debt. The new regression holds an I/O guard across the timeout, verifies both handles stay live, then drops the guard and drains both releases. Existing64-entry/250ms maintenance/2s drain/100ms RPC limits remain unchanged. No network call holds the provider or cleanup-table lock.

## Actual fault sequence

1. A creates the file; B keeps an O_RDWR remote-owner handle, writes64MiB and completes fsync plus exact readback in2047.896ms.
2. The controller verifies executable hash and process identity, then SIGKILLs A Node706396. A Meta706366 and B Node67058 remain unchanged.
3. B closes the old handle. It completes in0.726ms with explicit EIO; no silent successful close is reported during unavailable owner authority.
4. A restarts the same binary, config and disk. The controller unmounts only the exact dead-process FUSE mounts; it does not remove the socket. Node706539 becomes ready in287.851ms, with a new process session and serving epoch2.
5. B's first fresh open/read completes in14258.667ms, with67108864 bytes and SHA256 `e6e17e2df5d8d41949a9c0e1b5ffa7595008811937b9aebb1b17d1332e9e276d`.
6. B logs one queued cleanup retirement after replacement and before the old session's original expiry. This proves the observed single-handle retirement path; it does not prove the full cleanup-capacity or network-fault matrix.

Meta memory state stays live throughout. This proves Node crash recovery, not Meta restart durability, disk loss, multiple replica recovery or Redis/etcd parity. The14.3s read is a functional observation, not a qualified performance result.

## Candidate identity and remaining gates

Node SHA256: `d8937569feaee6026fdf14941e0b4f742c935a17798aeefd223f26e578fd49e8`.

Meta SHA256: `8bcfbcbd2f84be5029ab848c214aaff258ba8147684a141b17b262d0b6cc8d10`.

Immutable Linux artifacts: `/home/lzc.guest/afs-build/artifacts/v45-qualified`. Runtime directories are `/mnt/lima-afsadata/afs-delivery/p2-memory-lane-v45` and `/mnt/lima-afsbdata/afs-delivery/p2-memory-peer-v45`, isolated17880–17885 ports.

Read-only review found no correctness blocker in these three fixes. It noted a low-priority diagnostic gap: a second socket metadata error is reported as identity change instead of preserving the original filesystem error. Overall dirty-writeback/shutdown budget, current full POSIX/backend/fault matrices, multi-replica repair, RDMA security/faults, fair performance baselines and final long-run/deployment gates remain open. Formal69 cases stay NOT_RUN and ENV stays PREPARING. `docs/handoff.md` is unchanged.
