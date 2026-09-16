from pathlib import Path
import sys
import unittest


SCRIPT_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(SCRIPT_DIR))

from evaluate_repeated_fuse_performance import evaluate  # noqa: E402


def evaluation(*errors: str) -> dict:
    return {
        "schema": "dms.fuse-request-amplification-evaluation.v1",
        "status": "PASS" if not errors else "FAIL",
        "errors": list(errors),
        "rows": [],
    }


class RepeatedFusePerformanceEvaluatorTest(unittest.TestCase):
    def test_transient_latency_regression_does_not_fail_repeated_gate(self) -> None:
        result = evaluate(
            [
                (
                    Path("first.json"),
                    evaluation(
                        "peer_hot_read.4096: native p95_us regression ratio 1.080 exceeds 1.050"
                    ),
                ),
                (Path("second.json"), evaluation()),
            ]
        )

        self.assertEqual("PASS", result["status"])
        self.assertEqual([], result["persistent_latency_regressions"])
        self.assertEqual(
            [
                {
                    "case": "peer_hot_read.4096",
                    "percentile": "p95_us",
                    "failed_runs": ["first.json"],
                    "required_runs": 2,
                }
            ],
            result["transient_latency_regressions"],
        )

    def test_persistent_latency_regression_fails_repeated_gate(self) -> None:
        message = (
            "peer_hot_read.4096: native p50_us regression ratio 1.080 exceeds 1.050"
        )
        result = evaluate(
            [
                (Path("first.json"), evaluation(message)),
                (Path("second.json"), evaluation(message)),
            ]
        )

        self.assertEqual("FAIL", result["status"])
        self.assertEqual(
            "peer_hot_read.4096",
            result["persistent_latency_regressions"][0]["case"],
        )

    def test_non_latency_error_fails_if_any_pair_contains_it(self) -> None:
        result = evaluate(
            [
                (
                    Path("first.json"),
                    evaluation("peer_hot_read.4096: correctness not proven"),
                ),
                (Path("second.json"), evaluation()),
            ]
        )

        self.assertEqual("FAIL", result["status"])
        self.assertIn("correctness not proven", result["errors"][0])

    def test_two_independent_pairs_are_required(self) -> None:
        result = evaluate([(Path("only.json"), evaluation())])

        self.assertEqual("FAIL", result["status"])
        self.assertEqual(
            ["at least 2 independent paired evaluations are required"],
            result["errors"],
        )

    def test_same_input_file_cannot_count_as_two_independent_pairs(self) -> None:
        result = evaluate(
            [
                (Path("same.json"), evaluation()),
                (Path("./same.json"), evaluation()),
            ]
        )

        self.assertEqual("FAIL", result["status"])
        self.assertIn(
            "independent paired evaluations must use distinct input files",
            result["errors"],
        )


if __name__ == "__main__":
    unittest.main()
