#!/usr/bin/env python3
"""Cached mmap evaluator regression tests."""

from __future__ import annotations

import importlib.util
import json
import tempfile
import unittest
from pathlib import Path


MODULE_PATH = Path(__file__).with_name("evaluate_filesystem_mmap.py")
SPEC = importlib.util.spec_from_file_location("evaluate_filesystem_mmap", MODULE_PATH)
assert SPEC is not None and SPEC.loader is not None
evaluator = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(evaluator)


class FilesystemMmapEvaluatorTest(unittest.TestCase):
    def write_complete_evidence(self, directory: Path) -> None:
        checks = [
            {"check": name, "status": "passed"}
            for name in sorted(evaluator.EXPECTED_CHECKS)
        ]
        for item in checks:
            if item["check"] == "cached_page_hit_without_fuse_read":
                item["node_b_fuse_read_delta"] = 0
            if item["check"] == "remote_invalidate_mapped_page":
                item["meta_filesystem_watch_event_delta"] = 1
                item["node_b_kernel_invalidation_ok_delta"] = 1
                item["node_b_fuse_read_delta"] = 0
                item["writer_fsync_returned_before_mapped_visibility"] = True
                item["mapped_visibility_after_writer_fsync"] = True
                item["ack_order_machine_assertion"] = True
        (directory / "mmap-workload.json").write_text(
            json.dumps(
                {
                    "schema": "dms.filesystem.mmap-workload.v1",
                    "status": "passed",
                    "checks": checks,
                }
            )
            + "\n",
            encoding="utf-8",
        )
        (directory / "mmap-recovery.json").write_text(
            json.dumps(
                {
                    "schema": "dms.filesystem.mmap-recovery.v1",
                    "status": "passed",
                }
            )
            + "\n",
            encoding="utf-8",
        )

    def test_complete_evidence_passes(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            directory = Path(raw)
            self.write_complete_evidence(directory)
            result = evaluator.evaluate(directory)
            self.assertEqual(result["status"], "PASS")

    def test_cached_read_amplification_fails(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            directory = Path(raw)
            self.write_complete_evidence(directory)
            evidence = json.loads((directory / "mmap-workload.json").read_text())
            for item in evidence["checks"]:
                if item["check"] == "cached_page_hit_without_fuse_read":
                    item["node_b_fuse_read_delta"] = 1
            (directory / "mmap-workload.json").write_text(json.dumps(evidence) + "\n")
            result = evaluator.evaluate(directory)
            self.assertEqual(result["status"], "FAIL")
            self.assertTrue(any("extra FUSE read" in error for error in result["errors"]))

    def test_missing_recovery_fails(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            directory = Path(raw)
            self.write_complete_evidence(directory)
            (directory / "mmap-recovery.json").unlink()
            result = evaluator.evaluate(directory)
            self.assertEqual(result["status"], "FAIL")
            self.assertTrue(any("recovery" in error for error in result["errors"]))

    def test_remote_invalidation_requires_kernel_invalidation_metric(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            directory = Path(raw)
            self.write_complete_evidence(directory)
            evidence = json.loads((directory / "mmap-workload.json").read_text())
            for item in evidence["checks"]:
                if item["check"] == "remote_invalidate_mapped_page":
                    item["node_b_kernel_invalidation_ok_delta"] = 0
            (directory / "mmap-workload.json").write_text(json.dumps(evidence) + "\n")
            result = evaluator.evaluate(directory)
            self.assertEqual(result["status"], "FAIL")
            self.assertTrue(
                any("kernel invalidation" in error for error in result["errors"])
            )

    def test_remote_invalidation_requires_meta_watch_metric(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            directory = Path(raw)
            self.write_complete_evidence(directory)
            evidence = json.loads((directory / "mmap-workload.json").read_text())
            for item in evidence["checks"]:
                if item["check"] == "remote_invalidate_mapped_page":
                    item["meta_filesystem_watch_event_delta"] = 0
            (directory / "mmap-workload.json").write_text(json.dumps(evidence) + "\n")
            result = evaluator.evaluate(directory)
            self.assertEqual(result["status"], "FAIL")
            self.assertTrue(any("Meta watch" in error for error in result["errors"]))

    def test_remote_invalidation_rejects_string_only_ack_evidence(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            directory = Path(raw)
            self.write_complete_evidence(directory)
            evidence = json.loads((directory / "mmap-workload.json").read_text())
            for item in evidence["checks"]:
                if item["check"] == "remote_invalidate_mapped_page":
                    item.pop("writer_fsync_returned_before_mapped_visibility", None)
                    item.pop("mapped_visibility_after_writer_fsync", None)
                    item.pop("ack_order_machine_assertion", None)
                    item["ack_after_kernel_invalidation_evidence"] = "string-only evidence"
            (directory / "mmap-workload.json").write_text(json.dumps(evidence) + "\n")
            result = evaluator.evaluate(directory)
            self.assertEqual(result["status"], "FAIL")
            self.assertTrue(any("machine ACK-order assertion" in error for error in result["errors"]))


if __name__ == "__main__":
    unittest.main()
