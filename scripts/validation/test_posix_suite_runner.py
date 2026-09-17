#!/usr/bin/env python3
"""POSIX 上游 suite wrapper 的本地回归测试。"""

from __future__ import annotations

import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]


class PosixSuiteRunnerTest(unittest.TestCase):
    def test_pjdfstest_fake_suite_writes_m1_result(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            work = Path(raw)
            suite = work / "pjdfstest"
            case = suite / "tests/open/00.t"
            case.parent.mkdir(parents=True)
            case.write_text(
                "#!/usr/bin/env python3\n"
                "import os, pathlib\n"
                "path=pathlib.Path(os.environ['PJD_TEST_PATH'])/'ok.txt'\n"
                "path.write_text('ok')\n"
                "print('1..1')\n"
                "print('ok 1 - fake DMS mounted path is writable')\n",
                encoding="utf-8",
            )
            case.chmod(0o755)
            allowlist = work / "allow.txt"
            allowlist.write_text("open/00|tests/open/00.t\n", encoding="utf-8")
            mount = work / "mnt"
            output = work / "out"
            mount.mkdir()
            completed = subprocess.run(
                [
                    "python3",
                    str(ROOT / "scripts/validation/run_m1_pjdfstest.py"),
                    "--suite-dir",
                    str(suite),
                    "--allowlist",
                    str(allowlist),
                    "--mountpoint",
                    str(mount),
                    "--output",
                    str(output),
                ],
                text=True,
                capture_output=True,
                check=False,
            )
            self.assertEqual(completed.returncode, 0, completed.stdout + completed.stderr)
            result = json.loads((output / "m1-result.json").read_text(encoding="utf-8"))
            self.assertEqual(result["cases"][0]["id"], "posix-pjdfstest-supported")
            self.assertEqual(result["cases"][0]["status"], "PASS")
            self.assertEqual((mount / "ok.txt").read_text(encoding="utf-8"), "ok")

    def test_fstests_missing_check_is_explicit_failure(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            work = Path(raw)
            suite = work / "xfstests"
            (suite / "tests/generic").mkdir(parents=True)
            (suite / "tests/generic/001").write_text("# fake\n", encoding="utf-8")
            allowlist = work / "allow.txt"
            allowlist.write_text("generic/001|001\n", encoding="utf-8")
            mount = work / "mnt"
            output = work / "out"
            mount.mkdir()
            completed = subprocess.run(
                [
                    "python3",
                    str(ROOT / "scripts/validation/run_m1_fstests.py"),
                    "--suite-dir",
                    str(suite),
                    "--allowlist",
                    str(allowlist),
                    "--mountpoint",
                    str(mount),
                    "--output",
                    str(output),
                ],
                text=True,
                capture_output=True,
                check=False,
            )
            self.assertEqual(completed.returncode, 2)
            result = json.loads((output / "m1-result.json").read_text(encoding="utf-8"))
            self.assertEqual(result["cases"][0]["status"], "FAIL")
            self.assertIn("check entry is missing", result["cases"][0]["message"])


if __name__ == "__main__":
    unittest.main()
