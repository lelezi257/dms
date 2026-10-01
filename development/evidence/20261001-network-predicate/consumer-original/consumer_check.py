#!/usr/bin/env python3
"""Linux-only preparation/runner integration; never product acceptance proof."""
import json
import platform
import shutil
import sys
from pathlib import Path

base = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(base / "acceptance"))
import environment
import runner
import test_runner

assert sys.platform.startswith("linux") and platform.machine() == "aarch64"
lock_path = base / "preparing.lock.json"
lock = environment.load_json(lock_path)
bundle = environment.load_json(base / "bundle/bundle.json")
report = environment.evaluate_environment(lock, bundle, base / "bundle")
assert report["status"] == "BLOCKED", report
assert report["summary"] == {"pass": 36, "blocked": 10, "fail": 0}, report["summary"]
network = next(c for c in report["checks"] if c["name"] == "network-tls-fault-recovery")
assert network["status"] == "PASS", network
errors = environment.qualification_errors(lock, lock_path)
assert len(errors) == 10 and not any("network-tls-fault-recovery" in e for e in errors), errors

# A deliberately synthetic READY/FROZEN dispatcher fixture isolates the real
# environment consumer. These identities belong to the test harness only.
helper = test_runner.RunnerTests()
temp, root, manifest, _, cases_path, fixture_lock_path, results = helper.make_workspace()
with temp:
    shutil.copytree(base / "bundle", root / "bundle")
    shutil.copyfile(base / "bundle/acceptance.md", root / "acceptance.md")
    marker = root / "driver-executed"
    driver = root / "driver.py"
    driver.write_text(f"from pathlib import Path\nPath({str(marker)!r}).write_text('unsafe dispatch')\nprint('{{}}')\n")
    manifest["cases"][0]["driver"] = {"state": "READY", "command": [sys.executable, str(driver)]}
    test_runner.write_json(cases_path, manifest)
    attestation = helper.write_attestation_and_lock(root, cases_path, fixture_lock_path)
    fixture_lock = runner.load_json(fixture_lock_path)
    fixture_lock.update({"host": lock["host"], "image": lock["image"],
                         "environment_evidence": lock["environment_evidence"]})
    test_runner.write_json(fixture_lock_path, fixture_lock)
    result = runner.run_acceptance(helper.args(cases_path, fixture_lock_path, results,
        backend="DFS", meta="etcd", transport="TCP", profile="full",
        identity_attestation=str(attestation), contract=str(root / "acceptance.md")))
    assert result["summary"]["status"] == "BLOCKED", result["summary"]
    assert result["summary"]["full_release_gate_pass"] is False
    assert not marker.exists(), "environment guard dispatched a synthetic driver"
    assert "environment" in result["results"][0]["reason"], result["results"]
    output = {"status": "PASS", "scope": "preparation and synthetic dispatcher consumer only",
              "preparation": report["summary"], "qualification_errors": errors,
              "driver_executed": marker.exists(), "runner": result}
    (base / "linux/consumer.json").write_text(json.dumps(output, indent=2, sort_keys=True) + "\n")
print("PASS: network preparation accepted; ten remaining prerequisites block full dispatch")
