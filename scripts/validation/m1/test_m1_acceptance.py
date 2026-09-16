#!/usr/bin/env python3
"""M1 验收评估器的回归测试。"""

from __future__ import annotations

import copy
import importlib.util
import tempfile
import unittest
from pathlib import Path


MODULE_PATH = Path(__file__).with_name("m1_acceptance.py")
SPEC = importlib.util.spec_from_file_location("m1_acceptance", MODULE_PATH)
assert SPEC is not None and SPEC.loader is not None
acceptance = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(acceptance)


class M1AcceptanceTest(unittest.TestCase):
    def setUp(self) -> None:
        self.manifest, self.known_gaps = acceptance.load_contract()

    def result(self, tier: str, topology: str, *, purpose: str = "discovery") -> dict:
        cases = acceptance.select_cases(self.manifest, tier, topology)
        return {
            "schema": "dms.m1.acceptance-result.v1",
            "purpose": purpose,
            "tier": tier,
            "topology": topology,
            "source": {"commit": "0123456789abcdef", "dirty": False},
            "environment": {"kernel": "test", "arch": "aarch64", "profile": "unit"},
            "cases": [
                {"id": case["id"], "status": "PASS", "evidence": ["unit://pass"]}
                for case in cases
                if case["implementation_status"] == "implemented"
            ],
        }

    def test_contract_is_valid(self) -> None:
        summary = acceptance.validate_contract(self.manifest, self.known_gaps)
        self.assertIn("exemptions=0", summary)

    def test_missing_executor_is_rejected_only_for_implemented_case(self) -> None:
        manifest = copy.deepcopy(self.manifest)
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for case in manifest["cases"]:
                case["implementation_status"] = "planned"
            acceptance.validate_contract(manifest, self.known_gaps, root=root)

            manifest["cases"][0]["implementation_status"] = "implemented"
            with self.assertRaisesRegex(acceptance.ContractError, "缺少执行入口"):
                acceptance.validate_contract(manifest, self.known_gaps, root=root)

    def test_fast_single_vm_discovery_passes_after_m1_6_cases_are_implemented(self) -> None:
        evaluated = acceptance.evaluate(
            self.manifest,
            self.known_gaps,
            self.result("fast", "single-vm"),
        )
        self.assertEqual(evaluated["status"], "PASS")
        self.assertEqual(evaluated["planned_cases"], [])
        self.assertNotIn("lock-single-vm", evaluated["planned_cases"])
        self.assertNotIn("mmap-single-vm", evaluated["planned_cases"])
        self.assertEqual(
            next(
                case["implementation_status"]
                for case in self.manifest["cases"]
                if case["id"] == "lock-three-vm-fault"
            ),
            "implemented",
        )
        self.assertEqual(
            next(
                case["implementation_status"]
                for case in self.manifest["cases"]
                if case["id"] == "mmap-single-vm"
            ),
            "implemented",
        )

    def test_full_release_rejects_dirty_source(self) -> None:
        result = self.result("full", "single-vm", purpose="release")
        result["source"]["dirty"] = True
        evaluated = acceptance.evaluate(
            self.manifest,
            self.known_gaps,
            result,
        )
        self.assertEqual(evaluated["status"], "FAIL")
        self.assertTrue(any("源码目录无未提交修改" in error for error in evaluated["errors"]))

    def test_skip_without_exemption_fails(self) -> None:
        result = self.result("fast", "single-vm")
        result["cases"][0]["status"] = "SKIP"
        evaluated = acceptance.evaluate(self.manifest, self.known_gaps, result)
        self.assertEqual(evaluated["status"], "FAIL")
        self.assertTrue(any("没有带 Issue" in error for error in evaluated["errors"]))

    def test_missing_required_result_fails(self) -> None:
        result = self.result("fast", "single-vm")
        removed = result["cases"].pop()["id"]
        evaluated = acceptance.evaluate(self.manifest, self.known_gaps, result)
        self.assertEqual(evaluated["status"], "FAIL")
        self.assertTrue(any(removed in error for error in evaluated["errors"]))


if __name__ == "__main__":
    unittest.main()
