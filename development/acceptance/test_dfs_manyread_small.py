"""Guard tests for dfs_manyread_small evidence helpers."""
from __future__ import annotations

import importlib.util
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("dfs_manyread_small", Path(__file__).with_name("dfs_manyread_small.py"))
dfs_manyread_small = importlib.util.module_from_spec(spec)
spec.loader.exec_module(dfs_manyread_small)


def good_io(operation: str = "seq-read", barrier: str = "close") -> dict[str, object]:
    return {
        "operation": operation,
        "file_bytes": dfs_manyread_small.DATA_BYTES,
        "io_bytes": dfs_manyread_small.DATA_BYTES,
        "block_bytes": dfs_manyread_small.BLOCK_BYTES,
        "concurrency": dfs_manyread_small.CONCURRENCY,
        "barrier": barrier,
        "pattern_byte": dfs_manyread_small.PATTERN_BYTE,
        "operations": dfs_manyread_small.DATA_BYTES // dfs_manyread_small.BLOCK_BYTES,
        "cache_requested": "unobserved",
        "content_ok": True,
        "residency_observed": False,
        "wall_ns": 1000,
    }


def good_manifest() -> dict[str, object]:
    return {
        "status": "DATA_RECORDED",
        "product_source_commit": dfs_manyread_small.PRODUCT_SOURCE_COMMIT,
        "source6d": dfs_manyread_small.PRODUCT_SOURCE_COMMIT[:7],
        "compiler_input_map": dfs_manyread_small.COMPILER_INPUT_MAP,
        "map66": dfs_manyread_small.COMPILER_INPUT_MAP[:8],
        "io_tool": {"sha256": dfs_manyread_small.IO_TOOL_SHA256},
        "payload": {
            "relative_dir": "dfs-manyread-small-test",
            "name": dfs_manyread_small.PAYLOAD_NAME,
            "bytes": dfs_manyread_small.DATA_BYTES,
            "pattern_byte": dfs_manyread_small.PATTERN_BYTE,
            "sha256": dfs_manyread_small.expected_payload_sha(),
        },
        "write": {"rc": 0, "status": "PASS", "result": good_io("seq-write", "fdatasync"), "verify": {"status": "PASS"}},
        "content_verify": {"status": "PASS", "bytes": dfs_manyread_small.DATA_BYTES, "sha256": dfs_manyread_small.expected_payload_sha()},
        "dir_fsync": True,
        "parent_dir_fsync": True,
    }


class DfsManyReadSmallGuardTests(unittest.TestCase):
    def test_missing_or_bad_manifest_is_rejected(self) -> None:
        with self.assertRaises(ValueError):
            dfs_manyread_small.validate_manifest_shape({})
        bad = good_manifest()
        bad["status"] = "FAIL"
        with self.assertRaises(ValueError):
            dfs_manyread_small.validate_manifest_shape(bad)

    def test_manifest_shape_and_sha_are_checked(self) -> None:
        dfs_manyread_small.validate_manifest_shape(good_manifest())
        wrong_shape = good_manifest()
        wrong_shape["payload"]["bytes"] = 1
        with self.assertRaises(ValueError):
            dfs_manyread_small.validate_manifest_shape(wrong_shape)
        wrong_sha = good_manifest()
        wrong_sha["payload"]["sha256"] = "0" * 64
        with self.assertRaises(ValueError):
            dfs_manyread_small.validate_manifest_shape(wrong_sha)

    def test_io_result_rejects_failed_c_or_short_read(self) -> None:
        dfs_manyread_small.validate_io_result(good_io(), "seq-read", "close")
        bad = good_io()
        bad["content_ok"] = False
        with self.assertRaises(ValueError):
            dfs_manyread_small.validate_io_result(bad, "seq-read", "close")
        short = good_io()
        short["io_bytes"] = dfs_manyread_small.DATA_BYTES - 1
        with self.assertRaises(ValueError):
            dfs_manyread_small.validate_io_result(short, "seq-read", "close")

    def test_missing_rounds_do_not_record_data(self) -> None:
        one_round = [{"samples": [{"status": "PASS", "verify": {"status": "PASS"}}]}]
        self.assertEqual(dfs_manyread_small.read_status(one_round), "FAIL")

    def test_output_existing_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp) / "out"
            out.mkdir()
            with self.assertRaises(ValueError):
                dfs_manyread_small.validate_output_path(out)

    def test_existing_output_sentinel_is_not_overwritten(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp) / "out"
            out.mkdir()
            sentinel = out / "summary.json"
            sentinel.write_text("sentinel", encoding="utf-8")

            class Args:
                dfs_root = "/missing-dfs"
                io_tool = "/missing-io"
                output = str(out)

            result = dfs_manyread_small.writer(Args())
            self.assertEqual(result["status"], "BLOCKED")
            self.assertEqual(sentinel.read_text(encoding="utf-8"), "sentinel")

    def test_foreign_fuse_mount_is_rejected(self) -> None:
        with self.assertRaises(ValueError):
            dfs_manyread_small.require_dfs_mount({"fstype": "fuse", "source": "some-dfs", "target": "/mnt/dfs"}, Path("/mnt/dfs"))
        with self.assertRaises(ValueError):
            dfs_manyread_small.require_dfs_mount({"fstype": "fuse", "source": "afs-dfs", "target": "/mnt/other"}, Path("/mnt/dfs"))
        dfs_manyread_small.require_dfs_mount({"fstype": "fuse", "source": "afs-dfs", "target": "/mnt/dfs"}, Path("/mnt/dfs"))


if __name__ == "__main__":
    unittest.main()
