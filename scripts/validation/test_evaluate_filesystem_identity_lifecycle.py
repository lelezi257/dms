from __future__ import annotations

import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPT_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(SCRIPT_DIR))

import evaluate_filesystem_identity_lifecycle as evaluator
import extract_filesystem_identity_whitebox as whitebox_extractor
import run_filesystem_identity_lifecycle_3vm as three_vm_runner


def passing_workload() -> dict:
    return {
        "schema": "dms.filesystem.identity-lifecycle-workload.v1",
        "passed_operations": 5,
        "checks": [
            {
                "operation": "hardlink_same_inode",
                "same_inode": True,
                "source_nlink": 2,
                "linked_nlink": 2,
                "remote_stat_attempts": 1,
            },
            {
                "operation": "unlink_one_link_keeps_other_link",
                "removed_exists": False,
                "remaining_nlink": 1,
                "remaining_bytes": 9,
            },
            {
                "operation": "unlink_open_keeps_file_readable_until_close",
                "namespace_visible_after_unlink": False,
                "fd_read_bytes": 9,
                "close_completed": True,
            },
            {
                "operation": "symlink_readlink_exact_target",
                "readlink_matches": True,
                "readlink_bytes": 10,
                "target_survived_unlink": True,
            },
            {
                "operation": "orphan_lifecycle_observed",
                "namespace_visible_after_last_unlink": False,
                "open_ref_protected_read": True,
                "inode": 42,
            },
        ],
        "recovery_hardlink_path": "/identity/recovery-hardlink-b",
        "recovery_hardlink_bytes": 9,
        "recovery_symlink_path": "/identity/recovery-symlink",
        "recovery_symlink_target": "recovery-target",
    }


def passing_recovery() -> dict:
    return {
        "schema": "dms.filesystem.identity-lifecycle-recovery.v1",
        "hardlink_remaining_path": "/identity/recovery-hardlink-b",
        "hardlink_bytes": 9,
        "symlink_path": "/identity/recovery-symlink",
        "readlink": "recovery-target",
        "orphan_namespace_visible": False,
    }


class EvaluateFilesystemIdentityLifecycleTests(unittest.TestCase):
    def test_three_vm_run_id_rejects_remote_path_injection(self) -> None:
        self.assertEqual(three_vm_runner.validated_run_id("identity_01"), "identity_01")
        for invalid in ("../escape", "with/slash", "$(touch-pwned)", "", "a" * 65):
            with self.assertRaises(Exception):
                three_vm_runner.validated_run_id(invalid)

    def write_result_dir(
        self,
        root: Path,
        workload: dict | None = None,
        recovery: dict | None = None,
    ) -> None:
        (root / "identity-workload.json").write_text(
            json.dumps(workload if workload is not None else passing_workload()),
            encoding="utf-8",
        )
        (root / "identity-recovery.json").write_text(
            json.dumps(recovery if recovery is not None else passing_recovery()),
            encoding="utf-8",
        )
        (root / "node-a.prom").write_text(
            "dms_node_filesystem_operations_total 1\n"
            'dms_node_filesystem_inode_reference_transitions_total{transition="release_final"} 1\n',
            encoding="utf-8",
        )
        (root / "node-b.prom").write_text("dms_node_filesystem_operations_total 1\n", encoding="utf-8")
        (root / "meta.prom").write_text(
            "dms_meta_operations_total 1\n"
            'dms_meta_journal_appends_total{record_type="filesystem_orphan_reaped",result="ok"} 1\n',
            encoding="utf-8",
        )
        (root / "identity-whitebox.json").write_text(
            json.dumps({"reference_lifecycle_and_reap_seen": True}), encoding="utf-8"
        )

    def test_evaluator_accepts_complete_identity_lifecycle_result(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.write_result_dir(root)
            report = evaluator.evaluate(evaluator.DEFAULT_CONTRACT, root)
            self.assertEqual(report["status"], "PASS")

    def test_evaluator_rejects_missing_required_whitebox(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.write_result_dir(root)
            (root / "identity-whitebox.json").unlink()
            report = evaluator.evaluate(evaluator.DEFAULT_CONTRACT, root)
            self.assertEqual(report["status"], "FAIL")
            self.assertTrue(
                any("identity-whitebox.json" in gap for gap in report["observability"]["evidence_gaps"])
            )

    def test_evaluator_can_require_reference_and_reap_evidence(self) -> None:
        contract = dict(evaluator.DEFAULT_CONTRACT)
        contract["require_reference_and_reap_evidence"] = True
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.write_result_dir(root)
            (root / "identity-whitebox.json").unlink()
            report = evaluator.evaluate(contract, root)
            self.assertEqual(report["status"], "FAIL")
            self.assertTrue(any("whitebox" in error for error in report["errors"]))

    def test_evaluator_rejects_missing_required_operation(self) -> None:
        candidate = passing_workload()
        candidate["checks"] = [
            check for check in candidate["checks"] if check["operation"] != "symlink_readlink_exact_target"
        ]
        candidate["passed_operations"] = len(candidate["checks"])
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.write_result_dir(root, workload=candidate)
            report = evaluator.evaluate(evaluator.DEFAULT_CONTRACT, root)
            self.assertEqual(report["status"], "FAIL")
            self.assertTrue(any("symlink_readlink_exact_target" in error for error in report["errors"]))

    def test_evaluator_rejects_hardlink_that_uses_two_inodes(self) -> None:
        candidate = passing_workload()
        for check in candidate["checks"]:
            if check["operation"] == "hardlink_same_inode":
                check["same_inode"] = False
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.write_result_dir(root, workload=candidate)
            report = evaluator.evaluate(evaluator.DEFAULT_CONTRACT, root)
            self.assertEqual(report["status"], "FAIL")
            self.assertTrue(any("hardlink_same_inode" in error for error in report["errors"]))

    def test_evaluator_rejects_orphan_resurrection_after_recovery(self) -> None:
        recovery = passing_recovery()
        recovery["orphan_namespace_visible"] = True
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.write_result_dir(root, recovery=recovery)
            report = evaluator.evaluate(evaluator.DEFAULT_CONTRACT, root)
            self.assertEqual(report["status"], "FAIL")
            self.assertTrue(any("resurrected" in error for error in report["errors"]))

    def test_cli_creates_output_parent_even_when_evaluation_fails(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            output = root / "nested" / "evaluation.json"
            completed = subprocess.run(
                [
                    sys.executable,
                    str(SCRIPT_DIR / "evaluate_filesystem_identity_lifecycle.py"),
                    str(root / "missing-result-dir"),
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

    def test_whitebox_extractor_correlates_reference_and_reap_by_inode(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "identity-workload.json").write_text(
                json.dumps(passing_workload()), encoding="utf-8"
            )
            (root / "node-a.log").write_text(
                json.dumps(
                    {
                        "event": "node.filesystem.reference.acquired",
                        "inode": 42,
                    }
                )
                + "\n"
                + json.dumps(
                    {
                        "event": "node.filesystem.reference.released_final",
                        "inode": 42,
                    }
                )
                + "\n",
                encoding="utf-8",
            )
            (root / "meta.log").write_text(
                json.dumps({"event": "meta.filesystem.orphan.reaped", "inode": 42}) + "\n",
                encoding="utf-8",
            )
            evidence = whitebox_extractor.extract(root)
            self.assertTrue(evidence["reference_lifecycle_and_reap_seen"])


if __name__ == "__main__":
    unittest.main()
