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
REQUIRED_LEDGER = {
    field: 0 for field in CONTRACT["required_path_ledger_fields"]
}


def unified_runtime_candidate() -> dict:
    candidate = copy.deepcopy(BASELINE)
    candidate["document_role"] = "candidate"
    existing_ids = {case["id"] for case in candidate["cases"]}
    for rule in CONTRACT["cases"]:
        if rule["id"] in existing_ids:
            continue
        ledger = copy.deepcopy(REQUIRED_LEDGER)
        ledger["entry_worker_rpc"] = 1 if rule.get("entrypoint") == "kv" else 0
        ledger["node_meta_resolve"] = 1 if "ResolveObject" in " ".join(rule["foreground_calls"]) else 0
        ledger["node_meta_commit"] = 1 if "CommitVersion" in " ".join(rule["foreground_calls"]) else 0
        ledger["node_peer_pull_count"] = 1 if "PullBlock" in " ".join(rule["foreground_calls"]) else 0
        ledger["node_peer_pull_bytes"] = 65536 if rule["id"] == "image.lazy_first_range.65536" else 0
        ledger["current_cache_hit"] = 1 if "node_hot" in rule["id"] or "second_same_range" in rule["id"] else 0
        ledger["current_cache_miss"] = 1 if "first_range" in rule["id"] else 0
        ledger["payload_full_copy"] = rule["minimum_payload_copies"]
        ledger["payload_allocation"] = rule["minimum_payload_allocations"]
        row = {
            "id": rule["id"],
            "correctness": True,
            "samples": 30,
            "p50_ns": 100000.0,
            "lower_bound_p50_ns": 90000.0,
            "comparator_p50_ns": 200000.0,
            "rpc": rule["minimum_rpc"],
            "payload_copies": rule["minimum_payload_copies"],
            "payload_allocations": rule["minimum_payload_allocations"],
            "unattributed_fraction": 0.0,
            "path_ledger": ledger,
        }
        if rule["path_class"] == "architecture_penalty":
            row["architecture_penalty"] = {
                "reason": "首读必须解析 owner 并拉取远端 range，随后同 range 读应走本地复用。",
                "following_hot_read_p50_ns": 80000.0,
            }
        candidate["cases"].append(row)
    return candidate


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

    def test_unified_runtime_candidate_requires_path_ledger(self):
        candidate = unified_runtime_candidate()
        target = next(case for case in candidate["cases"] if case["id"] == "fs.read.node_hot.4096")
        del target["path_ledger"]
        result = evaluate_whitebox.evaluate(CONTRACT, BASELINE, candidate)
        self.assertEqual("FAIL", result["status"])
        self.assertTrue(any("missing path_ledger" in item for item in result["errors"]))

    def test_process_internal_entry_cannot_use_worker_rpc(self):
        candidate = unified_runtime_candidate()
        target = next(case for case in candidate["cases"] if case["id"] == "fs.create.4096")
        target["path_ledger"]["entry_worker_rpc"] = 1
        result = evaluate_whitebox.evaluate(CONTRACT, BASELINE, candidate)
        self.assertEqual("FAIL", result["status"])
        self.assertTrue(any("used Worker RPC" in item for item in result["errors"]))

    def test_image_second_same_range_cannot_pull_peer_payload(self):
        candidate = unified_runtime_candidate()
        target = next(
            case for case in candidate["cases"] if case["id"] == "image.lazy_second_same_range.65536"
        )
        target["path_ledger"]["node_peer_pull_bytes"] = 4096
        result = evaluate_whitebox.evaluate(CONTRACT, BASELINE, candidate)
        self.assertEqual("FAIL", result["status"])
        self.assertTrue(any("pulled peer payload again" in item for item in result["errors"]))

    def test_candidate_rpc_and_copy_fields_cannot_be_null(self):
        candidate = unified_runtime_candidate()
        target = next(case for case in candidate["cases"] if case["id"] == "fs.read.node_hot.4096")
        target["payload_copies"] = None
        target["path_ledger"]["payload_full_copy"] = None
        result = evaluate_whitebox.evaluate(CONTRACT, BASELINE, candidate)
        self.assertEqual("FAIL", result["status"])
        self.assertTrue(any("payload_copies=None" in item for item in result["errors"]))
        self.assertTrue(any("payload_full_copy" in item for item in result["errors"]))

    def test_partial_candidate_validates_only_submitted_piercing_cases(self):
        candidate = copy.deepcopy(BASELINE)
        candidate["document_role"] = "candidate_partial"
        rule = next(case for case in CONTRACT["cases"] if case["id"] == "fs.read.node_hot.4096")
        ledger = copy.deepcopy(REQUIRED_LEDGER)
        ledger["current_cache_hit"] = 1
        ledger["payload_full_copy"] = 1
        candidate["cases"] = [
            {
                "id": rule["id"],
                "correctness": True,
                "samples": 30,
                "p50_ns": 250000.0,
                "lower_bound_p50_ns": 100000.0,
                "comparator_p50_ns": 120000.0,
                "rpc": rule["minimum_rpc"],
                "payload_copies": rule["minimum_payload_copies"],
                "payload_allocations": rule["minimum_payload_allocations"],
                "unattributed_fraction": 0.0,
                "path_ledger": ledger,
            }
        ]

        result = evaluate_whitebox.evaluate(CONTRACT, BASELINE, candidate)

        self.assertEqual("PASS", result["status"], result["errors"])
        self.assertTrue(any("candidate_partial" in item for item in result["warnings"]))
        self.assertEqual("partial_structural", result["rows"][0]["contract_mode"])

    def test_partial_candidate_still_enforces_path_ledger(self):
        candidate = copy.deepcopy(BASELINE)
        candidate["document_role"] = "candidate_partial"
        rule = next(case for case in CONTRACT["cases"] if case["id"] == "fs.read.node_hot.4096")
        candidate["cases"] = [
            {
                "id": rule["id"],
                "correctness": True,
                "samples": 30,
                "p50_ns": 250000.0,
                "lower_bound_p50_ns": 100000.0,
                "comparator_p50_ns": 120000.0,
                "rpc": rule["minimum_rpc"],
                "payload_copies": rule["minimum_payload_copies"],
                "payload_allocations": rule["minimum_payload_allocations"],
                "unattributed_fraction": 0.0,
                "path_ledger": {
                    **copy.deepcopy(REQUIRED_LEDGER),
                    "entry_worker_rpc": 1,
                },
            }
        ]

        result = evaluate_whitebox.evaluate(CONTRACT, BASELINE, candidate)

        self.assertEqual("FAIL", result["status"])
        self.assertTrue(any("used Worker RPC" in item for item in result["errors"]))


if __name__ == "__main__":
    unittest.main()
