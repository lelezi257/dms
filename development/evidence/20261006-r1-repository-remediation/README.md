# R1 historical Python evidence remediation

Date: 2026-10-06.

## Scope

R1 removed only tracked historical Python evidence snapshots matched by:

```text
git ls-files 'development/evidence/**/*.py' \
  ':!development/evidence/20261001-network-preparation/**' \
  ':!development/evidence/20261001-verbs-preparation/**'
```

The two excluded fixture packets are still consumed by
`development/acceptance/test_environment_network.py` and
`development/acceptance/test_environment_verbs.py`; they remain byte-preserved in
the current tree. New untracked E2E scripts under
`development/evidence/20261006-e2e-current/` were not cleaned or modified.

## Result

- Removed from current tree: 384 tracked historical `.py` files, 5,493,299 bytes.
- Net current-tree byte reduction across the R1-owned path set after adding the
  restore maps, tool, tests, report, raw Linux receipts, `.gitattributes` and
  link updates: 5,295,479 bytes.
- Restore map: `historical-python-map.json`, with `commit`, `path`, `git_blob`,
  `sha256`, `bytes` and `mode` for every removed file at
  `e925c5bcf0408851ebfa08a59df29953374da9e9`.
- Restore tool: `development/tools/restore_historical_python.py`. It validates
  the manifest's `commit:path` tree entry, blob id, file mode, SHA-256 and byte
  count before writing; it rejects escaping paths, duplicate paths, symlink
  output roots/parents, existing output paths and non-directory parents.
- Restore tests: `development/tools/test_restore_historical_python.py`.
- Markdown links: 34 relative links in 18 README files now point to fixed
  GitHub blob URLs at the same commit. Raw manifests, checksums, command logs and
  historical receipts were not rewritten.
- Original README map: `original-readme-map.json` records the original
  `commit/path/git_blob/sha256/bytes/mode` for the 18 README files whose
  current-tree links changed. Raw packet validation that needs byte-identical
  historical packets should restore those README files from immutable Git archive
  at `e925c5bcf0408851ebfa08a59df29953374da9e9` or use this map as an overlay;
  do not rewrite packet manifests to match the current tree.
- `.gitattributes`: historical `development/evidence/**` marked generated,
  Markdown documentation marked documentation, `third_party/**` marked vendored.
  Maintained acceptance, tool and script Python outside `development/evidence/**`
  remains normal source.

Duplicate-content inventory of the 384-file set by current SHA-256 found 219
distinct byte identities, 69 duplicate groups, 234 files that are members of a
duplicate group, and a pure-content excess of 165 files / 2,949,222 bytes. This
is a byte-identity grouping over the bounded 384-file set; it does not relabel
the remaining unique historical experiment scripts as duplicates.

Read-only non-Python source-like inventory under `development/evidence/**`
found 82 tracked `.rs`/`.c`/`.h`/`.sh` files totaling 652,923 bytes. A heuristic
scan found one broad source-like snapshot root,
`development/evidence/20261001-authority`, with 51 such files. No non-Python
files were removed in R1.

## Retained Python

[Retained-file inventory](python-inventory.tsv) lists every retained baseline
Python tool/test/fixture and the two new restoration files, with classification,
size and SHA-256. All 34 baseline maintained tools and 23 regression tests remain
unchanged, including suite inventory/runner, standard/LTP/FSx drivers, identity
binding, environment/network/verbs and consistency/permission/lock/restart probes,
Owner/DFS acceptance and performance diagnostics. The 70 Python fixture files in
the two protected historical packets remain unchanged. This task adds one
restoration tool and one focused test file, outside generated evidence paths.
The inventory does not classify new E2E experiment runners as maintained source;
those remain with their own run records, unaffected by this cleanup.

## Verification

- ARM64 Linux restore-tool unit tests
  ([log](logs/restore-tool-tests.log), [exit](logs/restore-tool-tests.exit)):
  `cd /workspace/dms/source && python3 development/tools/test_restore_historical_python.py`
  -> `Ran 14 tests ... OK`.
- ARM64 Linux full restore after deletion
  ([log](logs/restore-full-after.log), [exit](logs/restore-full-after.exit)):
  `python3 development/tools/restore_historical_python.py --manifest development/evidence/20261006-r1-repository-remediation/historical-python-map.json --repo /workspace/dms/source --output-root /tmp/afs-r1-restore-after`
  -> `{"bytes": 5493299, "files": 384, "status": "PASS"}`.
- ARM64 Linux original README packet restoration
  ([log](logs/original-readme-archive.log), [exit](logs/original-readme-archive.exit)):
  `git archive e925c5bcf0408851ebfa08a59df29953374da9e9 -- <18 README paths>`
  restored and verified `18` files / `109452` bytes against
  `original-readme-map.json`.
- ARM64 Linux fixture regression
  ([log](logs/fixture-network-verbs.log), [exit](logs/fixture-network-verbs.exit)):
  `cd /workspace/dms/source/development/acceptance && python3 -m unittest test_environment_network.py test_environment_verbs.py`
  -> `Ran 37 tests ... OK`.
- Link check: 34 fixed blob links; 0 remaining relative Markdown links to removed
  `.py` files.
- Attribute check: evidence README and evidence Python report
  `linguist-generated`; maintained acceptance/tool Python reports unspecified;
  docs Markdown reports `linguist-documentation`; `third_party/fuser/README.md`
  reports both documentation and vendored.
- Static diff check: `git diff --check` -> clean.

Lima prints two harmless login-shell `cd` warnings before Linux commands because
the macOS path is not mounted inside the guest; the commands then run from
`/workspace/dms/source`.
