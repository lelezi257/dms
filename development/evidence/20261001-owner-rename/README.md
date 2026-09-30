# OwnerFs overwrite rename validation

Date: 2026-10-01. Linux ARM64 development lane, memory Meta, independent A/B FUSE mounts. These results do not qualify the full release, persistent-backend, performance or deployment matrix.

## Results

| Check | Result | Evidence |
| --- | --- | --- |
| Original v36 full OwnerFs pjdfstest | FAIL: 236 files / 8819 checks, ten unexpected rename failures | [Original proof](v36-original/full-236-ownerfs-standard-20260930T175904Z/artifacts/std-01-pjdfstest/proof.json) |
| Two regressions with original rename logic | Both FAIL with stale identity | [Before](v37/linux-before.log) |
| Two regressions with canonical rebind | Both PASS | [After](v37/linux-targeted-after.log) |
| Formatting, strict Clippy, feature checks and binaries | PASS | [Linux gate](v37/linux-integration.log) |
| Library / interface / shared error / privileged actual FUSE | 291 / 57 / 4 / 5 PASS; two explicit library environment ignores | [Linux gate](v37/linux-integration.log) |
| Compile source identity | All 143 host/Linux inputs match | [Manifest](v37/linux-source-hashes.json) |
| Probe Linux selftests | Four PASS, including ten positive reference scenarios | [Selftest](v37/linux-probe-selftest-final.log) |
| Actual A/B OwnerFs/DFS consistency | Ten PASS, including local and remote-Home hardlink rename | [Report](v37/consistency/report.json) |
| v37 full OwnerFs pjdfstest | PASS: 236 files / 8819 checks, zero unexpected failures/skips, 28 upstream TODO | [Qualified result](v37/full-owner/ownerfs/qualification.json) |
| v37 full DFS pjdfstest | PASS: 236 files / 8819 checks, zero unexpected failures/skips, 28 upstream TODO | [Qualified result](v37/full-dfs/dfs/qualification.json) |
| v37 actual OwnerFs cross-mount locks | Seven PASS, including 35-second waits | [Report](v37/owner-cross/report.json) |
| v37 actual DFS cross-mount locks under namespace load | FAIL: six PASS, interrupted blocking wait exceeds 55 seconds | [Original failure](v37/dfs-cross/report.json) |
| v37 actual DFS cross-mount locks after namespace load ended | Seven PASS; diagnostic repeat, not a repair qualification | [Repeat](v37/dfs-cross-repeat/report.json) |

## Mechanism

Replacing `dst` removes that directory entry. A separate hardlink `dst.lnk` still names the original inode. `OwnerState::rename_path` preserves that alias and rebinds the cached inode's canonical path before rebuilding the identity index. Without rebind, inode-based getattr follows the replacement through the old `dst` path and reports ESTALE.

Local rename, Home-side peer rename and remote caller cache updates share this helper. The fix adds no RPC, persistent type or public interface. Same-inode rename, directory descendant moves and open-unlinked handles remain covered by the Linux gate. A bounded independent [source review](v37/bounded-review.json) found no blocking issue.

## Identities and boundaries

Node SHA256: `143294f76870794aab05f7f314287622367914f98b329c98b5fccd3a60a88faf`.
Meta SHA256: `ffe7268031438318de65ee999e4872846974784141fba4d85ad451865a958d01`.

A Node PID576329 / Meta PID576299; B Node PID8811. PIDs and VM paths are observed run identities. The cross-worker report captures separate boot IDs, exact executable SHA/start time and actual mounts. Full-suite qualification verifies Node/Meta identity, boot ID, configuration digests and mount identity before and after execution. Complete raw TAP/accounting is retained. Assertions do not retry stale reads; rename fixtures reject existing run directories.

All compilation, tests and file I/O run on Linux. macOS performs editing, copying, hashing and VM orchestration. No executable, build cache, private key or VM disk is published here. No test was excluded after failure. Both complete suites qualify this v37 development candidate. The separate DFS lock cancellation failure remains open; a full POSIX PASS does not qualify that failed reliability check. The quiet repeat passes on unchanged binaries after the namespace run has ended; it does not close the original load-dependent failure. Formal69 cases remain NOT_RUN and ENV lock PREPARING. Required backend parity, remote/POSIX/security matrices, repair/GC, RDMA faults/resources, fair comparison, deployment and long runs remain open.
