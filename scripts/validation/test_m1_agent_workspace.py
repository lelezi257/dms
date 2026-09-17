#!/usr/bin/env python3
"""Agent workspace workload/evaluator regression tests."""

from __future__ import annotations

import importlib.util
import json
import tempfile
import unittest
from pathlib import Path


SCRIPT_DIR = Path(__file__).resolve().parent


def load_module(name: str, path: Path):
    spec = importlib.util.spec_from_file_location(name, path)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


workload = load_module("filesystem_agent_workspace_workload", SCRIPT_DIR / "filesystem_agent_workspace_workload.py")
evaluator = load_module("evaluate_m1_agent_workspace", SCRIPT_DIR / "evaluate_m1_agent_workspace.py")
runner = load_module("run_m1_agent_workspace", SCRIPT_DIR / "run_m1_agent_workspace.py")


class AgentWorkspaceTest(unittest.TestCase):
    def test_remote_verification_uses_digest_instead_of_embedding_payload(self) -> None:
        payload = bytes(range(256)) * 256
        source = runner._peer_file_verification_source("/mnt/workspace/file.bin", payload)
        self.assertIn(workload.digest_bytes(payload), source)
        self.assertIn(f"expected_size={len(payload)}", source)
        self.assertLess(len(source), 1024)

    def test_plan_is_replayable_and_covers_required_ops(self) -> None:
        first = workload.build_plan(42, 160)
        second = workload.build_plan(42, 160)
        self.assertEqual(first, second)
        counts: dict[str, int] = {}
        for operation in first:
            counts[operation["op"]] = counts.get(operation["op"], 0) + 1
        self.assertTrue(evaluator.REQUIRED_OPS <= set(counts), counts)

    def test_reference_model_digest_changes_with_content(self) -> None:
        model_a = {"workspace/a": b"one"}
        model_b = {"workspace/a": b"two"}
        self.assertNotEqual(workload.model_digest(model_a), workload.model_digest(model_b))

    def write_valid_evidence(self, directory: Path) -> None:
        model = {"workspace/pkg-00/file.bin": b"hello"}
        manifest = workload.model_manifest(model)
        evidence = {
            "schema": workload.SCHEMA,
            "status": "passed",
            "deployment": "three-vm",
            "operation_count": 160,
            "operation_counts": {op: 1 for op in evaluator.REQUIRED_OPS},
            "cross_node_verifications": 200,
            "latency_summary": {"count": 160, "p50_us": 1.0, "p95_us": 2.0, "p99_us": 3.0, "max_us": 4.0},
            "model_digest": workload.model_digest(model),
            "tree_digest_a": workload.tree_digest(manifest),
            "tree_digest_b": workload.tree_digest(manifest),
            "model_manifest": manifest,
            "tree_a": manifest,
            "tree_b": manifest,
            "request_amplification_summary": {
                "operation_count": 160,
                "metric_delta_series": 2,
                "interesting_deltas": {"A": {"dms_node_filesystem_requests_total": 160.0}, "B": {}, "C": {}},
            },
        }
        (directory / "agent-workspace.json").write_text(json.dumps(evidence), encoding="utf-8")
        (directory / "agent-workspace-plan.json").write_text(json.dumps({"schema": workload.PLAN_SCHEMA}), encoding="utf-8")
        (directory / "profile.json").write_text(json.dumps({"schema": "profile"}), encoding="utf-8")
        for name in (
            "node-a-before.prom",
            "node-a-after.prom",
            "node-b-before.prom",
            "node-b-after.prom",
            "meta-before.prom",
            "meta-after.prom",
        ):
            (directory / name).write_text("# test\n", encoding="utf-8")

    def test_evaluator_accepts_complete_evidence(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            directory = Path(tmp)
            self.write_valid_evidence(directory)
            result = evaluator.evaluate(directory)
            self.assertEqual(result["status"], "PASS", result)

    def test_evaluator_rejects_missing_operation_class(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            directory = Path(tmp)
            self.write_valid_evidence(directory)
            evidence = json.loads((directory / "agent-workspace.json").read_text())
            evidence["operation_counts"].pop("rename")
            (directory / "agent-workspace.json").write_text(json.dumps(evidence), encoding="utf-8")
            result = evaluator.evaluate(directory)
            self.assertEqual(result["status"], "FAIL")
            self.assertTrue(any("missing required operation" in error for error in result["errors"]))

    def test_evaluator_rejects_tree_mismatch(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            directory = Path(tmp)
            self.write_valid_evidence(directory)
            evidence = json.loads((directory / "agent-workspace.json").read_text())
            evidence["tree_b"]["workspace/pkg-00/file.bin"]["sha256"] = "bad"
            (directory / "agent-workspace.json").write_text(json.dumps(evidence), encoding="utf-8")
            result = evaluator.evaluate(directory)
            self.assertEqual(result["status"], "FAIL")
            self.assertTrue(any("tree_b" in error for error in result["errors"]))


if __name__ == "__main__":
    unittest.main()
