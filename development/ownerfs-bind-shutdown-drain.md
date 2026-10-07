# OwnerFs workspace shutdown ownership slice

2026-10-07, source main59f8753. G1 historical8/8/defaultOFF and existing results unchanged.

Current defect: Services owns the workspace join; its ten-second abort can allow Node to proceed to FUSE teardown while a blocking worker still owns an export. Driver stops after one cleanup error, dropping unresolved claims.

Narrow change: Node keeps the worker owner, Services only observes its completion and requests stop. Node joins the worker after services stop and before any FUSE teardown. The same mount thread retries raw EBUSY normal cleanup on its retained Active. Terminal errors keep the Driver/claim alive, notify the service monitor and log the original error; existing process shutdown watchdog fails124 rather than allowing an unproved teardown. Do not retry ESTALE/unknown runtime/permission errors or extend global deadlines. No new dependencies, vendor changes, host switch or runtime-wide refactor.

Regressions first: real privileged base bind held by child cwd during shutdown, released after first EBUSY, then normal cleanup/socket/lock exit; Node registration owner remains held after service cancellation until explicit join. Retain old-code FAIL before repair. Also cover terminal error retention in an isolated child with the existing short watchdog, not a hanging test runner. Linux root namespace only for physical cases, ordinary user for compilation. Affected controller/Node/config tests, fmt, Owner/DFS-only compilation and Clippy/build as warranted. Save exact compiler maps/test ELF, argv/exits, negative logs; no script/source snapshots. Runtime official-runc/whole-Node evidence is separate unless actually selected and executed. Main direct Lore commit/push, no feature approval gate.

## Additional failure branch

Readonly source verification found that fatal listener accept followed by EBUSY could wait before the service monitor was notified. Publish the original error before the same-thread cleanup wait; a selected root regression uses actual busy cwd/FD references and the real Services shutdown callback to arm the existing short test watchdog. The callback must occur before release. No dependency/runtime deadline changes.

## Evidence scope

Final selected tests comprise 11 ordinary controller/Node cases (including a real 10-second Services abort), 7 config cases and 8 privileged adapter cases. Four newly added cases are worker ownership, transient busy closure, terminal retained claim/exit124, and listener notification before busy drain. Controlled runtime and worker fixtures are explicitly separate from official-runc/whole-Node E2E. The terminal child records retained mount/control identity before its short watchdog exit; this is not a production failure/reconciliation matrix. Public binary entry has the existing15s watchdog; embedded node::run does not acquire that process deadline automatically.

Original busy closure FAIL and initial test payload-type compile failure remain evidence. Intermediate r2 passed selected checks but was superseded by the listener notification correction; only final r3 source is current. Final package and actual Node/runc active-container shutdown must be verified in a separate next small case before upgrading runtime qualification. Existing standards/timings retain their earlier versions and are not rerun.

## Bounded closure

Final r3 nine source gates and26 selected cases PASS, exact source/ELF/command identities and all failures retained in [the packet](evidence/20261007-ownerfs-bind-shutdown-drain/README.md). New release ELFs have not been packaged or deployed. Next is the current official-runc/Node active-workspace shutdown case, not a standards or performance rerun.

## Subsequent runtime subcase

The identified release ELFs were then packaged and the actual official-runc active-workspace Node stop passed [in this separate versioned packet](evidence/20261007-ownerfs-bind-active-node-stop/README.md). This does not extend the source regression scope above or close general drain/full ON.
