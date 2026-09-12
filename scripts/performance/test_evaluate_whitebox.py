from __future__ import annotations

import copy
import json
from pathlib import Path
import sys
import unittest


SCRIPT_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(SCRIPT_DIR))

import evaluate_whitebox  # noqa: E402


ROOT = SCRIPT_DIR.parents[1]
CONTRACT = json.loads(
    (ROOT / "benchmarks/whitebox/contract.json").read_text(encoding="utf-8")
)
BASELINE = json.loads(
    (ROOT / "benchmarks/whitebox/baselines/lima-aarch64-2026-09-12.json").read_text(
        encoding="utf-8"
    )
)


class WhiteboxEvaluatorTest(unittest.TestCase):
    def test_checked_in_baseline_passes_its_contract(self):
        result = evaluate_whitebox.evaluate(CONTRACT, BASELINE, BASELINE)
        self.assertEqual("PASS", result["status"], result["errors"])

    def test_environment_mismatch_is_not_a_hard_comparison(self):
        candidate = copy.deepcopy(BASELINE)
        candidate["profile"]["id"] = "another-machine"
        result = evaluate_whitebox.evaluate(CONTRACT, BASELINE, candidate)
        self.assertEqual("FAIL", result["status"])
        self.assertTrue(any("profile mismatch" in item for item in result["errors"]))

    def test_extra_rpc_cannot_be_declared_as_the_new_minimum(self):
        candidate = copy.deepcopy(BASELINE)
        candidate["cases"][0]["rpc"] += 1
        result = evaluate_whitebox.evaluate(CONTRACT, BASELINE, candidate)
        self.assertEqual("FAIL", result["status"])
        self.assertTrue(any("audited minimum" in item for item in result["errors"]))

    def test_p50_regression_over_ten_percent_fails(self):
        candidate = copy.deepcopy(BASELINE)
        candidate["cases"][0]["p50_ns"] *= 1.11
        result = evaluate_whitebox.evaluate(CONTRACT, BASELINE, candidate)
        self.assertEqual("FAIL", result["status"])
        self.assertTrue(any("p50 regression" in item for item in result["errors"]))

    def test_architecture_penalty_cannot_hide_unexplained_time(self):
        candidate = copy.deepcopy(BASELINE)
        target = next(
            case for case in candidate["cases"] if case["id"] == "sdk.peer_first_get.4096"
        )
        target["p50_ns"] = target["lower_bound_p50_ns"] * 1.26
        result = evaluate_whitebox.evaluate(CONTRACT, BASELINE, candidate)
        self.assertEqual("FAIL", result["status"])
        self.assertTrue(any("lower-bound budget" in item for item in result["errors"]))


if __name__ == "__main__":
    unittest.main()
