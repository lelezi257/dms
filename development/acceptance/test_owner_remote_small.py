"""Guard tests for owner_remote_small diagnostic evidence shape."""
from __future__ import annotations

import importlib.util
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("owner_remote_small", Path(__file__).with_name("owner_remote_small.py"))
owner_remote_small = importlib.util.module_from_spec(spec)
spec.loader.exec_module(owner_remote_small)


def read_result(**overrides: object) -> dict[str, object]:
    record: dict[str, object] = {
        "operation": "seq-read",
        "file_bytes": owner_remote_small.READ_BYTES,
        "io_bytes": owner_remote_small.READ_BYTES,
        "block_bytes": owner_remote_small.READ_BLOCK_BYTES,
        "concurrency": owner_remote_small.READ_CONCURRENCY,
        "barrier": "close",
        "pattern_byte": owner_remote_small.READ_PATTERN_BYTE,
        "operations": owner_remote_small.READ_BYTES // owner_remote_small.READ_BLOCK_BYTES,
        "wall_ns": 1000,
        "residency_observed": False,
        "cache_requested": "unobserved",
        "content_ok": True,
    }
    record.update(overrides)
    return record


class OwnerRemoteSmallGuardTests(unittest.TestCase):
    def test_validate_io_result_rejects_wrong_content_and_short_read(self) -> None:
        owner_remote_small.validate_io_result(read_result())
        with self.assertRaises(ValueError):
            owner_remote_small.validate_io_result(read_result(content_ok=False))
        with self.assertRaises(ValueError):
            owner_remote_small.validate_io_result(read_result(io_bytes=owner_remote_small.READ_BYTES - 1))
        with self.assertRaises(ValueError):
            owner_remote_small.validate_io_result(read_result(cache_requested="hot"))

    def test_existing_output_directory_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            existing = Path(tmp) / "out"
            existing.mkdir()
            with self.assertRaises(ValueError):
                owner_remote_small.validate_output_path(existing)

    def test_failed_cleanup_prevents_data_recorded(self) -> None:
        rounds = [
            {
                "samples": [
                    {"status": "PASS", "cleanup": {"status": "PASS"}},
                    {"status": "PASS", "cleanup": {"status": "FAIL", "left_in_place": "/tmp/sample"}},
                ]
            }
        ]
        self.assertEqual(owner_remote_small.compute_status(rounds, [], "delete"), "FAIL")

    def test_failed_syscall_prevents_data_recorded(self) -> None:
        rounds = [
            {
                "samples": [
                    {"status": "PASS", "cleanup": {"status": "PASS"}},
                    {"status": "FAIL", "cleanup": {"status": "PASS"}, "error": "OSError('unlink')"},
                ]
            }
        ]
        self.assertEqual(owner_remote_small.compute_status(rounds, [], "delete"), "FAIL")


    def test_missing_rounds_do_not_record_data(self) -> None:
        one_round = [{"samples": [{"status": "PASS", "cleanup": {"status": "PASS"}}, {"status": "PASS", "cleanup": {"status": "PASS"}}]}]
        self.assertEqual(owner_remote_small.delete_status(one_round), "FAIL")
        self.assertEqual(owner_remote_small.read_status(one_round), "FAIL")


    def test_all_requires_read_rounds_before_data_recorded(self) -> None:
        rounds = [
            {"samples": [{"status": "PASS", "cleanup": {"status": "PASS"}}, {"status": "PASS", "cleanup": {"status": "PASS"}}]}
            for _ in range(owner_remote_small.WARMUP_ROUNDS + owner_remote_small.MEASUREMENT_ROUNDS)
        ]
        self.assertEqual(owner_remote_small.delete_status(rounds), "DATA_RECORDED")
        self.assertEqual(owner_remote_small.compute_status(rounds, [], "all"), "FAIL")

    def test_all_missing_read_inputs_is_rejected(self) -> None:
        with self.assertRaises(ValueError):
            owner_remote_small.require_read_inputs_for_case("all", None)

    def test_read_missing_read_inputs_is_rejected(self) -> None:
        with self.assertRaises(ValueError):
            owner_remote_small.require_read_inputs_for_case("read", None)
        owner_remote_small.require_read_inputs_for_case("delete", None)


if __name__ == "__main__":
    unittest.main()
