#!/usr/bin/env python3

from __future__ import annotations

import importlib.util
import json
import tempfile
import unittest
from pathlib import Path


MODULE_PATH = Path(__file__).with_name("evaluate_filesystem_space_sync.py")
SPEC = importlib.util.spec_from_file_location("evaluate_filesystem_space_sync", MODULE_PATH)
assert SPEC is not None and SPEC.loader is not None
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class EvaluateFilesystemSpaceSyncTest(unittest.TestCase):
    def test_complete_evidence_passes(self) -> None:
        contract = {
            "schema": "dms.filesystem.space-sync-contract.v1",
            "required_operations": ["sync_callbacks"],
            "request_amplification_limits": {"sync_only_meta_commits": 0},
        }
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "space-sync-workload.json").write_text(
                json.dumps(
                    {
                        "schema": "dms.filesystem.space-sync-workload.v1",
                        "checks": [
                            {
                                "operation": "sync_callbacks",
                                "meta_commit_delta": 0,
                            }
                        ],
                    }
                )
            )
            (root / "space-sync-meta-recovery.json").write_text(
                json.dumps(
                    {
                        "schema": "dms.filesystem.space-sync-meta-recovery.v1",
                        "reserved_bytes_before": 4096,
                        "reserved_bytes_after": 4096,
                    }
                )
            )
            (root / "space-sync-owner-recovery.json").write_text(
                json.dumps({"schema": "dms.filesystem.space-sync-owner-recovery.v1"})
            )
            metrics = "\n".join(
                [
                    *(f'dms_node_fuse_callbacks_total{{operation="{name}"}} 1' for name in ("fallocate", "flush", "fsync", "fsyncdir")),
                    *(f'dms_node_filesystem_operations_total{{operation="{name}",result="ok"}} 1' for name in ("fallocate", "flush", "sync")),
                ]
            )
            (root / "node-a.prom").write_text(metrics)
            report = MODULE.evaluate(contract, root)
            self.assertEqual(report["status"], "PASS", report)


if __name__ == "__main__":
    unittest.main()
