#!/usr/bin/env python3

from __future__ import annotations

import importlib.util
import json
import tempfile
import unittest
from pathlib import Path


MODULE_PATH = Path(__file__).with_name("evaluate_m1_resource_soak.py")
SPEC = importlib.util.spec_from_file_location("evaluate_m1_resource_soak", MODULE_PATH)
assert SPEC is not None and SPEC.loader is not None
evaluator = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(evaluator)


class ResourceSoakEvaluatorTest(unittest.TestCase):
    def write_complete(self, directory: Path) -> None:
        node_before = {
            "rss_bytes": 100,
            "fd_count": 10,
            "threads": 4,
            "arena_allocated_bytes": 4096,
            "arena_reservations": 0,
            "filesystem_inode_references": 0,
        }
        node_after = dict(node_before)
        (directory / "resource-soak.json").write_text(
            json.dumps(
                {
                    "schema": "dms.m1.resource-soak.v1",
                    "status": "passed",
                    "expected_live_nodes": 2,
                    "nodes": [
                        {"name": "node-a", "before": node_before, "after": node_after},
                        {"name": "node-b", "before": node_before, "after": node_after},
                    ],
                    "meta": {
                        "before": {"rss_bytes": 100, "watch_lag_events": 0, "node_sessions_live": 2},
                        "after": {"rss_bytes": 100, "watch_lag_events": 0, "node_sessions_live": 2},
                    },
                }
            )
            + "\n"
        )

    def test_complete_soak_passes(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            directory = Path(raw)
            self.write_complete(directory)
            self.assertEqual(evaluator.evaluate(directory)["status"], "PASS")

    def test_leftover_inode_refs_fail(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            directory = Path(raw)
            self.write_complete(directory)
            evidence = json.loads((directory / "resource-soak.json").read_text())
            evidence["nodes"][0]["after"]["filesystem_inode_references"] = 1
            (directory / "resource-soak.json").write_text(json.dumps(evidence))
            result = evaluator.evaluate(directory)
            self.assertEqual(result["status"], "FAIL")
            self.assertTrue(any("inode references" in error for error in result["errors"]))

    def test_malformed_metric_value_fails(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            directory = Path(raw)
            self.write_complete(directory)
            evidence = json.loads((directory / "resource-soak.json").read_text())
            evidence["nodes"][0]["after"]["rss_bytes"] = "not-a-number"
            (directory / "resource-soak.json").write_text(json.dumps(evidence))
            result = evaluator.evaluate(directory)
            self.assertEqual(result["status"], "FAIL")
            self.assertTrue(any("expected numeric metric" in error for error in result["errors"]))


if __name__ == "__main__":
    unittest.main()
