# Owner standard evidence: limited historical reuse

2026-10-07. This is an evidence audit, **not a new standard run or acceptance PASS**. G1 8/8 and G2 counts unchanged. Installed product6d51aeb remains distinct from unvalidated orderly-recovery draft2a55b02.

| Classification | Exact version / scope / evidence |
| --- | --- |
| Historical actual PASS; limited reuse for delivered6d OFF | e925 release/mapb850fcf4: pinned pjdfstest d25636a,236 files/8819 checks/28 upstream TODO,0skip/unexpected failure. [Original release proof](../20261006-e2e-current/release-r1/runtime/results/owner-pjdfstest-release-r1/artifacts/std-01-pjdfstest/proof.json) |
| Historical dev actual PASS; limited reuse | e925 dev: fixed LTP6 of657,6PASS/651 unselected, no full LTP claim. [Original proof](../20261006-e2e-current/r3/owner-ltp-r1/artifacts/std-02-ltp/proof.json) |
| Historical dev actual PASS; limited smoke reuse | e925 dev: secfs edf5eb4a,seed1/-N1000/default262144; raw completion reports1073 operations. No new long/full/three-seed qualification. [Original proof](../20261006-e2e-current/r2/owner-fsx-r1/artifacts/std-03-fsx/proof.json) |
| Current installed identity only | Linux6.8.0-106/aarch64,6d Meta2c7b7d08/Node2cf1f538, suite Git pins/HEAD tracked diffs and pjdf/FSx binary hashes; fixed LTP install-root8 hashes match prior audit. [Read-only receipt](current-installed-suite-readonly.txt), [correct LTP path](current-ltp-install-readonly.txt) |
| Needs distinct future qualification | ON/remote/other backend/malformed native TOML, broad reliability/performance, unvalidated repair draft |

[Independent code-impact audit](code-impact-review.json) verifies e925154→6d157 fixed inputs:144 identical/10 modified/3 added, zero fixed-Git mismatch. **Owner paths are not all byte unchanged:** constructors now delegate with native eligibility false; cache construction is equivalent; LocalFs statvfs body is unchanged after receiver-to-parameter extraction. Ordinary Owner Backend methods/locks/FUSE/Meta remain byte identical; changed-path cache/admission and4 statfs guards have existing Linux PASS. Shared DFS capacity and startup changes are classified separately. No new full suite is required solely for these proven equivalent refactors; conditions and regression stop rules are explicit in the report.

[Reference index](reference-index.json) binds existing proofs by path/SHA/bytes without copying them. First metadata check used LTP source-tree paths rather than the original suite identity.install_root: exit1 preserved, corrected targeted read follows the exact old identity. No suite installation, build, service start or environment repair.

The afs-build85GiB ENOSPC blocker remains; this audit does not remove it. Main f09185e trial remains usable with original scope. After the user selects environment handling, return to final release source gates/new package and one4KiB orderly recovery; do not replay these standards to fill time.
