#!/usr/bin/env python3
"""Regression tests for the M1 fio-integrity evaluator."""

from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest import mock


MODULE_PATH = Path(__file__).with_name("evaluate_m1_fio.py")
SPEC = importlib.util.spec_from_file_location("evaluate_m1_fio", MODULE_PATH)
assert SPEC is not None and SPEC.loader is not None
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)

RUNNER_PATH = Path(__file__).with_name("run_m1_fio.py")
RUNNER_SPEC = importlib.util.spec_from_file_location("run_m1_fio", RUNNER_PATH)
assert RUNNER_SPEC is not None and RUNNER_SPEC.loader is not None
RUNNER = importlib.util.module_from_spec(RUNNER_SPEC)
RUNNER_SPEC.loader.exec_module(RUNNER)


def write_workload(directory: Path, *, large_size: int = 512 * 1024 * 1024) -> None:
    fio_file = directory / "fio.json"
    fio_file.write_text('{"jobs":[{"error":0}]}\n', encoding="utf-8")
    checks = []
    for case in MODULE.REQUIRED_CASES:
        item = {"case": case, "status": "passed", "kind": "posix", "size": 4096}
        if case.startswith("fio_"):
            item.update(
                {
                    "kind": "fio",
                    "fio_json": str(fio_file),
                    "local_sha256": "abc",
                    "remote_sha256": "abc",
                }
            )
        if case == "mmap_shared_hash":
            item.update({"kind": "mmap", "local_sha256": "def", "remote_sha256": "def"})
        checks.append(item)
    (directory / "fio-workload.json").write_text(
        json.dumps(
            {
                "schema": "dms.m1.fio-integrity-workload.v1",
                "status": "passed",
                "large_size_bytes": large_size,
                "checks": checks,
                "metrics": {"node_a_filesystem_ops_delta": 10, "node_b_filesystem_ops_delta": 2},
            }
        )
        + "\n",
        encoding="utf-8",
    )


class FioEvaluatorTest(unittest.TestCase):
    def test_runner_arena_has_space_beyond_large_object(self) -> None:
        self.assertEqual(RUNNER.arena_capacity_for_workload("512m"), 1024**3)
        self.assertEqual(RUNNER.arena_capacity_for_workload("1g"), 1280 * 1024**2)

    def test_three_vm_cleanup_removes_current_remote_workspace(self) -> None:
        harness = RUNNER.ThreeVmHarness.__new__(RUNNER.ThreeVmHarness)
        harness.remote = "/tmp/dms-m1-fio-run-1"
        harness.mount = {
            "A": "/tmp/dms-m1-fio-run-1/mnt",
            "B": "/tmp/dms-m1-fio-run-1/mnt",
        }
        harness.shell = mock.Mock()

        harness.cleanup()

        cleanup_calls = [
            call
            for call in harness.shell.call_args_list
            if call.args[1] == "rm -rf -- /tmp/dms-m1-fio-run-1"
        ]
        self.assertEqual(3, len(cleanup_calls))

    def test_passes_complete_evidence(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            directory = Path(raw)
            write_workload(directory)
            result = MODULE.evaluate(directory, minimum_large_bytes=512 * 1024 * 1024)
            self.assertEqual(result["status"], "PASS")

    def test_fails_missing_case(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            directory = Path(raw)
            write_workload(directory)
            data = json.loads((directory / "fio-workload.json").read_text())
            data["checks"] = [item for item in data["checks"] if item["case"] != "punch_hole_zero"]
            (directory / "fio-workload.json").write_text(json.dumps(data) + "\n")
            result = MODULE.evaluate(directory, minimum_large_bytes=512 * 1024 * 1024)
            self.assertEqual(result["status"], "FAIL")
            self.assertTrue(any("punch_hole_zero" in error for error in result["errors"]))

    def test_fails_when_large_case_is_too_small(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            directory = Path(raw)
            write_workload(directory, large_size=64 * 1024 * 1024)
            result = MODULE.evaluate(directory, minimum_large_bytes=512 * 1024 * 1024)
            self.assertEqual(result["status"], "FAIL")
            self.assertTrue(any("too small" in error for error in result["errors"]))

    def test_fails_hash_mismatch(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            directory = Path(raw)
            write_workload(directory)
            data = json.loads((directory / "fio-workload.json").read_text())
            for item in data["checks"]:
                if item["case"] == "fio_seq_4k":
                    item["remote_sha256"] = "mismatch"
                    break
            (directory / "fio-workload.json").write_text(json.dumps(data) + "\n")
            result = MODULE.evaluate(directory, minimum_large_bytes=512 * 1024 * 1024)
            self.assertEqual(result["status"], "FAIL")

    def test_fails_malformed_metric_delta(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            directory = Path(raw)
            write_workload(directory)
            data = json.loads((directory / "fio-workload.json").read_text())
            data["metrics"]["node_a_filesystem_ops_delta"] = "bad"
            (directory / "fio-workload.json").write_text(json.dumps(data) + "\n")
            result = MODULE.evaluate(directory, minimum_large_bytes=512 * 1024 * 1024)
            self.assertEqual(result["status"], "FAIL")
            self.assertTrue(any("expected numeric metric" in error for error in result["errors"]))


if __name__ == "__main__":
    unittest.main()
