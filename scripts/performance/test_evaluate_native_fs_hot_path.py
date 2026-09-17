#!/usr/bin/env python3

import importlib.util
import unittest
from pathlib import Path


MODULE_PATH = Path(__file__).with_name("evaluate_native_fs_hot_path.py")
SPEC = importlib.util.spec_from_file_location("evaluate_native_fs_hot_path", MODULE_PATH)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)


def result(run_id: str, *, p50: float = 100.0, p95: float = 120.0, whitebox=None):
    case = {
        "p50_us": p50,
        "p95_us": p95,
        "whitebox": whitebox or {},
    }
    reference = {"p50_us": 100.0, "p95_us": 120.0}
    return {
        "run_id": run_id,
        "lanes": {
            "memory": {
                "backends": {
                    "dms": {"cases": {"workspace.local_hot": case, "workspace.peer_repeat": case}},
                    "moosefs": {
                        "cases": {
                            "workspace.local_hot": reference,
                            "workspace.peer_repeat": reference,
                        }
                    },
                }
            }
        },
    }


class HotPathEvaluatorTest(unittest.TestCase):
    def setUp(self):
        self.contract = {
            "schema": "dms.native-fs-hot-path-contract.v1",
            "required_independent_runs": 2,
            "cases": ["workspace.local_hot", "workspace.peer_repeat"],
            "latency": {
                "maximum_dms_to_moosefs_p50_ratio": 1.10,
                "maximum_dms_to_moosefs_p95_ratio": 1.10,
            },
            "rpc": {
                "allowed_background_methods": ["Heartbeat"],
                "forbidden_methods": ["GetFilesystemXattr", "ReleaseFilesystemLockOwner"],
            },
        }

    def test_accepts_two_independent_quiet_hot_runs(self):
        evaluation = MODULE.evaluate(self.contract, [result("one"), result("two")])
        self.assertEqual(evaluation["status"], "PASS")

    def test_rejects_foreground_meta_rpc(self):
        metric = {
            "A:dms_rpc_client_requests_total{method=GetFilesystemXattr,result=ok,service=FilesystemMetadataService}": 1.0
        }
        evaluation = MODULE.evaluate(
            self.contract,
            [result("one", whitebox=metric), result("two", whitebox=metric)],
        )
        self.assertEqual(evaluation["status"], "FAIL")
        self.assertTrue(any("forbidden hot-path RPC" in error for error in evaluation["errors"]))

    def test_rejects_duplicate_or_slow_runs(self):
        evaluation = MODULE.evaluate(
            self.contract,
            [result("same", p50=111.0), result("same", p50=111.0)],
        )
        self.assertEqual(evaluation["status"], "FAIL")
        self.assertTrue(any("duplicate run_id" in error for error in evaluation["errors"]))
        self.assertTrue(any("p50_us ratio" in error for error in evaluation["errors"]))


if __name__ == "__main__":
    unittest.main()
