#!/usr/bin/env python3
"""Unit tests for scripts/package_sdk.py staging rewrites."""

from __future__ import annotations

import tempfile
import unittest
import json
import sys
from pathlib import Path
from unittest.mock import patch

import package_sdk

sys.path.insert(0, str(Path(__file__).resolve().parent / "sdk"))
import package_go


class PackageSdkRewriteTests(unittest.TestCase):
    def test_staged_protocol_preserves_source_constants_without_protoc(self) -> None:
        # 消费者使用的内联上限必须和 workspace 一致；不能仅复制生成 DTO。
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            generated = root / "dms.v1.rs"
            generated.write_text("// generated test fixture\n", encoding="utf-8")
            layout = package_sdk.PackageLayout(
                root, root / "stage", root / "target", root / "sdk.crate", root / "evidence"
            )
            layout.evidence_dir.mkdir()
            package_sdk.stage_sdk_crate(layout, generated, "0.1.0")
            protocol = (layout.stage_crate / "src/dms_protocol.rs").read_text()
            original = (package_sdk.SOURCE_ROOT / "protocol/src/lib.rs").read_text()
            self.assertEqual(protocol, original.replace(
                'tonic::include_proto!("dms.v1");',
                'include!("generated/dms.v1.rs");',
            ))
            self.assertIn("pub const MAX_INLINE_READ_BYTES", protocol)
            self.assertNotIn("tonic::include_proto!", protocol)

    def test_package_verification_uses_the_isolated_consumer_source(self) -> None:
        # 编译检查与真实 TCP/SHM 验收必须使用同一份消费者，不维护两套 API 清单。
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            layout = package_sdk.PackageLayout(
                root, root / "stage", root / "target", root / "sdk.crate", root / "evidence"
            )
            def simulate_command(command, **_):
                if command[0] == "tar":
                    (root / "consumer-check/vendor/dms-client-0.1.0").mkdir()

            with patch.object(package_sdk, "run_checked", side_effect=simulate_command) as run, \
                 patch.object(package_sdk, "write_cargo_directory_checksum"), \
                 patch.object(package_sdk, "stabilize_lock_to_source_versions"), \
                 patch.object(package_sdk, "verify_lock_uses_source_versions"):
                package_sdk.verify_consumer(layout, "0.1.0")
            self.assertEqual(
                (root / "consumer-check/src/main.rs").read_bytes(),
                (package_sdk.SOURCE_ROOT / "scripts/release/consumer.rs").read_bytes(),
            )
            self.assertEqual(run.call_args.args[0], ["cargo", "check", "--locked"])

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

    def test_copy_transformed_tree_ignores_macos_resource_forks(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            source = root / "source"
            destination = root / "destination"
            source.mkdir()
            (source / "lib.rs").write_text("pub fn ok() {}\n", encoding="utf-8")
            (source / "._lib.rs").write_bytes(b"\xa3not utf8")
            (source / ".DS_Store").write_bytes(b"metadata")

            package_sdk.copy_transformed_tree(source, destination, owner_module=None)

            self.assertTrue((destination / "lib.rs").is_file())
            self.assertFalse((destination / "._lib.rs").exists())
            self.assertFalse((destination / ".DS_Store").exists())

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

    def test_build_package_accepts_explicit_evidence_root(self) -> None:
        # 发布脚本默认 evidence 目录保留历史兼容；阶段化验证可显式指定新目录，
        # 避免不同阶段的日志混写到同一个固定路径。
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            evidence = root / "stage64" / "rust-package-01"
            layout = package_sdk.PackageLayout(
                root / "package",
                root / "package/stage",
                root / "package/target",
                root / "package/package/dms-client-0.1.0.crate",
                evidence,
            )

            with (
                patch.object(package_sdk, "workspace_version", return_value="0.1.0"),
                patch.object(package_sdk, "unique_layout", return_value=layout) as unique_layout,
                patch.object(package_sdk, "build_protocol_bindings", return_value=root / "dms.v1.rs"),
                patch.object(package_sdk, "stage_sdk_crate") as stage_sdk_crate,
                patch.object(package_sdk, "package_and_verify"),
                patch.object(package_sdk, "verify_consumer"),
                patch.object(package_sdk, "write_manifest"),
            ):
                result = package_sdk.build_package(evidence)

            unique_layout.assert_called_once_with("0.1.0", evidence)
            stage_sdk_crate.assert_called_once_with(layout, root / "dms.v1.rs", "0.1.0")
            self.assertEqual(result.evidence_dir, evidence)
            self.assertTrue(evidence.is_dir())

    def test_cargo_package_runs_offline(self) -> None:
        # RC 打包不应依赖公网 crates.io index；否则离线 VM 或临时网络抖动会让
        # 发布验证失败在基础设施层，而不是暴露 SDK 包本身的问题。
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            stage = root / "stage"
            stage.mkdir()
            (stage / "Cargo.toml").write_text(
                '[package]\nname = "dms-client"\nversion = "0.1.0"\nedition = "2024"\n',
                encoding="utf-8",
            )
            target = root / "target"
            package = target / "staging/package/dms-client-0.1.0.crate"
            layout = package_sdk.PackageLayout(
                root / "package",
                stage,
                target,
                root / "package/package/dms-client-0.1.0.crate",
                root / "evidence",
            )
            commands: list[list[str]] = []

            def simulate_command(command, **_):
                commands.append(command)
                if command[:2] == ["cargo", "package"]:
                    package.parent.mkdir(parents=True)
                    package.write_bytes(b"crate")

            with (
                patch.object(package_sdk, "run_checked", side_effect=simulate_command),
                patch.object(package_sdk, "stabilize_lock_to_source_versions"),
                patch.object(package_sdk, "verify_lock_uses_source_versions"),
            ):
                package_sdk.package_and_verify(layout)

            self.assertIn(
                ["cargo", "package", "--manifest-path", str(stage / "Cargo.toml"), "--locked", "--offline"],
                commands,
            )

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

    def test_lock_stabilization_recomputes_drift_after_each_pin(self) -> None:
        # `cargo update` 会重新求解整张依赖图。第一个 pin 可能已经顺带消除了
        # 第二个 drift；脚本不能继续使用旧 drift 列表去 pin 已不存在的版本。
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / "Cargo.toml").write_text(
                '[package]\nname = "dms-lock-test"\nversion = "0.0.0"\nedition = "2024"\n',
                encoding="utf-8",
            )
            (root / "Cargo.lock").write_text("", encoding="utf-8")
            evidence = root / "evidence"
            evidence.mkdir()
            commands: list[list[str]] = []

            def simulate_command(command, **_):
                commands.append(command)

            drift_results = [
                (
                    [
                        "wasm-bindgen: candidate versions ['0.2.128'] not subset of source ['0.2.127']",
                        "js-sys: candidate versions ['0.3.82'] not subset of source ['0.3.81']",
                    ],
                    [],
                ),
                ([], []),
            ]

            def simulate_drift(_lock_path):
                return drift_results.pop(0) if drift_results else ([], [])

            with (
                patch.object(package_sdk, "run_checked", side_effect=simulate_command),
                patch.object(package_sdk, "collect_lock_version_drift", side_effect=simulate_drift),
                patch.object(
                    package_sdk,
                    "choose_source_version",
                    side_effect=lambda name, _bad_version: {
                        "wasm-bindgen": "0.2.127",
                        "js-sys": "0.3.81",
                    }[name],
                ),
            ):
                package_sdk.stabilize_lock_to_source_versions(root, {}, evidence, "stale")

            self.assertEqual(
                commands,
                [
                    ["cargo", "generate-lockfile", "--offline", "--manifest-path", str(root / "Cargo.toml")],
                    ["cargo", "update", "-p", "wasm-bindgen@0.2.128", "--precise", "0.2.127", "--offline"],
                ],
            )
            self.assertEqual(
                (evidence / "stale-lock-version-pins.log").read_text(encoding="utf-8"),
                "wasm-bindgen@0.2.128 -> 0.2.127\n",
            )

    def test_packaged_manifest_contains_internalized_transport_dependencies(self) -> None:
        # dms-transport 被内联成 SDK 私有模块后，它原本的直接依赖也必须保留
        # 在发布 crate 的 Cargo.toml 中，否则 staging 编译到 grpc/error_status.rs
        # 才会发现缺少 prost-types / tonic-types。
        with tempfile.TemporaryDirectory() as temp:
            stage = Path(temp)
            package_sdk.write_package_cargo_toml(stage, "0.1.0", ["Cargo.toml", "src/**"])
            data = package_sdk.load_toml(stage / "Cargo.toml")
            dependencies = data["dependencies"]
            self.assertEqual(dependencies["prost-types"], "0.14.1")
            self.assertEqual(dependencies["tonic-types"], "0.14.6")

    def test_go_proxy_package_ignores_macos_resource_forks(self) -> None:
        # Go proxy zip 只按文件后缀过滤会误收 macOS AppleDouble 文件，例如
        # `._client.go` 或 `internal/pb/.../._dms.pb.go`。这些文件不是源码，
        # 进入候选包后会让外部消费者的 Go 编译失败。
        self.assertTrue(package_go.should_package_file(Path("client.go")))
        self.assertTrue(package_go.should_package_file(Path("internal/pb/dms/v1/dms.pb.go")))
        self.assertTrue(package_go.should_package_file(Path("go.mod")))
        self.assertFalse(package_go.should_package_file(Path("._client.go")))
        self.assertFalse(package_go.should_package_file(Path("internal/pb/dms/v1/._dms.pb.go")))


if __name__ == "__main__":
    unittest.main()
