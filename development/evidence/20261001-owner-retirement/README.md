# Retire cleanup debt after authoritative owner-session loss

Date: 2026-10-01. Linux ARM64 source candidate v44. At this source-gate checkpoint, A/B ran the [qualified v43 candidate](../20261001-owner-open/README.md); this source result does not qualify runtime restart faults or release acceptance.

## Results

| Check | Result | Evidence |
| --- | --- | --- |
| Original retry without session retirement | New regression FAIL | [Raw injection log](linux-before-retirement.log) |
| Pending release targeted regressions | 6 PASS in initial session-group implementation | [Initial targeted log](linux-initial-targeted.log) |
| Final implementation | 323 library PASS, two explicit environmental ignores | [Qualified Linux gate](linux-qualified-gate.log) |
| Interface / shared-error / privileged actual FUSE | 57 / 4 / 5 PASS | [Qualified Linux gate](linux-qualified-gate.log) |
| Formatting, strict all-target/all-feature Clippy, feature checks and build | PASS | [Qualified Linux gate](linux-qualified-gate.log) |
| Compile inputs | All143 host/Linux inputs match | [Host](host-source-hashes.json), [Linux](linux-source-hashes.json) |

## Contract

A queued release names the original owner process session, caller process session, inode, lease, admission sequence and opaque handle. Its exact identity remains until confirmed release or authoritative retirement.

The bounded retry pass asks Meta for the owner's current acknowledged session:

- The same session keeps ordinary Release retry.
- A different session or an authoritative absent/expired session retires only the exact old queued entry.
- Timeout, transport failure or another Meta error retains the debt and reports the error. It rotates the entry fairly instead of treating unknown as absence.

The final implementation caches the result once per owner Node within the pass. The initial implementation grouped by owner Node/session; the final mixed old/live-session regression requires only one query for that Node. Error results are cached too. Release itself is attempted at most once per selected handle. The existing64-entry cap,250ms maintenance /2s drain aggregate budgets and100ms per-call cap remain unchanged. Each network call receives the remaining budget.

Removing an entry requires both its unchanged handle and owner Arc identity, so a concurrent replacement is not removed. Queued entries consume capacity through entries.len(); live reservations remain separate and are not decremented during retirement. Existing same-session retry regressions explicitly register the mock session instead of accidentally taking the absent-session branch.

## Regression coverage

The four new regressions cover replaced/absent owner sessions, unknown session retention, mixed old/live sessions on one Node, and65 retired handles processed across two fair passes with one lookup per pass. Existing tests retain same-session release success/failure, exact cleanup identity, fair rotation and admission bounds. Restoring the old retry branch makes the retirement regression call a remote owner that must not be contacted and fail deterministically.

No wire field, RPC method, module boundary, file name or dependency changes. Meta session lookup is a maintenance-only request; ordinary write/read RPC counts are unchanged.

## Identity and limits

Node SHA256: eb1540ba7b95b9b19aade368d2e93227aba5427581f832a888c10af548a9c865.
Meta SHA256: 8f879feb0c8d58d59a41fa2aa9df06d071d7ac3078b19fcfe60196181d3bf351.
Immutable Linux artifacts: /home/lzc.guest/afs-build/artifacts/v44-qualified.

The binaries were not deployed at this source-gate checkpoint. The later [owner restart investigation](../20261001-owner-recovery/README.md) preserves the v44 runtime failure and repaired v45 proof. Actual complete restart/network faults, current full-suite completion, overall writeback/shutdown budgeting and durable-backend parity remain unqualified. Formal69 cases remain NOT_RUN; ENV lock remains PREPARING. Handoff documentation is unchanged.
