#!/usr/bin/env python3
"""Unit tests for scripts/package_sdk.py staging rewrites."""

from __future__ import annotations

import tempfile
import unittest
import json
from pathlib import Path

import package_sdk


class PackageSdkRewriteTests(unittest.TestCase):
    def test_dependency_crate_names_become_private_root_modules(self) -> None:
        text = "use dms_error::DmsError;\nlet _ = dms_tracing::current_exemplar();\n"
        rewritten = package_sdk.transform_text(text, owner_module=None)
        self.assertIn("use crate::dms_error::DmsError;", rewritten)
        self.assertIn("crate::dms_tracing::current_exemplar()", rewritten)

    def test_internal_crate_self_path_is_relocated(self) -> None:
        text = "use crate::{TraceContext, set_parent};\nlet _ = dms_metrics::registry();\n"
        rewritten = package_sdk.transform_text(text, owner_module="dms_tracing")
        self.assertIn("use crate::dms_tracing::{TraceContext, set_parent};", rewritten)
        self.assertIn("crate::dms_metrics::registry()", rewritten)

    def test_runtime_only_internal_cfgs_are_disabled_in_package(self) -> None:
        text = '#[cfg(feature = "runtime")]\nmod init;\n#[cfg(feature = "test-support")]\nmod support;\n'
        rewritten = package_sdk.transform_text(text, owner_module=None)
        self.assertNotIn('feature = "runtime"', rewritten)
        self.assertNotIn('feature = "test-support"', rewritten)
        self.assertEqual(rewritten.count("#[cfg(any())]"), 2)

    def test_unique_layout_never_targets_existing_directory(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            layout = package_sdk.unique_layout("0.1.0", Path(temp) / "evidence")
            self.assertIn("dms-client-sdk-0.1.0-", layout.package_root.name)
            self.assertEqual(layout.stage_crate.name, "dms-client-0.1.0")
            self.assertEqual(layout.crate_file.name, "dms-client-0.1.0.crate")

    def test_directory_source_checksum_is_written_for_unpacked_crate(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            unpacked = root / "dms-client-0.1.0"
            unpacked.mkdir()
            (unpacked / "Cargo.toml").write_text("[package]\nname = \"dms-client\"\n", encoding="utf-8")
            crate_file = root / "dms-client-0.1.0.crate"
            crate_file.write_bytes(b"crate bytes")
            package_sdk.write_cargo_directory_checksum(unpacked, crate_file)
            checksum = json.loads((unpacked / ".cargo-checksum.json").read_text(encoding="utf-8"))
            self.assertIn("Cargo.toml", checksum["files"])
            self.assertEqual(
                checksum["package"],
                "6c1a3e927bfe496d41c3f8c58bec46b4a964aa6435fd05fa55e52a0491a34159",
            )


if __name__ == "__main__":
    unittest.main()
