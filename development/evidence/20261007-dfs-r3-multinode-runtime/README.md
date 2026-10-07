# DFS multi-node attempt after verified historical-data release

2026-10-07, G2.24 bounded continuation on main4e51dca5. Product7e6e00a6,
157-input map151a2c6d, Meta76a1e34c/Nodec47be268/packagec3bb5a30/probe8ac3cced
unchanged. No Rust/vendor edits, rebuild or inherited full-suite acceptance.
G1 historical8/8 remains closed; G2.24 remains unaccepted.

**PASS — authorized capacity remediation.** The user selected verification of
host archives followed by release of recoverable historical test data on A.
Only the already normally stopped `dfs-r3-write-7e6-20261007-r1` A directory
was released. Its prior raw evidence archive omitted chunks, so a separate
full-root archive was created first:43,838,172B, SHA607a6110,229 entries,
500,139,034 regular-file bytes. Linux afs-build extraction matched every
file SHA, type, permission mode, uid/gid, mtime, xattr and symlink target.
Original inode identity is historical and cannot be recreated by extraction.
No process/FD/map reference or mount used the directory; source manifest was
rechecked immediately before release. Removed500,518,912 allocated bytes
(477.332MiB); A backing free increased1,894,559,744→2,395,078,656B.
A's8 protected processes and full mount inventory stayed exact. Only created
archive/restore temporary directories were then cleaned. No blanket cleanup,
VM resize, protected-service stop, predicate relaxation or Git-history change.
[Release/restore](capacity-release.json), [recoverable VM→host mapping](recovery-map.json).
The full archive contains private TLS/configuration and remains0600 outside Git.
The old host archive, failed records and old evidence links remain in place.

**PASS — fresh four-role admission.** All existing config/ELF/tool/mount/dependency
checks and budgets passed before any product service started. Original
used≤1GiB/free≥1GiB/free+case-used≥2GiB limits stayed unchanged, as did the new
128MiB ctl/600MiB node/2GiB aggregate plan. The earlier148.492MiB refusal remains
an immutable historical preparation result, with a separate new [readmission](readmission.json).
The unchanged exact r8 tool's9 Linux guards and unchanged older guards are
reused with their existing scope/identity; no repeated suite or score run.

**FAIL — first warmup write cohort.** All three workers reached READY and START.
A/C C probes returned rc2/null result, and their logs report `DFS metadata
condition changed during create`; B's C probe returned rc0/content_ok but its
worker later reported ENOTCONN during postcheck. The coordinator rejected a
failed C_DONE. Host relay cleanup raised BrokenPipeError while closing stdin,
masking the original failure in the outer receipt. Final relay rc/events files
and C-probe stderr were not preserved by that old path; these gaps are explicit,
not reconstructed or declared successful. Zero completed warmup cohorts,
zero measured cohorts, zero read cohorts; B alone is not an accepted performance
or whole-cohort result. No retry, timing percentile, concurrency or3FS parity claim.
[Exact failures and logs](runtime-failure.json), [command/hash index](command-index.json).

**PASS — normal product closure.** All4 bound actual waits are0, all8 owned
product/supervisor PIDs are gone, all11 protected identities and complete
initial mount inventories are exact. A subsequent check found no owned probe
or product process. Final case allocation591,925,248B is below2GiB and all role
caps/reserves still hold. These are closure facts, not a passed data case.
[Closure](closure-summary.json). Full logs, selected durable metadata and
all raw stdout/stderr are in the separate restricted [runtime archive](raw-archive.json);
the failed fixture remains on the VMs, while the old released A root is fully
recoverable from the additional archive. Original preparation archive is intact.

[Linux record audit](linux-record-audit.json) verified the original22 packet/fact inputs, exact release arithmetic, restored archive identities, unchanged budgets, failed cohort and normal closure. First verifier receipt-schema KeyError and corrected second PASS are both retained in the separate [audit archive/index](audit-index.json); no product rerun or environment repair. The audit does not validate itself or establish a data PASS.

**Inference and next small items.** Create reads the complete parent record and
conditions its single transaction on equality, then changes the parent's revision
(src/meta/dfs.rs:1034–1119,4267–4285). A condition failure maps to META_DFS_CONFLICT/
EBUSY and reaches FUSE without a create retry (:4092;src/error.rs:112;
src/node/fuse.rs:1517). Same-parent contention fits these observations, but the
failed condition was not logged, so root cause is not proven. Existing sequential
replay tests do not cover that concurrency. [Next item boundaries](next-item.json)
separate deterministic create-conflict recovery from test-relay failure capture
and drainage. Do not serialize the benchmark, blindly retry every conflict or
rerun this frozen attempt to conceal the failure. B's ENOTCONN may be teardown
racing postcheck; no product panic was captured.

Ordinary OwnerFs1.2×MooseFS throughput/0.8×independent latency, bind≥0.90×ext4,
DFS matched3FS and deletion-report targets remain unchanged. G2 counts stay
11 limited outputs/1 bind function in progress/15 unaccepted (G2.24 now has this
failed attempt). Complete project Goal remains ACTIVE.
