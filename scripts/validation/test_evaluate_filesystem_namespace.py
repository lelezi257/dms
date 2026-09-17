from __future__ import annotations

import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPT_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(SCRIPT_DIR))

import evaluate_filesystem_namespace as evaluator


class EvaluateFilesystemNamespaceTests(unittest.TestCase):
    def test_evaluator_requires_all_rounds_operations_recovery_and_metrics(self) -> None:
        contract = {"schema": "dms.filesystem.namespace-contract.v1", "minimum_rounds": 1}
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "namespace-workload.json").write_text(
                json.dumps(
                    {
                        "schema": "dms.filesystem.namespace-workload.v1",
                        "passed_rounds": 1,
                        "recovery_round": 0,
                        "paged_directory": {
                            "operation": "paged_readdir",
                            "entry_count": 300,
                            "wait_attempts": 1,
                        },
                        "round_results": [
                            {
                                "round": 0,
                                "checks": [{"operation": operation} for operation in evaluator.REQUIRED_OPERATIONS],
                            }
                        ],
                    }
                ),
                encoding="utf-8",
            )
            (root / "namespace-recovery.json").write_text(
                json.dumps({"schema": "dms.filesystem.namespace-recovery.v1", "round": 0}),
                encoding="utf-8",
            )
            (root / "node-a.prom").write_text("dms_node_filesystem_operations_total 1\n", encoding="utf-8")
            (root / "node-b.prom").write_text("dms_node_filesystem_operations_total 1\n", encoding="utf-8")
            (root / "meta.prom").write_text("dms_meta_operations_total 1\n", encoding="utf-8")
            report = evaluator.evaluate(contract, root)
            self.assertEqual(report["status"], "PASS")

    def test_evaluator_rejects_eventual_namespace_visibility(self) -> None:
        contract = {"schema": "dms.filesystem.namespace-contract.v1", "minimum_rounds": 1}
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            checks = [{"operation": operation} for operation in evaluator.REQUIRED_OPERATIONS]
            checks[0]["wait_attempts"] = 2
            (root / "namespace-workload.json").write_text(
                json.dumps(
                    {
                        "schema": "dms.filesystem.namespace-workload.v1",
                        "passed_rounds": 1,
                        "recovery_round": 0,
                        "paged_directory": {
                            "operation": "paged_readdir",
                            "entry_count": 300,
                            "wait_attempts": 1,
                        },
                        "round_results": [{"round": 0, "checks": checks}],
                    }
                ),
                encoding="utf-8",
            )
            (root / "namespace-recovery.json").write_text(
                json.dumps({"schema": "dms.filesystem.namespace-recovery.v1", "round": 0}),
                encoding="utf-8",
            )
            for filename, metric in (
                ("node-a.prom", "dms_node_filesystem_operations_total"),
                ("node-b.prom", "dms_node_filesystem_operations_total"),
                ("meta.prom", "dms_meta_operations_total"),
            ):
                (root / filename).write_text(f"{metric} 1\n", encoding="utf-8")

            report = evaluator.evaluate(contract, root)
            self.assertEqual(report["status"], "FAIL")
            self.assertTrue(any("not visible on first check" in error for error in report["errors"]))

    def test_evaluator_fails_when_round_check_is_missing(self) -> None:
        contract = {"schema": "dms.filesystem.namespace-contract.v1", "minimum_rounds": 1}
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "namespace-workload.json").write_text(
                json.dumps(
                    {
                        "schema": "dms.filesystem.namespace-workload.v1",
                        "passed_rounds": 1,
                        "recovery_round": 0,
                        "round_results": [{"round": 0, "checks": [{"operation": "rename"}]}],
                    }
                ),
                encoding="utf-8",
            )
            (root / "namespace-recovery.json").write_text(
                json.dumps({"schema": "dms.filesystem.namespace-recovery.v1", "round": 0}),
                encoding="utf-8",
            )
            report = evaluator.evaluate(contract, root)
            self.assertEqual(report["status"], "FAIL")
            self.assertTrue(any("missing checks" in error for error in report["errors"]))

    def test_cli_creates_output_parent_even_when_evaluation_fails(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            contract = root / "contract.json"
            contract.write_text(
                json.dumps({"schema": "dms.filesystem.namespace-contract.v1", "minimum_rounds": 1}),
                encoding="utf-8",
            )
            output = root / "nested" / "evaluation.json"
            completed = subprocess.run(
                [
                    sys.executable,
                    str(SCRIPT_DIR / "evaluate_filesystem_namespace.py"),
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
