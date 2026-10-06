# Existing default-OFF package reproduction and trial handoff

2026-10-07, existing ARM64 Linux `afs-g2-micro`, guest ext4. No Rust/vendor/runtime
behavior changes, no install/standard/performance rerun; G1 remains8/8 and G2.27
complete performance gate stays open. Source6d51aeb/map66dbbe3e/157 inputs and
existing Meta/Node ELFs are unchanged.

**Reproduction PASS:** run the existing Linux package builder once using exact
Git6d13 packaging inputs, already-installed fixed ELFs, explicit version/features,
SOURCE_DATE_EPOCH0 and umask077. New full archive and the original already tested
archive are byte-identical (`cmp` exit0) and both SHA256
`ee25d5892c4e884e67c86d4e5b9c6ab551af4c46649edab6a05da06659d1aff9`.
Package size14,728,435B. This is archive reproduction from existing Linux ELFs,
not a new reproducible Rust compiler/linker build claim.

[Result](result.json), [actual complete preflight](preflight.json),
[exact commands and raw outputs](commands.json), [fixed Git source input map](source-inputs.json),
[11 archive inputs exactly match Git6d](archive-input-match.json).
The original payload has [16 file hashes](../20261007-installed-off-6d/package-inventory.json)
and [fresh compiler-free install/35-check recovery PASS](../20261007-installed-off-6d/README.md).
Use those unchanged runtime results; no additional product test is claimed here.
Original package, reproduced package and full13 source staging files remain outside
Git. Metadata/Git provenance permits restoration without a new source snapshot.

The [versioned colleague checklist](../../../docs/guides/trial-6d.md) provides
fixed hashes, dependencies, fresh install/64MiB checks/center restart/normalstop,
verified scope and remaining issues. It preserves the archive's older25a8061
embedded-guide statement under its original identity and supplies current6d
receipts separately. A prerelease asset is anchored at the actual Rust source6d;
it does not replace stableLatest or claim complete G2.27/ON/POSIX/performance.
Publication metadata/digests are added after remote verification.

Mixed append offsets/locks/watch remain FAIL, native OFF, ordinary performance
FAIL/data remain, R2 official fuser API migration is still independently blocked.
Next independent performance item is Owner remote read/delete small data;
[read-only environment admission](../20261007-owner-remote-admission/README.md)
will distinguish legacy helper constraints from the new fixture contract.
