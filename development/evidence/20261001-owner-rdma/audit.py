#!/usr/bin/env python3
"""Check retained stage receipts and exact source inputs on Linux."""
import hashlib
import json
from pathlib import Path
import platform
import re
import sys

assert platform.system() == "Linux", "audit runs in Linux"
root = Path(sys.argv[1]).resolve(strict=True)
evidence = root / "development/evidence/20261001-owner-rdma"
final = evidence / "owner-linux-r5"
inputs = json.loads((final / "inputs-r5.json").read_text())["files"]
assert len(inputs) == 143
for name, digest in inputs.items():
    assert hashlib.sha256((root / name).read_bytes()).hexdigest() == digest, name
gate = final / "owner-full-r5"
for name in ("fmt", "clippy", "lib", "error", "contracts", "local-api", "fuse-build", "fuse", "build"):
    assert (gate / (name + ".exit")).read_text().strip() == "0", name
for name in ("no-features", "owner-features", "dfs-features", "owner-rdma", "dfs-rdma"):
    assert (final / "owner-features-r5" / (name + ".exit")).read_text().strip() == "0", name
pattern = re.compile(r"test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored;")
def counts(name, last=False):
    rows = [tuple(map(int, row)) for row in pattern.findall((gate / (name + ".log")).read_text())]
    assert rows and all(row[1] == 0 for row in rows), name
    if last:
        rows = rows[-1:]
    return {"passed": sum(row[0] for row in rows), "ignored": sum(row[2] for row in rows)}
summary = {name: counts(name, last=name == "lib") for name in ("lib", "error", "contracts", "local-api", "fuse")}
assert summary["lib"] == {"passed": 403, "ignored": 2}
assert summary["contracts"] == {"passed": 65, "ignored": 7}
actual = json.loads((evidence / "owner-rxe-r5/result.json").read_text())
assert actual["status"] == "PASS" and actual["exit_code"] == 0
binary_sha = (final / "owner-binary-r5/binary.sha256").read_text().split()[0]
assert actual["binary_sha256"] == binary_sha
assert all(actual["owned_peak"][kind] > 0 and not actual["owned_retained"][kind] for kind in ("qp", "cq", "mr", "pd", "ctx"))
completions = re.findall(r"AFS_RDMA_COMPLETE op=(READ|WRITE) bytes=(\d+)", (evidence / "owner-rxe-r5/test.log").read_text())
verbs = {kind: sum(int(size) for op, size in completions if op == kind) for kind in ("READ", "WRITE")}
assert verbs == {"READ": 2 * (4 * 1024 * 1024 + 17), "WRITE": 2 * (4 * 1024 * 1024 + 17)}, verbs
assert (evidence / "owner-original-failure/test.exit").read_text().strip() == "101"
assert (evidence / "prefetch-original/test.exit").read_text().strip() == "101"
assert json.loads((evidence / "owner-rxe-r1/result.json").read_text())["status"] == "FAIL"
native_sha = (evidence / "native-r2/identity.sha256").read_text().splitlines()[1].split()[0]
assert inputs["common/transport/tests/rdma_probe.rs"] == native_sha
assert json.loads((evidence / "native-r2/module/result.json").read_text())["status"] == "PASS"
baseline = json.loads((root / "development/evidence/20261001-file-commit/linux/compile-inputs.json").read_text())["files"]
native_inputs = {name: sha for name, sha in inputs.items() if name.startswith(("common/transport/", "common/error/")) or name in ("Cargo.lock", "Cargo.toml", "rust-toolchain.toml")}
for name, sha in native_inputs.items():
    if name != "common/transport/tests/rdma_probe.rs":
        assert baseline[name] == sha, name
links = re.findall(r"\]\(([^)]+)\)", (evidence / "README.md").read_text())
for link in links:
    assert (evidence / link).is_file(), link
files = {}
for path in sorted(evidence.rglob("*")):
    if not path.is_file() or path.name in ("audit-report.json", "manifest.json"):
        continue
    assert not path.name.startswith("._"), path
    files[str(path.relative_to(evidence))] = {"sha256": hashlib.sha256(path.read_bytes()).hexdigest(), "bytes": path.stat().st_size}
print(json.dumps({"status": "PASS", "level": "STAGE_GATE", "source_inputs": 143,
                  "linux_tests": summary, "actual_owner_rxe_tests": 2,
                  "verbs_bytes": verbs, "binary_sha256": binary_sha,
                  "native_reuse_inputs": native_inputs,
                  "native_reuse_scope": "five tests under their original native-r2 binary/environment identity",
                  "evidence_files": files, "readme_links_checked": len(links),
                  "formal_cases": "69 NOT_RUN", "environment": "PREPARING",
                  "scope": "source gate and small Owner RXE fixture; not cross-VM Node/FUSE, posted-DMA cancellation or formal acceptance"}, indent=2))
