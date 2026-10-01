# Acceptance data-volume reserve

## Result and scope

**Local preparation regression PASS:** eight inactive package-extraction trees
were archived from A's 32 GiB data ext4 to its existing root ext4. Available data
space increased from **1,089,331,200 to 5,280,145,408 bytes** (about 1.01 to
4.92 GiB), satisfying the 4 GiB reserve at this observation. Disk sizes and tested
data paths did not change. [Migration result](linux/apply/result.json) records
all content and attribute manifests; [fresh verification](linux/post-verification.json)
checks original-path links and archive content.

Nine [Linux script regressions](linux/strengthened-tests.log) pass.
[Seven original regressions](linux/tests.log) precede the actual migration.
These are environment preparation results, not a Rust stage gate or ENV-01
acceptance. [Summary](result.json) retains **69 NOT_RUN**, **PREPARING** and an
active delivery goal. The [acceptance contract](../../../docs/acceptance.md)
and handoff are unchanged.

## Preserved inputs and runtime

The whitelist contains only historical extracted packages. Installed `opt`,
state, logs, actual mounts, comparator datasets and v62 corrupt/RXE fault
fixtures remain on their original data volume. The original extraction paths
are links to exact archived trees under
`/var/lib/afs-acceptance/archives/20261001-extractions-v65` in the same Linux guest.
These paths are packaging fixtures, not tested data storage.

The manifests check SHA-256, lengths, modes, UID/GID, nanosecond mtime, xattrs
and symlink targets. Special files, hardlinks, active process references and
mounts are rejected. Existing destinations are never overwritten. Copies and
sources are rechecked before replacing an original path; only the redundant
verified source copy is removed.

[Before](linux/protected-before.json) and [after](linux/protected-after.json)
observations match for 31 protected fault-data/baseline-binary entries, ten live
data-volume executable identities and mountinfo. The post-check inspects every
live process mount namespace for mounts in either original or archive paths.
Existing disconnected FUSE fixtures remain untouched; A inventory retains their
`df` error instead of reporting an overall clean filesystem probe.

The actual migration used [this executed script](linux/executed-archive-extractions-v65.py).
A later [static review](static-review.md) requested stronger all-process mount
namespace checks, pre-mutation capacity estimates and destination-filesystem
checks. The [strengthened helper](linux/archive-extractions-v65.py) and
[tests](linux/test_archive_extractions_v65.py) include those protections. It was
regressed on Linux; the already completed 4 GiB migration was not repeated.
Its fresh manifest/mount audit passed. Original executed inputs and results
retain their own hashes.

## Four-guest observation

After confirming no cargo/rustc/clang build was active, the separate build VM
was stopped and C started. [Lima before](lima-before.jsonl) and
[after](lima-after.jsonl) retain the observations. Four acceptance guests run
with ctl 2 CPU/4 GiB and A/B/C 2 CPU/6 GiB, totaling eight vCPUs and 22 GiB;
other VMs remain stopped. All four report kernel `6.8.0-142-generic`, no swap
entries and their intended dedicated-network addresses.

- [A inventory](inventory-a.json): data reserve restored; historical disconnected
  FUSE mounts produce an overall `df` UNKNOWN with valid data-ext4 rows.
- [B inventory](inventory-b.json): data ext4 reserve observed.
- [C initial inventory](inventory-c.json): restart had removed the transient RXE
  device. The unchanged [configuration helper](configure-rxe.sh) was syntax
  checked and run in Linux; [actual output](c-rxe-configure.log),
  [stderr](c-rxe-configure.stderr) and [exit](c-rxe-configure.exit) are preserved.
  [C after RXE](inventory-c-rxe.json) observes rxe0/eth0 ACTIVE. Device presence
  does not prove cross-VM verbs payload success or product RDMA acceptance.
- [ctl inventory](inventory-ctl.json): state ext4 reserve and historical LTP loop
  mounts remain recorded.

[Host capacity before](host-df-before.log), [after archive](host-df-after.log) and
[after topology change](host-df-topology.log) remain above the 40 GiB ongoing
reserve at these observations. They do not retrospectively prove the 100 GiB
initial preparation threshold, complete resource isolation, physical durability
or future capacity for every workload.

## Validation reuse and remaining qualification

All 143 [v64 compilation inputs](../20261001-file-commit/linux/compile-inputs.json)
remain byte-identical. Its [complete Linux source gate](../20261001-file-commit/README.md)
is reused under the same input identity; no product Rust/dependency/RPC format
changed. This batch ran script checks and actual migration/configuration
verification only. Full POSIX, 8 GiB, performance matrices and soak remain in
their planned stages.

ENV-01 still requires consolidated live network/TLS/backend/verbs probes,
reference applicability/accounting, actual comparator mount I/O and complete
frozen identities. Comparator mount I/O readiness and equivalent durable
performance qualification are separate predicates in acceptance §3.4 and §5.
A successful mount cannot qualify performance; missing fair durability does not
invalidate an independently proven mount subcheck. Both qualification records
must stay explicit. Older preflight reports retain their original observations.

The runner currently verifies FROZEN/PASS and selected identities; it does not
yet enforce every mandatory environment predicate, and its full dispatch cannot
bootstrap a PREPARING lock. A preparation verifier and frozen live-verification
path remain required. Neither filling lock fields nor this reserve repair
justifies promoting a TODO driver or setting the lock to FROZEN/PASS.

## Restore an extraction

Restoration is an explicit maintenance operation on an inactive fixture. Use
`linux/apply/result.json` for the selected source/archive paths and exact
manifest. Check all process references and mount namespaces first. Copy the
archive with Linux `cp -a` into a new sibling of the original-path link; compare
its complete manifest with the saved one. Rename the link to a backup, rename
that verified directory into the original path, then recheck. If installation
fails, restore the link. Keep the archive until restored bytes are verified.

Full restoration consumes the recovered data space and reinstates the original
reserve shortage. Re-observe capacity before resuming any acceptance lane.
This helper refuses already migrated paths; rerunning `--apply` is not a restore
operation.
