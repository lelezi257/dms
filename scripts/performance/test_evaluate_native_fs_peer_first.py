import importlib.util
from pathlib import Path
import unittest


MODULE_PATH = Path(__file__).with_name("evaluate_native_fs_peer_first.py")
SPEC = importlib.util.spec_from_file_location("evaluate_native_fs_peer_first", MODULE_PATH)
assert SPEC and SPEC.loader
evaluator = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(evaluator)


def case(value: float, field: str, rounds: int = 5, whitebox=None):
    rows = []
    for index in range(rounds):
        row = {"round": f"round-{index}", "p50_us": 100.0, "throughput_mib_s": 100.0}
        row[field] = value
        rows.append(row)
    return {
        "correctness": True,
        "p50_us": 100.0,
        "throughput_mib_s": 100.0,
        "rounds": rows,
        "whitebox": whitebox or {},
    }


class EvaluatorTest(unittest.TestCase):
    def setUp(self):
        self.contract = {
            "schema": "dms.native-fs-peer-first-contract.v1",
            "required_independent_runs": 2,
            "minimum_paired_memory_rounds": 5,
            "thresholds": {
                "workspace_peer_first_p50_ratio_max": 1.15,
                "sequential_512m_peer_first_throughput_ratio_min": 0.90,
                "workspace_peer_repeat_p50_ratio_max": 1.10,
                "sequential_512m_peer_repeat_throughput_ratio_min": 0.90,
                "workspace_local_hot_p50_ratio_max": 1.10,
            },
            "rpc_contract": {
                "large_peer_first_legacy_pull_block_max": 0,
                "large_peer_first_pull_streams_per_round_max": 1,
                "large_peer_first_report_replicas_per_round_max": 1,
                "foreground_synchronous_report_replicas_max": 0,
            },
            "required_fault_contracts": ["checksum_mismatch_rejected"],
        }
        self.result = self.make_result("run-1")

    def make_result(self, run_id):
        dms = {
            "workspace.peer_first": case(110.0, "p50_us"),
            "sequential_512m.peer_first": case(
                95.0,
                "throughput_mib_s",
                whitebox={
                    'B:dms_rpc_client_requests_total{method=PullBlocks,result=ok,service=PeerService}': 5,
                    'B:dms_rpc_client_requests_total{method=ReportReplicas,result=ok,service=MetadataService}': 5,
                },
            ),
            "workspace.peer_repeat": case(105.0, "p50_us"),
            "sequential_512m.peer_repeat": case(100.0, "throughput_mib_s"),
            "workspace.local_hot": case(100.0, "p50_us"),
        }
        mfs = {
            "workspace.peer_first": case(100.0, "p50_us"),
            "sequential_512m.peer_first": case(100.0, "throughput_mib_s"),
            "workspace.peer_repeat": case(100.0, "p50_us"),
            "sequential_512m.peer_repeat": case(100.0, "throughput_mib_s"),
            "workspace.local_hot": case(100.0, "p50_us"),
        }
        return {
            "schema": "dms.native-vs-moosefs-result.v1",
            "run_id": run_id,
            "same_environment": True,
            "environment": {
                "source_sha": "abc",
                "resolved_hashes": {"dms_node": "node-hash", "dms_meta": "meta-hash"},
            },
            "lanes": {
                "memory": {
                    "media": "tmpfs",
                    "rounds": 5,
                    "backends": {
                        "dms": {"correctness": True, "cases": dms},
                        "moosefs": {"correctness": True, "cases": mfs},
                    },
                }
            },
            "p4_contracts": {
                "foreground_synchronous_report_replicas": 0,
                "fault_contracts": {"checksum_mismatch_rejected": True},
            },
        }

    def test_two_complete_runs_pass(self):
        result = evaluator.evaluate(
            self.contract,
            [self.result, self.make_result("run-2")],
        )
        self.assertEqual("PASS", result["status"])

    def test_legacy_per_block_rpc_fails(self):
        large = self.result["lanes"]["memory"]["backends"]["dms"]["cases"][
            "sequential_512m.peer_first"
        ]
        large["whitebox"] = {
            'B:dms_rpc_client_requests_total{method=PullBlock,result=ok,service=PeerService}': 512
        }
        result = evaluator.evaluate(
            self.contract,
            [self.result, self.make_result("run-2")],
        )
        self.assertEqual("FAIL", result["status"])
        self.assertTrue(any("legacy PullBlock" in error for error in result["errors"]))

    def test_missing_fault_contract_fails(self):
        self.result["p4_contracts"]["fault_contracts"] = {}
        result = evaluator.evaluate(
            self.contract,
            [self.result, self.make_result("run-2")],
        )
        self.assertEqual("FAIL", result["status"])
        self.assertTrue(any("fault contract" in error for error in result["errors"]))

    def test_per_block_replica_reports_fail(self):
        large = self.result["lanes"]["memory"]["backends"]["dms"]["cases"][
            "sequential_512m.peer_first"
        ]
        large["whitebox"][
            'B:dms_rpc_client_requests_total{method=ReportReplicas,result=ok,service=MetadataService}'
        ] = 512
        result = evaluator.evaluate(
            self.contract,
            [self.result, self.make_result("run-2")],
        )
        self.assertEqual("FAIL", result["status"])
        self.assertTrue(any("ReportReplicas count" in error for error in result["errors"]))

    def test_threshold_regression_fails(self):
        peer = self.result["lanes"]["memory"]["backends"]["dms"]["cases"][
            "workspace.peer_first"
        ]
        for row in peer["rounds"]:
            row["p50_us"] = 130.0
        result = evaluator.evaluate(
            self.contract,
            [self.result, self.make_result("run-2")],
        )
        self.assertEqual("FAIL", result["status"])
        self.assertTrue(any("workspace.peer_first" in error for error in result["errors"]))

    def test_different_binaries_fail(self):
        second = self.make_result("run-2")
        second["environment"]["resolved_hashes"]["dms_node"] = "other-node-hash"
        result = evaluator.evaluate(self.contract, [self.result, second])
        self.assertEqual("FAIL", result["status"])
        self.assertTrue(any("identical DMS binaries" in error for error in result["errors"]))


if __name__ == "__main__":
    unittest.main()
