from __future__ import annotations

import copy
import json
from pathlib import Path
import sys
import unittest


SCRIPT_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(SCRIPT_DIR))

import evaluate_native_filesystem  # noqa: E402


ROOT = SCRIPT_DIR.parents[1]
CONTRACT = json.loads(
    (ROOT / "benchmarks/whitebox/native-filesystem-vs-glue-contract.json").read_text(
        encoding="utf-8"
    )
)


def passing_result() -> dict:
    cases = {}
    for rule in CONTRACT["cases"]:
        cases[rule["id"]] = {
            "correctness": True,
            "samples": 30,
            "p50_us": 80.0,
            "p95_us": 100.0,
            "unattributed_fraction": 0.05,
            "path_ledger": copy.deepcopy(rule["native_path"]),
        }
    glue_cases = {
        case_id: {
            "correctness": True,
            "samples": 30,
            "p50_us": 100.0,
            "p95_us": 100.0,
        }
        for case_id in cases
    }
    return {
        "schema": "dms.native-filesystem-vs-glue-result.v1",
        "same_environment": True,
        "workload": copy.deepcopy(CONTRACT["workload"]),
        "backends": {
            "native": {"correctness": True, "cases": cases},
            "glue": {"correctness": True, "cases": glue_cases},
        },
    }


class NativeFilesystemEvaluatorTest(unittest.TestCase):
    def test_matching_result_passes(self):
        result = evaluate_native_filesystem.evaluate(CONTRACT, passing_result())
        self.assertEqual("PASS", result["status"], result["errors"])

    def test_workload_must_match(self):
        candidate = passing_result()
        candidate["workload"]["file_count"] = 12
        result = evaluate_native_filesystem.evaluate(CONTRACT, candidate)
        self.assertEqual("FAIL", result["status"])
        self.assertTrue(any("file_count" in error for error in result["errors"]))

    def test_native_and_glue_must_share_environment(self):
        candidate = passing_result()
        candidate["same_environment"] = False
        result = evaluate_native_filesystem.evaluate(CONTRACT, candidate)
        self.assertEqual("FAIL", result["status"])
        self.assertTrue(any("same environment" in error for error in result["errors"]))

    def test_process_internal_path_cannot_use_worker_rpc(self):
        candidate = passing_result()
        candidate["backends"]["native"]["cases"]["local_hot_read.4096"]["path_ledger"][
            "frontend_worker_rpc"
        ] = 1
        result = evaluate_native_filesystem.evaluate(CONTRACT, candidate)
        self.assertEqual("FAIL", result["status"])
        self.assertTrue(any("frontend_worker_rpc" in error for error in result["errors"]))

    def test_peer_first_read_requires_one_resolve_and_one_pull(self):
        candidate = passing_result()
        candidate["backends"]["native"]["cases"]["peer_first_read.65536"]["path_ledger"][
            "peer_pull"
        ] = 2
        result = evaluate_native_filesystem.evaluate(CONTRACT, candidate)
        self.assertEqual("FAIL", result["status"])
        self.assertTrue(any("peer_pull" in error for error in result["errors"]))

    def test_class_specific_improvement_is_enforced(self):
        candidate = passing_result()
        candidate["backends"]["native"]["cases"]["peer_first_read.4096"]["p50_us"] = 96.0
        result = evaluate_native_filesystem.evaluate(CONTRACT, candidate)
        self.assertEqual("FAIL", result["status"])
        self.assertTrue(any("below 5.000%" in error for error in result["errors"]))

    def test_create_p95_regression_above_twenty_percent_fails(self):
        candidate = passing_result()
        candidate["backends"]["native"]["cases"]["create_write.4096"]["p95_us"] = 121.0
        result = evaluate_native_filesystem.evaluate(CONTRACT, candidate)
        self.assertEqual("FAIL", result["status"])
        self.assertTrue(any("p95 ratio" in error for error in result["errors"]))

    def test_synchronous_create_cannot_regress_more_than_fifteen_percent(self):
        candidate = passing_result()
        candidate["backends"]["native"]["cases"]["create_write.4096"]["p50_us"] = 116.0
        result = evaluate_native_filesystem.evaluate(CONTRACT, candidate)
        self.assertEqual("FAIL", result["status"])
        self.assertTrue(any("p50 ratio" in error for error in result["errors"]))

    def test_synchronous_overwrite_uses_same_fifteen_percent_tail_bound(self):
        candidate = passing_result()
        overwrite = candidate["backends"]["native"]["cases"]["middle_overwrite.65536"]
        overwrite["p95_us"] = 114.9
        self.assertEqual(
            "PASS",
            evaluate_native_filesystem.evaluate(CONTRACT, candidate)["status"],
        )
        overwrite["p95_us"] = 115.1
        result = evaluate_native_filesystem.evaluate(CONTRACT, candidate)
        self.assertEqual("FAIL", result["status"])
        self.assertTrue(any("p95 ratio" in error for error in result["errors"]))

    def test_unattributed_time_above_ten_percent_fails(self):
        candidate = passing_result()
        candidate["backends"]["native"]["cases"]["middle_overwrite.65536"][
            "unattributed_fraction"
        ] = 0.11
        result = evaluate_native_filesystem.evaluate(CONTRACT, candidate)
        self.assertEqual("FAIL", result["status"])
        self.assertTrue(any("unattributed_fraction" in error for error in result["errors"]))


if __name__ == "__main__":
    unittest.main()
