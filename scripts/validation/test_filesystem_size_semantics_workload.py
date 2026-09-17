from __future__ import annotations

import os
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPT_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(SCRIPT_DIR))

import filesystem_size_semantics_workload as workload


class FilesystemSizeSemanticsWorkloadTests(unittest.TestCase):
    def test_sparse_zero_checker_accepts_only_zero_bytes(self) -> None:
        workload.assert_zeroes(b"\0" * 16, "zero case")
        with self.assertRaises(AssertionError):
            workload.assert_zeroes(b"\0x\0", "non-zero case")

    def test_pwrite_beyond_eof_uses_posix_sparse_read_semantics(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            mount = Path(directory)
            metrics = workload.MetricProbe(None)
            result = workload.pwrite_beyond_eof_sparse(mount, mount, metrics)
            self.assertEqual(result["operation"], "pwrite_beyond_eof_sparse")
            self.assertEqual((mount / "pwrite-beyond-eof.txt").stat().st_size, workload.SPARSE_OFFSET + 4)
            with (mount / "pwrite-beyond-eof.txt").open("rb") as stream:
                stream.seek(4)
                self.assertEqual(stream.read(32), b"\0" * 32)
                stream.seek(workload.SPARSE_OFFSET)
                self.assertEqual(stream.read(4), b"tail")

    def test_o_trunc_clears_file_before_remote_read(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            mount = Path(directory)
            result = workload.open_o_trunc(mount, mount)
            self.assertEqual(result["operation"], "open_o_trunc")
            self.assertEqual((mount / "open-o-trunc.txt").read_bytes(), b"")

    def test_concurrent_o_append_keeps_each_record_once(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            mount = Path(directory)
            result = workload.concurrent_o_append(mount, mount)
            self.assertEqual(result["operation"], "concurrent_o_append")
            content = (mount / "append.txt").read_bytes().splitlines()
            self.assertEqual(len(content), 16)
            self.assertEqual(len(set(content)), 16)

    def test_recovery_check_is_immediate_not_eventual(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            mount = Path(directory)
            with self.assertRaises(FileNotFoundError):
                workload.verify_recovery(mount, "missing")
            (mount / "visibility.txt").write_bytes(b"dms-size:cross-node")
            result = workload.verify_recovery(mount, "dms-size:cross-node")
            self.assertEqual(result["schema"], workload.RECOVERY_SCHEMA)

    def test_run_contains_each_required_operation_once(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            mount = Path(directory)
            result = workload.run(mount, mount, None)
            operations = {check["operation"] for check in result["checks"]}
            self.assertEqual(workload.REQUIRED_OPERATIONS, operations)


if __name__ == "__main__":
    unittest.main()
