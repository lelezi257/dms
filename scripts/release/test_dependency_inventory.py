"""仅在 Linux 执行：验证许可表达式、缺失报告、原文与路径边界。"""

import hashlib
import json
from pathlib import Path
import tempfile
import unittest

import dependency_inventory as inventory


class InventoryTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.crate = self.root / "crate"
        self.crate.mkdir()
        (self.crate / "Cargo.toml").write_text('[package]\nname="sample"\n')
        self.package = {
            "name": "sample", "version": "1.0.0", "id": "registry+example#sample@1.0.0",
            "source": "registry+example", "license": "(MIT OR Apache-2.0) AND Unicode-3.0",
            "license_file": None, "manifest_path": str(self.crate / "Cargo.toml"),
        }

    def collect(self):
        return inventory.collect({"packages": [self.package]}, self.root / "out", "lock-hash")

    def test_expression_and_original_bytes_preserved(self):
        content = b"Synthetic license fixture\r\nNot a rights declaration\r\n"
        (self.crate / "LICENSE-MIT").write_bytes(content)
        report = self.collect()
        item = report["packages"][0]
        self.assertEqual(item["license_expression"], self.package["license"])
        saved = self.root / "out" / item["files"][0]["path"]
        self.assertEqual(saved.read_bytes(), content)
        self.assertEqual(item["files"][0]["sha256"], hashlib.sha256(content).hexdigest())
        self.assertEqual(report["packages_with_issues"], 0)

    def test_missing_files_reported_not_omitted(self):
        report = self.collect()
        self.assertEqual(report["package_count"], 1)
        self.assertEqual(report["packages_with_issues"], 1)
        self.assertIn("no license/notice", report["packages"][0]["issues"][0])
        self.assertEqual(len(json.loads((self.root / "out/inventory.json").read_text())["packages"]), 1)

    def test_nested_notice_and_explicit_nonstandard_filename(self):
        nested = self.crate / "vendor/component"
        nested.mkdir(parents=True)
        (nested / "NOTICE.txt").write_text("upstream notice")
        (self.crate / "legal.txt").write_text("license")
        self.package["license_file"] = "legal.txt"
        report = self.collect()
        self.assertEqual(report["license_text_count"], 2)

    def test_escape_and_symlink_rejected(self):
        outside = self.root / "secret"
        outside.write_text("not distributable")
        (self.crate / "LICENSE").symlink_to(outside)
        self.package["license_file"] = "../secret"
        report = self.collect()
        self.assertEqual(report["license_text_count"], 0)
        self.assertTrue(report["packages"][0]["issues"])
        self.assertFalse((self.root / "out/texts").exists())

    def test_repeat_is_byte_identical(self):
        (self.crate / "LICENSE").write_text("license")
        self.collect()
        first = (self.root / "out/inventory.json").read_bytes()
        self.collect()
        self.assertEqual((self.root / "out/inventory.json").read_bytes(), first)

    def test_workspace_packages_not_mislabeled_third_party(self):
        self.package["source"] = None
        self.assertEqual(self.collect()["package_count"], 0)

    def test_output_rejects_unowned_files(self):
        (self.crate / "LICENSE").write_text("license")
        self.collect()
        (self.root / "out/user-note.txt").write_text("must not overwrite")
        with self.assertRaises(ValueError):
            inventory.prepare_output(self.root / "out")

    def test_output_rejects_symlink_directory(self):
        (self.crate / "LICENSE").write_text("license")
        self.collect()
        (self.root / "out/linked").symlink_to(self.crate, target_is_directory=True)
        with self.assertRaises(ValueError):
            inventory.prepare_output(self.root / "out")

    def test_owned_output_can_be_repeated(self):
        (self.crate / "LICENSE").write_text("license")
        report = self.collect()
        self.assertEqual(inventory.prepare_output(self.root / "out"), {report["packages"][0]["files"][0]["path"]})


if __name__ == "__main__":
    unittest.main()
