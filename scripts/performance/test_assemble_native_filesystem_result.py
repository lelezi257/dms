import tempfile
import unittest
from pathlib import Path

from scripts.performance.assemble_native_filesystem_result import (
    delta_metric,
    exact_average,
    parse_prometheus,
    percentile,
    static_payload_copy_stages,
)


class AssembleNativeFilesystemResultTests(unittest.TestCase):
    def test_parse_and_delta_prometheus_labels(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            before_path = Path(directory) / "before.prom"
            after_path = Path(directory) / "after.prom"
            before_path.write_text(
                'dms_rpc_client_requests_total{method="PullBlock",result="ok",service="PeerService"} 2\n',
                encoding="utf-8",
            )
            after_path.write_text(
                'dms_rpc_client_requests_total{method="PullBlock",result="ok",service="PeerService"} 5\n',
                encoding="utf-8",
            )
            self.assertEqual(
                delta_metric(
                    parse_prometheus(before_path),
                    parse_prometheus(after_path),
                    "dms_rpc_client_requests_total",
                    {"service": "PeerService", "method": "PullBlock"},
                ),
                3,
            )

    def test_percentile_matches_workload_rule(self) -> None:
        self.assertEqual(percentile([1, 2, 3, 4, 5], 0.5), 3)
        self.assertEqual(percentile([1, 2, 3, 4, 5], 0.95), 5)

    def test_exact_average_preserves_non_integral_measurement(self) -> None:
        self.assertEqual(exact_average(90, 30), 3)
        self.assertEqual(exact_average(4, 3), 1.333333)

    def test_payload_copy_stages_describe_path_depth_per_segment(self) -> None:
        self.assertEqual(static_payload_copy_stages("create_write.1048576"), 2)
        self.assertEqual(static_payload_copy_stages("local_hot_read.1048576"), 1)
        self.assertEqual(static_payload_copy_stages("peer_first_read.1048576"), 3)
        self.assertEqual(static_payload_copy_stages("metadata_hot.4096"), 0)
        self.assertEqual(static_payload_copy_stages("open_close.4096"), 0)
        self.assertEqual(static_payload_copy_stages("readdir.root"), 0)


if __name__ == "__main__":
    unittest.main()
