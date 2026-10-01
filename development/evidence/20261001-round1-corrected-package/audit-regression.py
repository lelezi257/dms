#!/usr/bin/env python3
"""Negative retained-evidence checks; all execution is Linux-only."""
import importlib.util
import json
import pathlib
import platform
import shutil
import tempfile

assert platform.system() == "Linux"
ROOT = pathlib.Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("round1_audit", ROOT / "audit.py")
audit = importlib.util.module_from_spec(spec)
spec.loader.exec_module(audit)
audit.evaluate(ROOT)
results = [{"name": "retained valid packet", "ok": True}]

def change(path, function):
    value = json.loads(path.read_text())
    function(value)
    path.write_text(json.dumps(value))

cases = [
    ("wrong archive", "r1/a/evidence/install.json", lambda v: v.update(package_sha256="0" * 64)),
    ("wrong R1 policy", "r1/a/evidence/r1-replicas.json", lambda v: v["placement"].update(desired_copies=2)),
    ("bad data", "async/b/evidence/dfs-cold-read.json", lambda v: v.update(actual_sha256="0" * 64)),
    ("no async debt", "async/a/evidence/replication-before-join.json", lambda v: v.update(available_copies=2)),
    ("missing completion", "async/a/evidence/repair-completed.json", lambda v: v[0].update(tasks=[])),
    ("false suite pass", "consistency/report.json", lambda v: v.update(status="FAIL")),
    ("wrong central Meta", "consistency/report.json", lambda v: v["identity"]["meta_after_suite"]["identity"]["expected_processes"]["meta"].update(sha256="0" * 64)),
]
for name, filename, mutation in cases:
    with tempfile.TemporaryDirectory() as temp:
        copied = pathlib.Path(temp) / "evidence"
        shutil.copytree(ROOT, copied)
        change(copied / filename, mutation)
        try:
            audit.evaluate(copied)
        except AssertionError as error:
            results.append({"name": name, "ok": True, "rejected": str(error)})
        else:
            raise AssertionError("invalid evidence accepted: " + name)
print(json.dumps({"status": "PASS", "results": results}, indent=2))
