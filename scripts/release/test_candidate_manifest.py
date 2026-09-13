"""候选发布总清单的合同测试。"""

from __future__ import annotations

import json
from pathlib import Path
import tempfile
import unittest

import candidate_manifest


class CandidateManifestTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root / "Cargo.toml").write_text(
            '[workspace.package]\nversion = "0.1.0"\n',
            encoding="utf-8",
        )

    def write_file(self, relative: str, content: bytes = b"payload") -> Path:
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(content)
        return path

    def test_manifest_collects_release_artifacts_without_publishing(self) -> None:
        sdk = self.write_file("artifacts/sdk/dms-client-0.1.0.crate", b"rust-sdk")
        server = self.write_file("artifacts/server/dms-server-0.1.0-linux-x86_64.tar.gz", b"server")
        self.write_file(
            "artifacts/server/dms-server-0.1.0-linux-x86_64.tar.gz.sha256",
            f"{candidate_manifest.sha256_file(server)}  {server.name}\n".encode(),
        )
        go_zip = self.write_file(
            "artifacts/go/github.com/lelezi257/dms/sdk/go/@v/v0.1.0-rc.abc.zip",
            b"go-sdk",
        )
        self.write_file("artifacts/go/github.com/lelezi257/dms/sdk/go/@v/list", b"v0.1.0-rc.abc\n")
        self.write_file("artifacts/go/github.com/lelezi257/dms/sdk/go/@v/v0.1.0-rc.abc.mod", b"module x\n")
        self.write_file("artifacts/go/github.com/lelezi257/dms/sdk/go/@v/v0.1.0-rc.abc.info", b"{}\n")
        juicefs = self.write_file("artifacts/juicefs-dms/juicefs-dms-0.1.0-rc-linux-aarch64.tar.gz", b"juicefs")
        self.write_file(
            "artifacts/juicefs-dms/juicefs-dms-0.1.0-rc-linux-aarch64.tar.gz.sha256",
            f"{candidate_manifest.sha256_file(juicefs)}  {juicefs.name}\n".encode(),
        )
        inventory = self.write_file(
            "artifacts/legal/inventory.json",
            json.dumps({"package_count": 3, "packages_with_issues": 0, "license_text_count": 4}).encode(),
        )
        acceptance = self.write_file(
            "evidence/accept/consumer-result.json",
            json.dumps({"status": "passed", "sdk_source": "registry+http://127.0.0.1"}).encode(),
        )

        args = type("Args", (), {
            "source_root": self.root,
            "rust_sdk_crate": sdk,
            "server_archive": server,
            "juicefs_dms_archive": juicefs,
            "go_proxy": self.root / "artifacts/go",
            "third_party_inventory": inventory,
            "acceptance_result": acceptance,
        })()
        manifest = candidate_manifest.build_manifest(args)

        self.assertFalse(manifest["release"]["remote_publish"])
        self.assertEqual(manifest["release"]["version"], "0.1.0")
        kinds = {item["kind"]: item for item in manifest["artifacts"]}
        self.assertEqual(kinds["rust-sdk-crate"]["sha256"], candidate_manifest.sha256_file(sdk))
        self.assertEqual(kinds["server-archive"]["companion_sha256"], "artifacts/server/dms-server-0.1.0-linux-x86_64.tar.gz.sha256")
        self.assertEqual(kinds["juicefs-dms-archive"]["companion_sha256"], "artifacts/juicefs-dms/juicefs-dms-0.1.0-rc-linux-aarch64.tar.gz.sha256")
        self.assertEqual(kinds["go-sdk-module-proxy"]["archive_sha256"], candidate_manifest.sha256_file(go_zip))
        self.assertEqual(kinds["third-party-inventory"]["packages_with_issues"], 0)
        self.assertEqual(kinds["acceptance-result"]["status"], "passed")

    def test_wrong_sdk_name_fails_before_manifest_is_trusted(self) -> None:
        sdk = self.write_file("dms-common-0.1.0.crate")
        with self.assertRaisesRegex(ValueError, "Rust SDK crate 文件名"):
            candidate_manifest.rust_sdk_record(sdk, "0.1.0", self.root)

    def test_server_companion_checksum_must_match(self) -> None:
        server = self.write_file("dms-server-0.1.0-linux-x86_64.tar.gz", b"server")
        self.write_file("dms-server-0.1.0-linux-x86_64.tar.gz.sha256", b"0" * 64 + b"  bad\n")
        with self.assertRaisesRegex(ValueError, "companion sha256"):
            candidate_manifest.server_record(server, "0.1.0", self.root)

    def test_go_proxy_must_identify_one_module(self) -> None:
        self.write_file("one/@v/list", b"v0.1.0-rc.a\n")
        self.write_file("two/@v/list", b"v0.1.0-rc.b\n")
        with self.assertRaisesRegex(ValueError, "应有且仅有一份"):
            candidate_manifest.go_proxy_record(self.root, self.root)


if __name__ == "__main__":
    unittest.main()
