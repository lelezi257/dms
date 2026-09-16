#!/usr/bin/env python3
"""JuiceFS current-SDK 构建来源合同的轻量回归测试。"""

from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import tempfile
import unittest


MODULE_PATH = Path(__file__).with_name("build_juicefs_with_current_sdk.py")
SPEC = importlib.util.spec_from_file_location("build_juicefs_with_current_sdk", MODULE_PATH)
assert SPEC is not None and SPEC.loader is not None
module = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(module)


class JuicefsCurrentSdkBuildTest(unittest.TestCase):
    def test_shared_path_mapping_rejects_non_shared_path(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "shared" / "source"
            source.mkdir(parents=True)
            self.assertEqual(
                module.map_shared_path(source, root / "shared", Path("/workspace/dms")),
                Path("/workspace/dms/source"),
            )
            with self.assertRaisesRegex(module.BuildError, "不在共享宿主根目录"):
                module.map_shared_path(root / "outside", root / "shared", Path("/workspace/dms"))

    def test_validation_rejects_stale_sdk_or_changed_artifact(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            sdk = root / "sdk"
            sdk.mkdir()
            (sdk / "go.mod").write_text("module example/sdk\n", encoding="utf-8")
            artifact = root / "juicefs"
            artifact.write_bytes(b"binary-v1")
            provenance = root / "build-provenance.json"
            provenance.write_text(
                json.dumps(
                    {
                        "schema": module.SCHEMA,
                        "dependency_mode": "local-replace",
                        "artifact_sha256": module.sha256_file(artifact),
                        "dms_sdk_content_sha256": module.source_tree_sha256(sdk),
                    }
                ),
                encoding="utf-8",
            )
            module.validate_artifact(artifact, provenance, sdk)

            (sdk / "client.go").write_text("package sdk\n", encoding="utf-8")
            with self.assertRaisesRegex(module.BuildError, "SDK 已过期"):
                module.validate_artifact(artifact, provenance, sdk)

            (sdk / "client.go").unlink()
            artifact.write_bytes(b"binary-v2")
            with self.assertRaisesRegex(module.BuildError, "二进制哈希"):
                module.validate_artifact(artifact, provenance, sdk)


if __name__ == "__main__":
    unittest.main()
