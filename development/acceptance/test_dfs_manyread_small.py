"""Guard tests for dfs_manyread_small evidence helpers."""
from __future__ import annotations

import importlib.util
import json
import os
import subprocess
import sys
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


    def test_start_event_rejects_wrong_reader_round_or_session(self) -> None:
        good = json.dumps({"event": "START", "reader_id": "B", "session_token": "s", "round": 1, "round_token": "s:round:1"})
        self.assertEqual(dfs_manyread_small.parse_start_event(good, 1, reader_id="B", session_token="s")["round_token"], "s:round:1")
        ack0 = json.dumps({"event": "ACK", "reader_id": "B", "session_token": "s", "round": 0})
        self.assertEqual(dfs_manyread_small.parse_ack_event(ack0, 0, reader_id="B", session_token="s")["event"], "ACK")
        ack = json.dumps({"event": "ACK", "reader_id": "B", "session_token": "s", "round": 1, "round_token": "s:round:1"})
        self.assertEqual(dfs_manyread_small.parse_ack_event(ack, 1, reader_id="B", session_token="s", round_token="s:round:1")["event"], "ACK")
        with self.assertRaises(ValueError):
            dfs_manyread_small.parse_ack_event(ack, 1, reader_id="B", session_token="s", round_token="other")
        with self.assertRaises(ValueError):
            dfs_manyread_small.parse_start_event(good, 2, reader_id="B", session_token="s")
        with self.assertRaises(ValueError):
            dfs_manyread_small.parse_start_event(good, 1, reader_id="C", session_token="s")
        with self.assertRaises(ValueError):
            dfs_manyread_small.parse_start_event(good, 1, reader_id="B", session_token="other")
        with self.assertRaises(ValueError):
            dfs_manyread_small.parse_start_event('{"event":"START","reader_id":"B","round":1}', 1, reader_id="B")

    def test_ready_pair_requires_exact_two_unique_readers(self) -> None:
        events = [
            {"event": "READY", "reader_id": "B", "session_token": "s", "round": 1},
            {"event": "READY", "reader_id": "C", "session_token": "s", "round": 1},
        ]
        self.assertEqual(dfs_manyread_small.validate_ready_pair(events, expected_round=1, session_token="s")["status"], "PASS")
        with self.assertRaises(ValueError):
            dfs_manyread_small.validate_ready_pair(events[:1], expected_round=1, session_token="s")
        with self.assertRaises(ValueError):
            dfs_manyread_small.validate_ready_pair([events[0], dict(events[0])], expected_round=1, session_token="s")
        with self.assertRaises(ValueError):
            dfs_manyread_small.validate_ready_pair([events[0], {**events[1], "round": 2}], expected_round=1, session_token="s")

    def test_json_line_reader_times_out_on_partial_line_and_keeps_buffer(self) -> None:
        read_fd, write_fd = os.pipe()
        try:
            reader_file = os.fdopen(read_fd, "r", encoding="utf-8", buffering=1)
            os.write(write_fd, b'{"event":"START"')
            reader = dfs_manyread_small.JsonLineReader(reader_file)
            with self.assertRaises(TimeoutError):
                reader.read_line(0.01)
            os.write(write_fd, b'}\n')
            self.assertEqual(json.loads(reader.read_line(1))["event"], "START")
            reader_file.close()
        finally:
            try:
                os.close(write_fd)
            except OSError:
                pass

    def test_run_io_timeout_and_nonzero_child_fail_closed(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            slow = Path(tmp) / "slow.py"
            slow.write_text("#!/usr/bin/env python3\nimport sys,time\nsys.stdout.write('{\\\"partial\\\":')\nsys.stdout.flush()\ntime.sleep(5)\n", encoding="utf-8")
            slow.chmod(0o755)
            timed = dfs_manyread_small.run_io(slow, Path(tmp) / "payload", "seq-read", "close", "existing", timeout=0.05)
            self.assertEqual(timed["status"], "FAIL")
            self.assertTrue(timed["timed_out"])
            self.assertTrue(timed["child_reaped"])
            self.assertIsNone(timed["rc"])
            self.assertIsInstance(timed["stdout"], str)

            bad = Path(tmp) / "bad.py"
            bad.write_text("#!/usr/bin/env python3\nimport json,sys\nprint(json.dumps(" + repr(good_io()) + "))\nsys.exit(7)\n", encoding="utf-8")
            bad.chmod(0o755)
            failed = dfs_manyread_small.run_io(bad, Path(tmp) / "payload", "seq-read", "close", "existing", timeout=1)
            self.assertEqual(failed["rc"], 7)
            self.assertEqual(failed["status"], "FAIL")
            self.assertEqual(failed["verify"]["status"], "FAIL")

    def test_sync_status_requires_measurement_rounds_and_final_eof(self) -> None:
        rounds = [{"round": 0, "measured": False, "samples": [{"status": "PASS", "verify": {"status": "PASS"}}]}]
        rounds.extend({"round": i, "measured": True, "samples": [{"status": "PASS", "verify": {"status": "PASS"}}]} for i in range(1, 6))
        self.assertEqual(dfs_manyread_small.sync_read_status(rounds, {"status": "PASS"}, {"status": "PASS"}), "DATA_RECORDED")
        wrong = list(rounds)
        wrong[1] = {**wrong[1], "measured": False}
        self.assertEqual(dfs_manyread_small.sync_read_status(wrong, {"status": "PASS"}, {"status": "PASS"}), "FAIL")
        self.assertEqual(dfs_manyread_small.sync_read_status(rounds, {"status": "PASS"}, {"status": "FAIL"}), "FAIL")

    def coordinator_input(self, failed_done: bool = False, wrong_round: bool = False, missing_peer: bool = False, bad_result: bool = False) -> str:
        lines = [
            {"event": "HELLO", "reader_id": "B", "session_token": "s"},
        ]
        if not missing_peer:
            lines.append({"event": "HELLO", "reader_id": "C", "session_token": "s"})
        for round_index in range(1, 6):
            lines.append({"event": "READY", "reader_id": "B", "session_token": "s", "round": round_index})
            lines.append({"event": "READY", "reader_id": "C", "session_token": "s", "round": round_index + 1 if wrong_round and round_index == 1 else round_index})
            token = f"s:round:{round_index}"
            status = "FAIL" if failed_done and round_index == 1 else "PASS"
            b_result = good_io()
            if bad_result and round_index == 1:
                b_result["io_bytes"] = dfs_manyread_small.DATA_BYTES - 1
            lines.append({"event": "DONE", "reader_id": "B", "session_token": "s", "round": round_index, "round_token": token, "status": status, "rc": 0, "result": b_result})
            lines.append({"event": "DONE", "reader_id": "C", "session_token": "s", "round": round_index, "round_token": token, "status": "PASS", "rc": 0, "result": good_io()})
        lines.extend([
            {"event": "FINAL", "reader_id": "B", "session_token": "s", "status": "DATA_RECORDED"},
            {"event": "FINAL", "reader_id": "C", "session_token": "s", "status": "DATA_RECORDED"},
        ])
        return "\n".join(json.dumps(line) for line in lines) + "\n"

    def run_coordinator_process(self, stdin_text: str, timeout: str = "1") -> tuple[subprocess.CompletedProcess[str], dict[str, object]]:
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp) / "coord"
            completed = subprocess.run(
                [sys.executable, str(Path(dfs_manyread_small.__file__)), "coordinator", "--output", str(out),
                 "--session-token", "s", "--round-timeout", timeout, "--readers", "B", "C"],
                input=stdin_text,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                timeout=5,
            )
            summary = json.loads((out / "summary.json").read_text(encoding="utf-8"))
            return completed, summary

    def test_coordinator_records_ctl_clock_boundaries_with_real_stdio_process(self) -> None:
        completed, summary = self.run_coordinator_process(self.coordinator_input())
        self.assertEqual(completed.returncode, 0, completed.stderr)
        starts = [json.loads(line) for line in completed.stdout.splitlines()]
        self.assertEqual(len(starts), 22)
        self.assertEqual(sum(1 for event in starts if event["event"] == "START"), 10)
        self.assertEqual(sum(1 for event in starts if event["event"] == "ACK"), 12)
        self.assertEqual(sum(1 for event in starts if event["event"] == "ACK" and event["round"] == 0), 2)
        self.assertEqual(summary["status"], "DATA_RECORDED")
        self.assertEqual(summary["platform"], {"system": "Linux", "machine": "aarch64", "euid": 0})
        self.assertEqual(summary["monotonic_clock"]["monotonic"], True)
        self.assertEqual(len(summary["rounds"]), 5)
        for row in summary["rounds"]:
            self.assertLessEqual(row["start_before_emit_ns"], row["end_after_both_done_ns"])
            self.assertGreaterEqual(row["elapsed_ns"], 0)


    def test_coordinator_ack_gates_fast_reader_next_ready_with_real_stdio_process(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp) / "coord"
            proc = subprocess.Popen(
                [sys.executable, str(Path(dfs_manyread_small.__file__)), "coordinator", "--output", str(out),
                 "--session-token", "s", "--round-timeout", "1", "--readers", "B", "C"],
                stdin=subprocess.PIPE,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
                bufsize=1,
            )
            assert proc.stdin is not None and proc.stdout is not None

            def send(event: dict[str, object]) -> None:
                proc.stdin.write(json.dumps(event) + "\n")
                proc.stdin.flush()

            def recv_event() -> dict[str, object]:
                return json.loads(proc.stdout.readline())

            send({"event": "HELLO", "reader_id": "B", "session_token": "s"})
            send({"event": "HELLO", "reader_id": "C", "session_token": "s"})
            ack0_events = [recv_event(), recv_event()]
            self.assertEqual({event["event"] for event in ack0_events}, {"ACK"})
            self.assertEqual({event["round"] for event in ack0_events}, {0})
            send({"event": "READY", "reader_id": "B", "session_token": "s", "round": 1})
            send({"event": "READY", "reader_id": "C", "session_token": "s", "round": 1})
            starts = [recv_event(), recv_event()]
            self.assertEqual({event["event"] for event in starts}, {"START"})
            token = starts[0]["round_token"]
            send({"event": "DONE", "reader_id": "B", "session_token": "s", "round": 1, "round_token": token,
                  "status": "PASS", "rc": 0, "result": good_io()})
            # Fast B cannot legally send READY round 2 until coordinator acknowledges both DONEs.
            send({"event": "DONE", "reader_id": "C", "session_token": "s", "round": 1, "round_token": token,
                  "status": "PASS", "rc": 0, "result": good_io()})
            acks = [recv_event(), recv_event()]
            self.assertEqual({event["event"] for event in acks}, {"ACK"})
            self.assertEqual({event["reader_id"] for event in acks}, {"B", "C"})
            proc.stdin.close()
            proc.stdout.close()
            assert proc.stderr is not None
            proc.stderr.close()
            proc.kill()
            proc.wait(timeout=2)

    def test_coordinator_fail_closed_on_missing_peer_wrong_round_or_failed_done(self) -> None:
        for payload, expected in [
            (self.coordinator_input(missing_peer=True), "expected HELLO"),
            (self.coordinator_input(wrong_round=True), "unexpected round"),
            (self.coordinator_input(failed_done=True), "status FAIL"),
            (self.coordinator_input(bad_result=True), "invalid C result"),
        ]:
            completed, summary = self.run_coordinator_process(payload, timeout="0.05")
            self.assertNotEqual(completed.returncode, 0)
            self.assertEqual(summary["status"], "BLOCKED")
            self.assertIn(expected, summary["error"])



if __name__ == "__main__":
    unittest.main()
