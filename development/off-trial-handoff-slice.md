# Existing OFF candidate package reproduction and colleague handoff

Independent G2.27 delivery branch, after the current6d installation/recovery
PASS. Do not reopenG1 or claim complete performance/ON qualification.

1. Pin existing source6d51aeb/map66dbbe3e, Meta/Node hashes and original package
   ee25d589…; compare all11 non-generated archive inputs with Git6d bytes.
2. On existing ARM64 Linux afs-g2-micro only, admit complete packaging deps,
   ≥1GiB free guest ext4 and a fresh owned staging root. Source files are
   retrieved from fixed Git6d; no Rust rebuild or private/vendor edits.
3. Run existing build-package.sh once with the already-installed exact ELFs,
   explicit source/version/features/SOURCE_DATE_EPOCH0 and restrictiveumask077.
   Compare new full archive bytes/SHA with the already-run original package,
   and require package16-file hashes/manifest exact. Retain mismatch asFAIL,
   stop affected branch on an actual environment blocker; do not tune packaging
   conditions to force equality or redo passed install/standard/performance.
4. Prepare a versioned colleague checklist with executable install/selfcheck/
   normalstop commands and the same payload identity. Publish code/doc/evidence
   via Lore and normal Gitpush. A separately downloadable trial asset must be
   explicitly prerelease/defaultOFF and preserve performance/ON/R2 gaps.

The package payload/ELFs/full staging source remain outsideGit. Git evidence is
commands/results, input hashes/provenance and index; no new source snapshots.
