#!/usr/bin/env python3
"""M1 总执行器的回归测试。"""

from __future__ import annotations

import copy
import importlib.util
import json
import tempfile
import unittest
from pathlib import Path


MODULE_PATH = Path(__file__).with_name("m1_runner.py")
SPEC = importlib.util.spec_from_file_location("m1_runner", MODULE_PATH)
assert SPEC is not None and SPEC.loader is not None
runner = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(runner)
M1_DIR = MODULE_PATH.parent


class M1RunnerTest(unittest.TestCase):
    def test_profile_variables_can_reference_common_variables(self) -> None:
        self.assertEqual(
            runner.resolve_variables(
                {
                    "root": "/workspace/source",
                    "binary": "{root}/target/release/dms-node",
                }
            )["binary"],
            "/workspace/source/target/release/dms-node",
        )
        with self.assertRaisesRegex(runner.ProfileError, "循环引用"):
            runner.resolve_variables({"left": "{right}", "right": "{left}"})

    def test_variable_overrides_require_name_value(self) -> None:
        self.assertEqual(
            runner.parse_variable_overrides(
                ["vm_root=/workspace/dms/source", "rounds=3"]
            ),
            {"vm_root": "/workspace/dms/source", "rounds": "3"},
        )
        with self.assertRaisesRegex(runner.ProfileError, "NAME=VALUE"):
            runner.parse_variable_overrides(["missing-separator"])

    def test_run_nonce_is_unique_and_safe_for_remote_paths(self) -> None:
        first = runner.make_run_nonce()
        second = runner.make_run_nonce()
        self.assertNotEqual(first, second)
        self.assertRegex(first, r"^[0-9-]+[0-9a-f]{6}$")
        self.assertEqual(first, first.lower())
        self.assertLessEqual(
            len(f"resource-return-to-baseline-{first}-node-a"),
            64,
        )

    def manifest(self) -> dict:
        return {
            "cases": [
                {
                    "id": "case-a",
                    "implementation_status": "implemented",
                    "tiers": ["fast"],
                    "topologies": ["single-vm"],
                }
            ]
        }

    def profile(self, command: list[str]) -> dict:
        return {
            "schema": runner.PROFILE_SCHEMA,
            "name": "unit",
            "commands": {
                "case-a": {
                    "argv": command,
                    "expected_evidence": ["{case_dir}/proof.txt"],
                }
            },
        }

    def test_profile_must_cover_selected_cases(self) -> None:
        profile = self.profile(["true"])
        profile["commands"].clear()
        with self.assertRaisesRegex(runner.ProfileError, "缺少命令"):
            runner.validate_profile(profile, self.manifest(), "fast", "single-vm")

    def test_checked_in_m1_contract_has_no_global_performance_threshold(self) -> None:
        manifest = json.loads((M1_DIR / "acceptance-manifest.json").read_text(encoding="utf-8"))
        policy = manifest["policy"]
        self.assertNotIn("performance_regression_limit_percent", policy)
        self.assertNotIn("minimum_performance_runs", policy)
        performance = {
            case["id"]: case
            for case in manifest["cases"]
            if case["id"] == "performance-regression"
        }["performance-regression"]
        self.assertIn("分类性能合同", performance["expected"])

    def test_checked_in_single_vm_profile_uses_stage_neutral_suite_paths(self) -> None:
        profile = json.loads(
            (M1_DIR / "profiles" / "single-vm.json").read_text(encoding="utf-8")
        )
        variables = profile["variables"]
        self.assertEqual("/tmp/dms-m1/pjdfstest", variables["pjdfstest_dir"])
        self.assertEqual("/tmp/dms-m1/xfstests-dev", variables["fstests_dir"])
        self.assertNotIn("g004", json.dumps(variables))

    def test_three_vm_performance_cases_receive_acceptance_purpose(self) -> None:
        profile = json.loads(
            (M1_DIR / "profiles" / "three-vm.json").read_text(encoding="utf-8")
        )
        for case_id in ("whitebox-path-gate", "performance-regression"):
            argv = profile["commands"][case_id]["argv"]
            self.assertIn("--purpose", argv)
            self.assertEqual("{purpose}", argv[argv.index("--purpose") + 1])
        self.assertIn(
            "--reuse-existing",
            profile["commands"]["performance-regression"]["argv"],
        )

    def test_planned_case_cannot_be_executed_as_complete_acceptance(self) -> None:
        manifest = self.manifest()
        manifest["cases"][0]["implementation_status"] = "planned"
        with self.assertRaisesRegex(runner.ProfileError, "planned"):
            runner.validate_profile(self.profile(["true"]), manifest, "fast", "single-vm")

    def test_success_requires_declared_evidence(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            case = self.manifest()["cases"][0]
            profile = self.profile(
                [
                    "python3",
                    "-c",
                    "from pathlib import Path; Path(r'{case_dir}/proof.txt').write_text('ok')",
                ]
            )
            result = runner.run_case(case, profile["commands"]["case-a"], {"root": str(root)}, root)
            self.assertEqual(result["status"], "PASS")
            self.assertTrue((root / "cases" / "case-a" / "proof.txt").is_file())

    def test_zero_exit_without_evidence_fails(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            case = self.manifest()["cases"][0]
            result = runner.run_case(
                case,
                self.profile(["true"])["commands"]["case-a"],
                {"root": str(root)},
                root,
            )
            self.assertEqual(result["status"], "FAIL")
            self.assertIn("evidence is missing", result["message"])

    def test_nonzero_exit_is_recorded_and_does_not_raise(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            case = self.manifest()["cases"][0]
            command = copy.deepcopy(self.profile(["python3", "-c", "raise SystemExit(7)"]))
            command["commands"]["case-a"]["expected_evidence"] = []
            result = runner.run_case(
                case,
                command["commands"]["case-a"],
                {"root": str(root)},
                root,
            )
            self.assertEqual(result["status"], "FAIL")
            execution = json.loads(
                (root / "cases" / "case-a" / "execution.json").read_text(encoding="utf-8")
            )
            self.assertEqual(execution["returncode"], 7)


if __name__ == "__main__":
    unittest.main()
