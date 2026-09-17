import importlib.util
import json
from pathlib import Path
import tempfile
import unittest


MODULE_PATH = Path(__file__).with_name("assemble_native_vs_moosefs_result.py")
SPEC = importlib.util.spec_from_file_location("assemble_native_vs_moosefs_result", MODULE_PATH)
assert SPEC and SPEC.loader
assembler = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(assembler)


class AssembleTest(unittest.TestCase):
    def test_moosefs_operation_snapshot_is_summed_across_clients(self):
        payload = {
            "dataset": {
                "operations": [
                    {"stats_current_hour": {"lookup": 2, "read": 3}},
                    {"stats_current_hour": {"lookup": 5, "read": 7}},
                ]
            }
        }
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "mfs.json"
            path.write_text(json.dumps(payload), encoding="utf-8")
            self.assertEqual(
                assembler.moosefs_operations(path),
                {"lookup": 7.0, "read": 10.0},
            )

    def test_copy_models_cover_every_workload_case(self):
        cases = set(assembler.PHASE_CASE.values())
        self.assertEqual(set(assembler.COPY_MODELS["dms"]), cases)
        self.assertEqual(set(assembler.COPY_MODELS["moosefs"]), cases)

    def test_dms_rpc_counts_only_aggregates_client_requests(self):
        whitebox = {
            'A:dms_rpc_client_requests_total{method=PullBlock,service=PeerService}': 2,
            'B:dms_rpc_client_requests_total{method=PullBlock,service=PeerService}': 3,
            'A:dms_rpc_client_duration_seconds_sum{method=PullBlock,service=PeerService}': 0.4,
            'C:dms_rpc_server_requests_total{method=PullBlock,service=PeerService}': 5,
        }
        self.assertEqual(assembler.dms_rpc_counts(whitebox), {"PullBlock": 5})


if __name__ == "__main__":
    unittest.main()
