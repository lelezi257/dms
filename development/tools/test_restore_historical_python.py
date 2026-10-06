#!/usr/bin/env python3
"""Regression tests for historical Python evidence restoration."""

from __future__ import annotations

import hashlib
import importlib.util
import json
import subprocess
import tempfile
import unittest
from pathlib import Path


MODULE_PATH = Path(__file__).with_name("restore_historical_python.py")
spec = importlib.util.spec_from_file_location("restore_historical_python", MODULE_PATH)
restore_historical_python = importlib.util.module_from_spec(spec)
assert spec and spec.loader
spec.loader.exec_module(restore_historical_python)


def git(repo: Path, *args: str, stdin: str | None = None) -> str:
    proc = subprocess.run(
        ["git", "-C", str(repo), *args],
        text=True,
        input=stdin,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=True,
    )
    return proc.stdout.strip()


class RestoreHistoricalPythonTests(unittest.TestCase):
    def setUp(self) -> None:
        self.td = tempfile.TemporaryDirectory()
        self.addCleanup(self.td.cleanup)
        self.root = Path(self.td.name)
        self.repo = self.root / "repo"
        self.repo.mkdir()
        git(self.repo, "init")
        git(self.repo, "config", "user.email", "test@example.invalid")
        git(self.repo, "config", "user.name", "Restore Test")
        self.rel = "development/evidence/example/script.py"
        path = self.repo / self.rel
        path.parent.mkdir(parents=True)
        path.write_text("#!/usr/bin/env python3\nprint('ok')\n", encoding="utf-8")
        path.chmod(0o755)
        git(self.repo, "add", self.rel)
        git(self.repo, "commit", "-m", "fixture")
        self.blob = git(self.repo, "rev-parse", f"HEAD:{self.rel}")
        self.data = path.read_bytes()
        self.entry = {
            "commit": git(self.repo, "rev-parse", "HEAD"),
            "path": self.rel,
            "git_blob": self.blob,
            "sha256": hashlib.sha256(self.data).hexdigest(),
            "bytes": len(self.data),
            "mode": "100755",
        }
        other = self.repo / "development/evidence/example/other.py"
        other.write_bytes(self.data)
        git(self.repo, "add", other.relative_to(self.repo).as_posix())
        git(self.repo, "commit", "-m", "same blob other path")
        self.head = git(self.repo, "rev-parse", "HEAD")
        self.entry["commit"] = self.head

    def write_manifest(self, entries: list[dict]) -> Path:
        manifest = self.root / "manifest.json"
        manifest.write_text(json.dumps({"files": entries}, indent=2), encoding="utf-8")
        return manifest

    def restore(self, entries: list[dict]):
        return restore_historical_python.restore(self.write_manifest(entries), self.repo, self.root / "out")

    def test_restores_exact_bytes_and_mode(self) -> None:
        self.assertEqual({"files": 1, "bytes": len(self.data)}, self.restore([self.entry]))
        out = self.root / "out" / self.rel
        self.assertEqual(self.data, out.read_bytes())
        self.assertEqual(0o755, out.stat().st_mode & 0o777)

    def test_rejects_escape_path(self) -> None:
        bad = dict(self.entry, path="../outside.py")
        with self.assertRaisesRegex(restore_historical_python.RestoreError, "escapes"):
            self.restore([bad])

    def test_rejects_duplicate_paths(self) -> None:
        with self.assertRaisesRegex(restore_historical_python.RestoreError, "duplicate path"):
            self.restore([self.entry, dict(self.entry)])

    def test_rejects_wrong_commit(self) -> None:
        bad = dict(self.entry, commit="0" * 40)
        with self.assertRaisesRegex(restore_historical_python.RestoreError, "cannot read git tree"):
            self.restore([bad])

    def test_rejects_wrong_path_even_with_same_blob(self) -> None:
        bad = dict(self.entry, path="development/evidence/example/missing.py")
        with self.assertRaisesRegex(restore_historical_python.RestoreError, "path missing"):
            self.restore([bad])

    def test_rejects_tree_blob_mismatch(self) -> None:
        other_blob = git(self.repo, "hash-object", "-w", "--stdin", stdin="different\n")
        bad = dict(self.entry, git_blob=other_blob)
        with self.assertRaisesRegex(restore_historical_python.RestoreError, "git tree blob mismatch"):
            self.restore([bad])

    def test_rejects_tree_mode_mismatch(self) -> None:
        bad = dict(self.entry, mode="100644")
        with self.assertRaisesRegex(restore_historical_python.RestoreError, "git tree mode mismatch"):
            self.restore([bad])

    def test_rejects_missing_blob(self) -> None:
        bad = dict(self.entry, git_blob="0" * 40)
        with self.assertRaisesRegex(restore_historical_python.RestoreError, "git tree blob mismatch"):
            self.restore([bad])

    def test_rejects_wrong_sha256(self) -> None:
        bad = dict(self.entry, sha256="0" * 64)
        with self.assertRaisesRegex(restore_historical_python.RestoreError, "sha256 mismatch"):
            self.restore([bad])

    def test_rejects_wrong_byte_count(self) -> None:
        bad = dict(self.entry, bytes=len(self.data) + 1)
        with self.assertRaisesRegex(restore_historical_python.RestoreError, "byte count mismatch"):
            self.restore([bad])

    def test_rejects_bad_mode(self) -> None:
        bad = dict(self.entry, mode="040000")
        with self.assertRaisesRegex(restore_historical_python.RestoreError, "mode"):
            self.restore([bad])

    def test_rejects_existing_output_file(self) -> None:
        out = self.root / "out" / self.rel
        out.parent.mkdir(parents=True)
        out.write_text("do not overwrite\n", encoding="utf-8")
        with self.assertRaisesRegex(restore_historical_python.RestoreError, "output already exists"):
            self.restore([self.entry])

    def test_rejects_existing_output_symlink(self) -> None:
        out = self.root / "out" / self.rel
        out.parent.mkdir(parents=True)
        out.symlink_to(self.root / "outside")
        with self.assertRaisesRegex(restore_historical_python.RestoreError, "output already exists"):
            self.restore([self.entry])

    def test_rejects_symlink_parent(self) -> None:
        parent = self.root / "out" / "development" / "evidence"
        parent.parent.mkdir(parents=True)
        parent.symlink_to(self.root)
        with self.assertRaisesRegex(restore_historical_python.RestoreError, "parent is a symlink"):
            self.restore([self.entry])


if __name__ == "__main__":
    unittest.main()
