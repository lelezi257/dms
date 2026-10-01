#!/usr/bin/env python3
"""Linux-only negative checks for healthy-flow evidence qualification."""
import json
import pathlib
import shutil
import subprocess
import sys
import tempfile

assert sys.platform == "linux"
root = pathlib.Path(__file__).parent


def run(path):
    return subprocess.run([sys.executable, str(path / "audit.py")], capture_output=True, text=True, timeout=30)


baseline = run(root)
assert baseline.returncode == 0, baseline.stderr
mutations = (
    ("a/replicas-after-write.json", "desired_copies", 1),
    ("a/owner-local-write.json", "actual_sha256", "invalid"),
    ("a/workspace.json", "home_serving", False),
)
for filename, key, value in mutations:
    with tempfile.TemporaryDirectory(prefix="afs-round1-audit-") as temp:
        target = pathlib.Path(temp) / "evidence"
        shutil.copytree(root, target, ignore=shutil.ignore_patterns("__pycache__"))
        path = target / filename
        data = json.loads(path.read_text())
        if key == "desired_copies":
            data["chunks"][0]["placement"][key] = value
        elif key == "home_serving":
            data["root"][key] = value
        else:
            data[key] = value
        path.write_text(json.dumps(data))
        assert run(target).returncode != 0, key
for filename, value in (
    ("controller/fixed-regression.exit", "1\n"),
    ("gate/full/gate/lib.exit", "1\n"),
    ("c/rdma-required/metrics.txt", "missing counters\n"),
):
    with tempfile.TemporaryDirectory(prefix="afs-round1-audit-") as temp:
        target = pathlib.Path(temp) / "evidence"
        shutil.copytree(root, target, ignore=shutil.ignore_patterns("__pycache__"))
        (target / filename).write_text(value)
        assert run(target).returncode != 0, filename
print(json.dumps({"status": "PASS", "checks": 7, "scope": "one valid bundle and six invalid bundles; not formal acceptance"}))
