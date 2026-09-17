#!/usr/bin/env python3
"""M1 performance wrapper 的轻量合同测试。"""

from __future__ import annotations

import importlib.util
import json
from argparse import Namespace
from pathlib import Path
import tempfile
import unittest


MODULE_PATH = Path(__file__).with_name("run_m1_performance.py")
SPEC = importlib.util.spec_from_file_location("run_m1_performance", MODULE_PATH)
assert SPEC is not None and SPEC.loader is not None
module = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(module)


class M1PerformanceTest(unittest.TestCase):
    def setUp(self) -> None:
        self._original_source_head = module.current_source_head
        module.current_source_head = lambda: "current-head"

    def tearDown(self) -> None:
        module.current_source_head = self._original_source_head

    def write_pass_files(self, output: Path) -> None:
        (output / "performance-evaluation.json").write_text(
            json.dumps(
                {
                    "schema": "dms.native-filesystem-vs-glue-evaluation.v1",
                    "status": "PASS",
                }
            ),
            encoding="utf-8",
        )
        (output / "amplification-evaluation.json").write_text(
            json.dumps(
                {
                    "schema": "dms.fuse-request-amplification-evaluation.v1",
                    "status": "PASS",
                }
            ),
            encoding="utf-8",
        )

    def write_result(
        self,
        output: Path,
        *,
        source_head: str = "current-head",
        include_hashes: bool = True,
        include_provenance: bool = True,
    ) -> dict:
        artifacts = {
            "dms_node": output / "dms-node",
            "dms_meta": output / "dms-meta",
            "juicefs": output / "juicefs",
        }
        for name, path in artifacts.items():
            path.write_bytes(f"{name}-bytes".encode("utf-8"))
        provenance_path = output / "juicefs-provenance.json"
        provenance_path.write_text('{"schema":"test"}\n', encoding="utf-8")
        environment: dict[str, object] = {
            "source_head": source_head,
            "artifacts": {name: str(path) for name, path in artifacts.items()},
        }
        if include_hashes:
            environment["hashes"] = {
                name: module.sha256_file(path) for name, path in artifacts.items()
            }
        if include_provenance:
            environment["artifact_provenance"] = {
                "juicefs": {
                    "path": str(artifacts["juicefs"]),
                    "sha256": module.sha256_file(artifacts["juicefs"]),
                    "provenance": {
                        "path": str(provenance_path),
                        "sha256": module.sha256_file(provenance_path),
                    },
                }
            }
        result = {
            "schema": "dms.native-filesystem-vs-glue-result.v1",
            "same_environment": True,
            "workload": {"rounds": 6},
            "environment": environment,
        }
        (output / "result.json").write_text(json.dumps(result), encoding="utf-8")
        self.write_pass_files(output)
        return result

    def test_new_run_refuses_unprovenanced_prebuilt_juicefs(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            args = Namespace(
                juicefs_source=None,
                juicefs=root / "juicefs",
                juicefs_provenance=None,
            )
            with self.assertRaisesRegex(RuntimeError, "juicefs-provenance"):
                module.prepare_juicefs(args, root / "output")

    def test_existing_result_requires_same_environment_six_rounds_and_two_passes(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)
            self.write_result(output)
            module.validate_existing(output, 6)

            value = json.loads((output / "result.json").read_text(encoding="utf-8"))
            value["same_environment"] = False
            (output / "result.json").write_text(json.dumps(value), encoding="utf-8")
            with self.assertRaisesRegex(RuntimeError, "same run"):
                module.validate_existing(output, 6)

    def test_existing_result_rejects_stale_source_head_by_default(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)
            self.write_result(output, source_head="old-head")
            with self.assertRaisesRegex(RuntimeError, "current HEAD"):
                module.validate_existing(output, 6)

    def test_existing_result_rejects_missing_freshness_fields_by_default(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)
            self.write_result(output, include_hashes=False)
            with self.assertRaisesRegex(RuntimeError, "artifact paths or hashes"):
                module.validate_existing(output, 6)

        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)
            self.write_result(output, include_provenance=False)
            with self.assertRaisesRegex(RuntimeError, "artifact_provenance"):
                module.validate_existing(output, 6)

    def test_existing_result_rejects_artifact_hash_drift_by_default(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)
            self.write_result(output)
            (output / "dms-node").write_bytes(b"changed")
            with self.assertRaisesRegex(RuntimeError, "artifact hash changed: dms_node"):
                module.validate_existing(output, 6)

    def test_stale_evidence_requires_explicit_discovery_boundary(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)
            self.write_result(output, source_head="old-head", include_hashes=False, include_provenance=False)
            module.validate_existing(output, 6, allow_stale_evidence=True, purpose="discovery")
            with self.assertRaisesRegex(RuntimeError, "release"):
                module.validate_existing(output, 6, allow_stale_evidence=True, purpose="release")

    def test_release_rounds_are_balanced(self) -> None:
        module.validate_round_count(6)
        module.validate_round_count(8)
        with self.assertRaisesRegex(RuntimeError, "at least six"):
            module.validate_round_count(4)
        with self.assertRaisesRegex(RuntimeError, "even number"):
            module.validate_round_count(7)


if __name__ == "__main__":
    unittest.main()
