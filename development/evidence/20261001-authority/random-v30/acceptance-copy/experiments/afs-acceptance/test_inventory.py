#!/usr/bin/env python3
import json
import os
import pathlib
import subprocess
import sys
import tempfile
import time
import unittest

import inventory


class InventoryTests(unittest.TestCase):
    def test_parse_start_ticks_ignores_spaces_in_comm(self):
        fields_after_comm = ["S"] + [str(i) for i in range(4, 53)]
        fields_after_comm[19] = "123456"
        stat = "42 (name with spaces) " + " ".join(fields_after_comm)
        self.assertEqual(inventory.parse_start_ticks(stat), 123456)

    def test_rejects_invalid_process_name_and_pid(self):
        with self.assertRaises(inventory.InventoryError):
            inventory.parse_process_assignment("bad/name=123")
        with self.assertRaises(inventory.InventoryError):
            inventory.parse_process_assignment("meta=not-a-pid")

    def test_collects_current_process_without_cmdline_or_environ(self):
        probe = inventory.ProcessProbe("self", os.getpid(), "test")
        identity = inventory.collect_process_identity(probe)
        self.assertEqual(identity["name"], "self")
        self.assertEqual(identity["pid"], os.getpid())
        self.assertRegex(identity["sha256"], r"^[0-9a-f]{64}$")
        self.assertGreater(identity["start_ticks"], 0)
        self.assertIn("exe_path", identity)
        self.assertNotIn("cmdline", identity)
        self.assertNotIn("environ", identity)

    def test_dead_pid_exits_nonzero(self):
        proc = subprocess.Popen([sys.executable, "-c", "pass"])
        proc.wait(timeout=10)
        result = subprocess.run(
            [sys.executable, str(pathlib.Path(inventory.__file__)), "--process", f"dead={proc.pid}"],
            capture_output=True,
            text=True,
            timeout=20,
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("inventory error", result.stderr)

    def test_pid_file_probe_on_live_sleep(self):
        sleeper = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(30)"])
        try:
            with tempfile.TemporaryDirectory() as temp:
                pid_file = pathlib.Path(temp) / "sleep.pid"
                pid_file.write_text(f"{sleeper.pid}\n", encoding="utf-8")
                result = subprocess.run(
                    [sys.executable, str(pathlib.Path(inventory.__file__)), "--pid-file", f"sleep={pid_file}"],
                    capture_output=True,
                    text=True,
                    timeout=20,
                )
            self.assertEqual(result.returncode, 0, result.stderr)
            state = json.loads(result.stdout)
            process = state["processes"]["sleep"]
            self.assertEqual(process["pid"], sleeper.pid)
            self.assertEqual(process["pid_source"], str(pid_file))
            self.assertRegex(process["sha256"], r"^[0-9a-f]{64}$")
        finally:
            sleeper.terminate()
            try:
                sleeper.wait(timeout=5)
            except subprocess.TimeoutExpired:
                sleeper.kill()
                sleeper.wait(timeout=5)


if __name__ == "__main__":
    unittest.main()
