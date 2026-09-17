#!/usr/bin/env python3

from __future__ import annotations

import importlib.util
import json
import tempfile
import unittest
from pathlib import Path


MODULE_PATH = Path(__file__).with_name("evaluate_m1_fault_matrix.py")
SPEC = importlib.util.spec_from_file_location("evaluate_m1_fault_matrix", MODULE_PATH)
assert SPEC is not None and SPEC.loader is not None
evaluator = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(evaluator)


class FaultMatrixEvaluatorTest(unittest.TestCase):
    def write_complete(self, root: Path) -> None:
        (root / "size").mkdir()
        (root / "locks").mkdir()
        (root / "mmap").mkdir()
        (root / "size/evaluation.json").write_text(json.dumps({"status": "PASS"}))
        (root / "size/size-workload.json").write_text(
            json.dumps({"checks": [{"operation": "cross_node_visibility"}]})
        )
        (root / "size/size-recovery.json").write_text(json.dumps({"status": "passed"}))
        (root / "locks/distributed-locks-3vm.json").write_text(
            json.dumps(
                {
                    "status": "passed",
                    "checks": [
                        {"operation": "blocking_wakeup"},
                        {"operation": "meta_restart_reclaim"},
                        {"operation": "node_restart_epoch_fencing"},
                    ],
                }
            )
        )
        (root / "mmap/evaluation.json").write_text(json.dumps({"status": "PASS"}))
        (root / "mmap/mmap-workload.json").write_text(
            json.dumps(
                {
                    "checks": [
                        {"check": "map_shared_msync"},
                        {
                            "check": "remote_invalidate_mapped_page",
                            "ack_order_machine_assertion": True,
                            "node_b_kernel_invalidation_ok_delta": 1,
                            "meta_filesystem_watch_event_delta": 1,
                        },
                    ]
                }
            )
        )
        (root / "mmap/mmap-recovery.json").write_text(json.dumps({"status": "passed"}))

    def test_complete_matrix_passes(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            self.write_complete(root)
            self.assertEqual(evaluator.evaluate(root)["status"], "PASS")

    def test_missing_ack_order_fails(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            self.write_complete(root)
            data = json.loads((root / "mmap/mmap-workload.json").read_text())
            data["checks"][1]["ack_order_machine_assertion"] = False
            (root / "mmap/mmap-workload.json").write_text(json.dumps(data))
            result = evaluator.evaluate(root)
            self.assertEqual(result["status"], "FAIL")
            self.assertTrue(any("ACK" in error for error in result["errors"]))


if __name__ == "__main__":
    unittest.main()
