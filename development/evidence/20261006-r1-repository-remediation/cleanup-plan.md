# R1 cleanup plan

1. Preserve `development/evidence/20261001-network-preparation` and `20261001-verbs-preparation` because current acceptance tests copy them as fixtures.
2. Generate a 384-entry immutable map from commit `e925c5bcf0408851ebfa08a59df29953374da9e9` with path, Git blob, SHA-256, bytes and mode.
3. Add a restore tool that validates path containment, duplicate paths, blob id, SHA-256, byte count and mode before writing.
4. Add focused unit tests for positive restore and escape/duplicate/blob/SHA/mode/missing failures.
5. Run the new Python tests on ARM64 Linux before deleting current-tree historical `.py` files.
6. Replace Markdown references to removed scripts with fixed GitHub blob links while leaving raw manifests and receipts unchanged.
7. Remove only the mapped 384 tracked historical `.py` files from the current tree and verify full Linux restore into a fresh root.
8. Update only `.gitattributes`, R1 remediation status/results and the independent R1 evidence report.
