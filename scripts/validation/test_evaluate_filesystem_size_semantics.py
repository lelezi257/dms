from __future__ import annotations

import argparse
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPT_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(SCRIPT_DIR))

import evaluate_filesystem_size_semantics as evaluator
import run_filesystem_size_semantics_3vm as three_vm_runner


def passing_workload() -> dict:
    checks = [{"operation": operation, "remote_stat": {"size": 1}} for operation in evaluator.REQUIRED_OPERATIONS]
    for check in checks:
        if check["operation"] in {"truncate_grow_sparse", "pwrite_beyond_eof_sparse"}:
            check["arena_logical_bytes_delta"] = 4
            check["max_expected_allocation_delta"] = 4096
    for check in checks:
        if check["operation"] == "concurrent_o_append":
            check["record_count"] = 16
    return {
        "schema": "dms.filesystem.size-semantics-workload.v1",
        "passed_operations": len(evaluator.REQUIRED_OPERATIONS),
        "checks": checks,
        "recovery_path": "/visibility.txt",
        "recovery_expected": "dms-size:cross-node",
    }


class EvaluateFilesystemSizeSemanticsTests(unittest.TestCase):
    def test_three_vm_run_id_rejects_remote_path_injection(self) -> None:
        self.assertEqual(three_vm_runner.validated_run_id("size-final_01"), "size-final_01")
        for invalid in ("../escape", "with/slash", "$(touch-pwned)", "", "a" * 65):
            with self.assertRaises(argparse.ArgumentTypeError):
                three_vm_runner.validated_run_id(invalid)

    def write_result_dir(self, root: Path, workload: dict) -> None:
        (root / "size-workload.json").write_text(json.dumps(workload), encoding="utf-8")
        (root / "size-recovery.json").write_text(
            json.dumps(
                {
                    "schema": "dms.filesystem.size-semantics-recovery.v1",
                    "path": "/visibility.txt",
                    "bytes": len("dms-size:cross-node".encode("ascii")),
                }
            ),
            encoding="utf-8",
        )
        (root / "node-a.prom").write_text("dms_node_filesystem_operations_total 1\n", encoding="utf-8")
        (root / "node-b.prom").write_text("dms_node_filesystem_operations_total 1\n", encoding="utf-8")
        (root / "meta.prom").write_text("dms_meta_operations_total 1\n", encoding="utf-8")

    def test_evaluator_accepts_complete_size_semantics_result(self) -> None:
        contract = {"schema": "dms.filesystem.size-semantics-contract.v1", "required_operation_count": 6}
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.write_result_dir(root, passing_workload())
            report = evaluator.evaluate(contract, root)
            self.assertEqual(report["status"], "PASS")

    def test_evaluator_rejects_missing_sparse_allocation_evidence(self) -> None:
        contract = {"schema": "dms.filesystem.size-semantics-contract.v1", "required_operation_count": 6}
        candidate = passing_workload()
        for check in candidate["checks"]:
            if check["operation"] == "truncate_grow_sparse":
                check["arena_logical_bytes_delta"] = None
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.write_result_dir(root, candidate)
            report = evaluator.evaluate(contract, root)
            self.assertEqual(report["status"], "FAIL")
            self.assertTrue(any("missing Arena logical bytes delta" in error for error in report["errors"]))

    def test_evaluator_rejects_sparse_allocation_by_hole_length(self) -> None:
        contract = {"schema": "dms.filesystem.size-semantics-contract.v1", "required_operation_count": 6}
        candidate = passing_workload()
        for check in candidate["checks"]:
            if check["operation"] == "pwrite_beyond_eof_sparse":
                check["arena_logical_bytes_delta"] = 1024 * 1024
                check["max_expected_allocation_delta"] = 4096
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.write_result_dir(root, candidate)
            report = evaluator.evaluate(contract, root)
            self.assertEqual(report["status"], "FAIL")
            self.assertTrue(any("sparse operation allocated" in error for error in report["errors"]))

    def test_cli_creates_output_parent_even_when_evaluation_fails(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            contract = root / "contract.json"
            contract.write_text(json.dumps({"schema": "dms.filesystem.size-semantics-contract.v1"}), encoding="utf-8")
            output = root / "nested" / "evaluation.json"
            completed = subprocess.run(
                [
                    sys.executable,
                    str(SCRIPT_DIR / "evaluate_filesystem_size_semantics.py"),
                    str(root / "missing-result-dir"),
                    "--contract",
                    str(contract),
                    "--output",
                    str(output),
                ],
                text=True,
                capture_output=True,
                check=False,
            )
            self.assertEqual(completed.returncode, 1)
            self.assertTrue(output.is_file())
            self.assertEqual(json.loads(output.read_text(encoding="utf-8"))["status"], "FAIL")


if __name__ == "__main__":
    unittest.main()
