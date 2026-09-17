import importlib.util
import json
from pathlib import Path
import tempfile
import unittest


MODULE_PATH = Path(__file__).with_name("evaluate_native_vs_moosefs.py")
SPEC = importlib.util.spec_from_file_location("evaluate_native_vs_moosefs", MODULE_PATH)
assert SPEC and SPEC.loader
evaluator = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(evaluator)


def case(rounds: int) -> dict:
    return {
        "correctness": True,
        "p50_us": 10.0,
        "p95_us": 12.0,
        "p99_us": 15.0,
        "throughput_mib_s": 100.0,
        "rounds": [{"round": f"round-{index}"} for index in range(rounds)],
        "resources": {
            "network": {"rx_bytes": 1, "tx_bytes": 1},
            "cpu_ticks": 1,
            "context_switches": 1,
            "rss_peak_bytes": 1,
        },
        "copy_evidence": {"type": "code_path_model", "path": "x"},
        "whitebox": {},
    }


class EvaluatorTest(unittest.TestCase):
    def setUp(self):
        self.contract = {
            "schema": "dms.native-vs-moosefs-contract.v1",
            "evidence": {"minimum_paired_memory_rounds": 5},
            "workload": {"required_cases": ["one"]},
        }
        self.result = {
            "schema": "dms.native-vs-moosefs-result.v1",
            "same_environment": True,
            "lanes": {
                "memory": {
                    "media": "tmpfs",
                    "rounds": 5,
                    "backends": {
                        "dms": {"correctness": True, "cases": {"one": case(5)}},
                        "moosefs": {"correctness": True, "cases": {"one": case(5)}},
                    },
                },
                "disk": {
                    "media": "vm_virtual_disk",
                    "rounds": 1,
                    "backends": {
                        "dms": {"correctness": True, "cases": {"one": case(1)}},
                        "moosefs": {"correctness": True, "cases": {"one": case(1)}},
                    },
                },
            },
            "preview_verdict": {"status": "READY", "checks": [{"passed": True}]},
            "analysis": {
                "architecture_inherent": [{"finding": "cold miss"}],
                "implementation_findings": [{"finding": "hot path"}],
            },
        }

    def test_complete_evidence_passes(self):
        evaluation = evaluator.evaluate(self.contract, self.result)
        self.assertEqual(evaluation["status"], "PASS")

    def test_missing_rounds_fail(self):
        self.result["lanes"]["memory"]["rounds"] = 4
        evaluation = evaluator.evaluate(self.contract, self.result)
        self.assertEqual(evaluation["status"], "FAIL")
        self.assertTrue(any("paired rounds" in error for error in evaluation["errors"]))


if __name__ == "__main__":
    unittest.main()
