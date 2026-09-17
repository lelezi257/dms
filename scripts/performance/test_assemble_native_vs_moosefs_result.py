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

    def test_p4_contract_receipt_is_normalized(self):
        receipt = {
            "schema": "dms.native-fs-peer-first-contract-receipt.v1",
            "foreground_synchronous_report_replicas": 0,
            "fault_contracts": {"checksum_mismatch_rejected": True},
            "mechanism_contracts": {"multi_block_read_uses_one_peer_stream": True},
        }
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "p4-contracts.json"
            path.write_text(json.dumps(receipt), encoding="utf-8")
            self.assertEqual(
                assembler.load_p4_contract_receipt(path),
                {
                    "foreground_synchronous_report_replicas": 0,
                    "fault_contracts": {"checksum_mismatch_rejected": True},
                    "mechanism_contracts": {
                        "multi_block_read_uses_one_peer_stream": True
                    },
                },
            )

    def test_p4_contract_receipt_rejects_unknown_schema(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "p4-contracts.json"
            path.write_text(json.dumps({"schema": "unknown"}), encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "invalid P4 contract receipt"):
                assembler.load_p4_contract_receipt(path)


if __name__ == "__main__":
    unittest.main()
