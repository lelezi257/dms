"""Reject corrupted evidence without mutating the published source packet."""
import json
import pathlib
import shutil
import subprocess
import sys
import tempfile
import unittest

assert sys.platform == "linux"
SOURCE = pathlib.Path(sys.argv.pop(1)).resolve()
REL = pathlib.Path("development/evidence/20261001-round3-3fs-reference")


class AuditFailures(unittest.TestCase):
    def exercise(self, relative, mutate, expected):
        with tempfile.TemporaryDirectory(prefix="afs-v84-audit-") as name:
            root = pathlib.Path(name) / "source"
            root.mkdir()
            for entry in SOURCE.iterdir():
                if entry.name != "development":
                    (root / entry.name).symlink_to(entry, target_is_directory=entry.is_dir())
            (root / "development").mkdir()
            for entry in (SOURCE / "development").iterdir():
                if entry.name != "evidence":
                    (root / "development" / entry.name).symlink_to(entry, target_is_directory=entry.is_dir())
            (root / "development/evidence").mkdir()
            for entry in (SOURCE / "development/evidence").iterdir():
                target = root / "development/evidence" / entry.name
                if entry.name == REL.name:
                    shutil.copytree(entry, target)
                else:
                    target.symlink_to(entry, target_is_directory=entry.is_dir())
            path = root / REL / relative
            value = json.loads(path.read_text())
            mutate(value)
            path.write_text(json.dumps(value))
            result = subprocess.run([sys.executable, str(root / REL / "probes/audit.py"), str(root), str(pathlib.Path(name) / "result")], capture_output=True, text=True, timeout=15)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn(expected, result.stderr)

    def test_wrong_restart_content(self):
        self.exercise("b/v84-read-after-restart.json", lambda v: v.update(sha256="0" * 64), "b/v84-read-after-restart.json full")

    def test_false_physical_match(self):
        self.exercise("a/v84-physical-after.json", lambda v: v["receipts"][0].update(observed_sha256="0" * 64), "aafter chunk0")

    def test_forced_cleanup_is_not_normal(self):
        self.exercise("ctl/v84-stop-final.json", lambda v: v["stopped"][0].update(forced_kill=True), "ctl no forced signal")

    def test_successful_unmount_is_required(self):
        self.exercise("b/v84-stop-final.json", lambda v: v["stopped"][0]["umount"].update(exit=1), "b unmounted")


if __name__ == "__main__":
    unittest.main()
