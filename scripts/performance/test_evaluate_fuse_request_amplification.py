from __future__ import annotations

import json
from pathlib import Path
import sys
import unittest


SCRIPT_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(SCRIPT_DIR))

import evaluate_fuse_request_amplification as evaluator  # noqa: E402


ROOT = SCRIPT_DIR.parents[1]
CONTRACT = json.loads(
    (ROOT / "benchmarks/whitebox/fuse-request-amplification-contract.json").read_text(
        encoding="utf-8"
    )
)


def make_result() -> dict:
    cases = {}
    labels = CONTRACT["required_operation_labels"]
    for rule in CONTRACT["cases"]:
        per_operation = {
            group: {label: 0 for label in group_labels}
            for group, group_labels in labels.items()
        }
        per_operation["bytes"] = {
            "fuse_read": 0,
            "fuse_write": 0,
            "data_core_read_resolved": 0,
            "data_core_prepare_put": 0,
            "data_core_prepare_range": 0,
        }
        cases[rule["id"]] = {
            "correctness": True,
            "samples": 40,
            "p50_us": 100.0,
            "p95_us": 120.0,
            "amplification_ledger": {
                "totals": {},
                "per_user_operation": per_operation,
                "copy_model": {"full_copy_stages_per_segment": 0},
            },
        }
    return {
        "schema": "dms.native-filesystem-vs-glue-result.v1",
        "same_environment": True,
        "backends": {"native": {"correctness": True, "cases": cases}},
    }


class FuseRequestAmplificationEvaluatorTest(unittest.TestCase):
    def test_matching_result_passes(self):
        candidate = make_result()
        result = evaluator.evaluate(CONTRACT, candidate)
        self.assertEqual("PASS", result["status"], result["errors"])

    def test_native_only_regression_run_does_not_claim_backend_comparison(self):
        candidate = make_result()
        candidate["same_environment"] = False
        candidate["measured_backends"] = ["native"]
        result = evaluator.evaluate(CONTRACT, candidate)
        self.assertEqual("PASS", result["status"], result["errors"])

    def test_two_backend_run_still_requires_same_environment(self):
        candidate = make_result()
        candidate["same_environment"] = False
        candidate["measured_backends"] = ["native", "glue"]
        result = evaluator.evaluate(CONTRACT, candidate)
        self.assertEqual("FAIL", result["status"])
        self.assertTrue(any("same environment" in error for error in result["errors"]))

    def test_missing_boundary_label_fails(self):
        candidate = make_result()
        del candidate["backends"]["native"]["cases"]["metadata_hot.4096"][
            "amplification_ledger"
        ]["per_user_operation"]["fuse_callbacks"]["readdir"]
        result = evaluator.evaluate(CONTRACT, candidate)
        self.assertEqual("FAIL", result["status"])
        self.assertTrue(any("labels missing" in error for error in result["errors"]))

    def test_amplification_above_bound_fails(self):
        candidate = make_result()
        candidate["backends"]["native"]["cases"]["local_hot_read.4096"][
            "amplification_ledger"
        ]["per_user_operation"]["meta_rpc"]["lookup"] = 1
        result = evaluator.evaluate(CONTRACT, candidate)
        self.assertEqual("FAIL", result["status"])
        self.assertTrue(any("meta_rpc.lookup" in error for error in result["errors"]))

    def test_latency_is_not_duplicated_in_amplification_gate(self):
        candidate = make_result()
        candidate["backends"]["native"]["cases"]["create_write.4096"]["p95_us"] = 999999.0
        result = evaluator.evaluate(CONTRACT, candidate)
        self.assertEqual("PASS", result["status"], result["errors"])

    def test_contract_freezes_one_commit_and_one_pull_for_one_mebibyte(self):
        rules = {rule["id"]: rule for rule in CONTRACT["cases"]}
        create = rules["create_write.1048576"]["maximum_per_user_operation"]
        peer = rules["peer_first_read.1048576"]["maximum_per_user_operation"]
        self.assertEqual(1, create["meta_rpc"]["commit"])
        self.assertEqual(1, create["fuse_callbacks"]["write"])
        self.assertEqual(1, peer["meta_rpc"]["lookup"])
        self.assertEqual(1, peer["peer_rpc"]["pull_block"])


if __name__ == "__main__":
    unittest.main()
