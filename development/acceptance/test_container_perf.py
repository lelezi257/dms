"""Evidence guards for container workspace performance diagnostics."""
from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import tempfile
import unittest


spec = importlib.util.spec_from_file_location("perf", Path(__file__).with_name("container-workspace-perf-linux.py"))
perf = importlib.util.module_from_spec(spec)
spec.loader.exec_module(perf)


def io_result(operation: str = "seq-read") -> dict[str, object]:
    return {
        "operation": operation,
        "file_bytes": perf.DATA_BYTES,
        "io_bytes": perf.DATA_BYTES,
        "block_bytes": perf.BLOCK_BYTES,
        "concurrency": 1,
        "barrier": "close" if operation == "seq-read" else "fsync",
        "pattern_byte": perf.PATTERN_BYTE,
        "operations": perf.DATA_BYTES // perf.BLOCK_BYTES,
        "wall_ns": 1000,
        "client_cpu_ns": 900,
        "barrier_ns": 10,
        "p50_ns": 10,
        "p95_ns": 20,
        "p99_ns": 30,
        "content_ok": True,
        "cache_requested": "unobserved",
    }


def metadata_result() -> dict[str, object]:
    return {
        "files": 1000,
        "file_bytes": 4096,
        "concurrency": 1,
        "path_form": "absolute",
        "phases": [
            {"name": name, "wall_ns": 100 + i, "client_cpu_ns": 50, "operations": 1}
            for i, name in enumerate(["create_write_close", "stat", "read_close", "readdir", "rename", "unlink"])
        ],
    }


def sample(target: str) -> dict[str, object]:
    return {
        "target": target,
        "write": {"result": io_result("seq-write")},
        "read": {"result": io_result("seq-read")},
        "metadata": {"result": metadata_result()},
    }


class ContainerPerfEvidenceTests(unittest.TestCase):
    def test_validate_summary_rejects_missing_pair(self) -> None:
        summary = {
            "status": "DATA_RECORDED",
            "cohorts": [
                {
                    "rounds": [
                        {"samples": [sample("experiment")]}
                        for _ in range(perf.WARMUP_ROUNDS + perf.MEASUREMENT_ROUNDS)
                    ]
                }
            ],
        }
        with self.assertRaises(ValueError):
            perf.validate_summary(summary)

    def test_validate_result_rejects_wrong_content_and_timing(self) -> None:
        bad_content = io_result()
        bad_content["content_ok"] = False
        with self.assertRaises(ValueError):
            perf.verify_io_result(bad_content, "seq-read", "close")
        bad_timing = io_result()
        bad_timing["p95_ns"] = 1
        with self.assertRaises(ValueError):
            perf.verify_io_result(bad_timing, "seq-read", "close")
        bad_cache = io_result()
        bad_cache["cache_requested"] = "hot"
        with self.assertRaises(ValueError):
            perf.verify_io_result(bad_cache, "seq-read", "close")

    def test_validate_pair_requires_six_metadata_phases(self) -> None:
        record = {"samples": [sample("experiment"), sample("reference")]}
        perf.validate_pair(record)
        record["samples"][0]["metadata"]["result"]["phases"] = []
        with self.assertRaises(ValueError):
            perf.validate_pair(record)

    def test_newest_stdout_for_argv_matches_exact_argv(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            argv = ["--root", "/tmp/state", "exec", "c", "/io"]
            wrong = root / "command-0001.command.json"
            wrong.write_text(json.dumps({"argv": argv + ["extra"]}))
            (root / "command-0001.stdout").write_text("{}")
            (root / "command-0001.exit.json").write_text(json.dumps({"success": True, "code": 0}))
            right = root / "command-0002.command.json"
            right.write_text(json.dumps({"argv": argv}))
            (root / "command-0002.stdout").write_text(json.dumps({"ok": True}))
            (root / "command-0002.exit.json").write_text(json.dumps({"success": True, "code": 0}))
            found = perf.newest_stdout_for_argv(root, argv, set())
            self.assertEqual(found["stdout"], {"ok": True})

    def test_newest_stdout_rejects_absent_exact_argv(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "command-0001.command.json").write_text(json.dumps({"argv": ["nearly"]}))
            with self.assertRaises(ValueError):
                perf.newest_stdout_for_argv(root, ["exact"], set())


if __name__ == "__main__":
    unittest.main()
