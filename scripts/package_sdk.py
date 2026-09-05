#!/usr/bin/env python3
"""Build a single-crate Rust SDK candidate package.

The source workspace intentionally keeps shared implementation in private
`common/*` and `protocol` crates.  This release-prep tool creates a disposable
staging crate that vendors those private crates as internal modules of
`dms-client`, pre-generates protobuf bindings, and then runs `cargo package`.

It does not edit the source crates and it does not publish anything remotely.
"""

from __future__ import annotations

import argparse
import datetime as _dt
import os
import re
import shutil
import subprocess
import sys
import hashlib
import json
import tomllib
from dataclasses import dataclass
from pathlib import Path


SOURCE_ROOT = Path(__file__).resolve().parents[1]
ARTIFACTS_ROOT = SOURCE_ROOT / "artifacts"
DEFAULT_EVIDENCE_ROOT = SOURCE_ROOT.parent / "evidence" / "2026-09-05-v01-stage4" / "sdk"

INTERNAL_CRATES = {
    "dms-error": "dms_error",
    "dms-metrics": "dms_metrics",
    "dms-protocol": "dms_protocol",
    "dms-shm": "dms_shm",
    "dms-tracing": "dms_tracing",
    "dms-transport": "dms_transport",
}

SOURCE_DEPENDENCY_MANIFESTS = [
    SOURCE_ROOT / "sdk/rust/dms-client/Cargo.toml",
    SOURCE_ROOT / "common/error/Cargo.toml",
    SOURCE_ROOT / "common/metrics/Cargo.toml",
    SOURCE_ROOT / "common/shm/Cargo.toml",
    SOURCE_ROOT / "common/tracing/Cargo.toml",
    SOURCE_ROOT / "common/transport/Cargo.toml",
    SOURCE_ROOT / "protocol/Cargo.toml",
]


@dataclass(frozen=True)
class PackageLayout:
    package_root: Path
    stage_crate: Path
    target_dir: Path
    crate_file: Path
    evidence_dir: Path


def workspace_version(source_root: Path = SOURCE_ROOT) -> str:
    cargo_toml = (source_root / "Cargo.toml").read_text(encoding="utf-8")
    match = re.search(r"(?m)^version\s*=\s*\"([^\"]+)\"", cargo_toml)
    if not match:
        raise RuntimeError("workspace package version not found in Cargo.toml")
    return match.group(1)


def unique_layout(version: str, evidence_root: Path = DEFAULT_EVIDENCE_ROOT) -> PackageLayout:
    stamp = _dt.datetime.now().strftime("%Y%m%d-%H%M%S")
    base = ARTIFACTS_ROOT / f"dms-client-sdk-{version}-{stamp}-{os.getpid()}"
    if base.exists():
        raise RuntimeError(f"refusing to reuse existing artifact directory: {base}")
    stage_crate = base / f"dms-client-{version}"
    target_dir = base / "target"
    crate_file = base / "package" / f"dms-client-{version}.crate"
    return PackageLayout(
        package_root=base,
        stage_crate=stage_crate,
        target_dir=target_dir,
        crate_file=crate_file,
        evidence_dir=evidence_root,
    )


def run_checked(
    command: list[str],
    *,
    cwd: Path,
    log_path: Path,
    env: dict[str, str] | None = None,
) -> None:
    log_path.parent.mkdir(parents=True, exist_ok=True)
    with log_path.open("w", encoding="utf-8") as log:
        log.write(f"$ {' '.join(command)}\n")
        log.write(f"# cwd: {cwd}\n\n")
        log.flush()
        result = subprocess.run(
            command,
            cwd=cwd,
            env=env,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            check=False,
        )
        log.write(result.stdout)
        log.write(f"\n# exit: {result.returncode}\n")
    if result.returncode != 0:
        raise RuntimeError(f"command failed, see {log_path}: {' '.join(command)}")


def load_toml(path: Path) -> dict:
    with path.open("rb") as handle:
        return tomllib.load(handle)


def dependency_name_is_internal(name: str, spec: object) -> bool:
    if name in INTERNAL_CRATES:
        return True
    return isinstance(spec, dict) and "path" in spec and name.startswith("dms-")


def dependency_version(spec: object) -> str | None:
    if isinstance(spec, str):
        return spec
    if isinstance(spec, dict):
        value = spec.get("version")
        return value if isinstance(value, str) else None
    return None


def dependency_features(spec: object) -> set[str]:
    if isinstance(spec, dict):
        features = spec.get("features", [])
        if isinstance(features, list):
            return {feature for feature in features if isinstance(feature, str)}
    return set()


def dependency_optional(spec: object) -> bool:
    return isinstance(spec, dict) and bool(spec.get("optional", False))


def source_dependency_specs() -> dict[str, list[object]]:
    specs: dict[str, list[object]] = {}
    for manifest in SOURCE_DEPENDENCY_MANIFESTS:
        data = load_toml(manifest)
        for section in ["dependencies", "dev-dependencies", "build-dependencies"]:
            dependencies = data.get(section, {})
            if not isinstance(dependencies, dict):
                continue
            for name, spec in dependencies.items():
                if dependency_name_is_internal(name, spec):
                    continue
                specs.setdefault(name, []).append(spec)
    return specs


def verify_dependency_manifest_consistency(stage_crate: Path, evidence_dir: Path) -> None:
    source_specs = source_dependency_specs()
    stage_data = load_toml(stage_crate / "Cargo.toml")
    checked: list[str] = []
    errors: list[str] = []
    for section in ["dependencies", "dev-dependencies"]:
        dependencies = stage_data.get(section, {})
        if not isinstance(dependencies, dict):
            continue
        for name, spec in sorted(dependencies.items()):
            expected = source_specs.get(name, [])
            if not expected:
                errors.append(f"{section}.{name}: not present in SDK source manifests")
                continue
            version = dependency_version(spec)
            expected_versions = {dependency_version(item) for item in expected}
            expected_versions.discard(None)
            if version not in expected_versions:
                errors.append(
                    f"{section}.{name}: version {version!r} not in source manifest versions {sorted(expected_versions)!r}"
                )
            features = dependency_features(spec)
            expected_features = set().union(*(dependency_features(item) for item in expected))
            if not features.issubset(expected_features):
                errors.append(
                    f"{section}.{name}: features {sorted(features)!r} exceed source manifest union {sorted(expected_features)!r}"
                )
            if dependency_optional(spec) and not any(dependency_optional(item) for item in expected):
                errors.append(f"{section}.{name}: marked optional but source manifests never mark it optional")
            checked.append(
                f"{section}.{name}: version={version}; features={sorted(features)}; optional={dependency_optional(spec)}"
            )
    log_path = evidence_dir / "00-dependency-manifest-consistency.log"
    log_path.parent.mkdir(parents=True, exist_ok=True)
    log_path.write_text("\n".join(checked + errors) + "\n", encoding="utf-8")
    if errors:
        raise RuntimeError(f"dependency manifest consistency failed, see {log_path}")


def lock_versions(lock_path: Path) -> dict[str, set[str]]:
    data = load_toml(lock_path)
    versions: dict[str, set[str]] = {}
    for package in data.get("package", []):
        if not isinstance(package, dict):
            continue
        name = package.get("name")
        version = package.get("version")
        if isinstance(name, str) and isinstance(version, str):
            versions.setdefault(name, set()).add(version)
    return versions


def verify_lock_uses_source_versions(lock_path: Path, evidence_dir: Path, log_name: str) -> None:
    errors, checked = collect_lock_version_drift(lock_path)
    log_path = evidence_dir / log_name
    log_path.write_text("\n".join(checked + errors) + "\n", encoding="utf-8")
    if errors:
        raise RuntimeError(f"lock version consistency failed, see {log_path}")


def collect_lock_version_drift(lock_path: Path) -> tuple[list[str], list[str]]:
    source_versions = lock_versions(SOURCE_ROOT / "Cargo.lock")
    candidate_versions = lock_versions(lock_path)
    errors: list[str] = []
    checked: list[str] = []
    for name, versions in sorted(candidate_versions.items()):
        if name.startswith("dms-"):
            continue
        expected = source_versions.get(name)
        if expected is None:
            errors.append(f"{name}: new registry package in candidate lock: {sorted(versions)}")
            continue
        if not versions.issubset(expected):
            errors.append(f"{name}: candidate versions {sorted(versions)} not subset of source {sorted(expected)}")
        else:
            checked.append(f"{name}: {sorted(versions)}")
    return errors, checked


def choose_source_version(name: str, bad_version: str) -> str:
    expected_versions = sorted(lock_versions(SOURCE_ROOT / "Cargo.lock").get(name, set()))
    if not expected_versions:
        raise RuntimeError(f"{name} is not present in source Cargo.lock")
    bad_major = bad_version.split(".", 1)[0]
    same_major = [version for version in expected_versions if version.split(".", 1)[0] == bad_major]
    return same_major[-1] if same_major else expected_versions[-1]


def stabilize_lock_to_source_versions(
    manifest_dir: Path,
    env: dict[str, str],
    evidence_dir: Path,
    log_prefix: str,
) -> None:
    run_checked(
        ["cargo", "generate-lockfile", "--offline", "--manifest-path", str(manifest_dir / "Cargo.toml")],
        cwd=manifest_dir,
        env=env,
        log_path=evidence_dir / f"{log_prefix}-generate-lockfile-offline.log",
    )
    update_log = evidence_dir / f"{log_prefix}-lock-version-pins.log"
    updates: list[str] = []
    for _ in range(20):
        errors, _ = collect_lock_version_drift(manifest_dir / "Cargo.lock")
        if not errors:
            break
        progressed = False
        for error in errors:
            match = re.match(r"([^:]+): candidate versions \[(.*?)\] not subset", error)
            if not match:
                raise RuntimeError(f"cannot stabilize lock drift: {error}")
            name = match.group(1)
            bad_versions = [item.strip().strip("'") for item in match.group(2).split(",") if item.strip()]
            for bad_version in bad_versions:
                source_versions = lock_versions(SOURCE_ROOT / "Cargo.lock").get(name, set())
                if bad_version in source_versions:
                    continue
                target = choose_source_version(name, bad_version)
                run_checked(
                    ["cargo", "update", "-p", f"{name}@{bad_version}", "--precise", target, "--offline"],
                    cwd=manifest_dir,
                    env=env,
                    log_path=evidence_dir / f"{log_prefix}-pin-{name}-{bad_version}-to-{target}.log",
                )
                updates.append(f"{name}@{bad_version} -> {target}")
                progressed = True
        if not progressed:
            raise RuntimeError(f"lock drift remained but no update was possible: {errors}")
    update_log.write_text("\n".join(updates) + ("\n" if updates else "no pins needed\n"), encoding="utf-8")


def stage_direct_dependency_names(stage_crate: Path) -> list[str]:
    data = load_toml(stage_crate / "Cargo.toml")
    names: set[str] = set()
    for section in ["dependencies", "dev-dependencies"]:
        dependencies = data.get(section, {})
        if isinstance(dependencies, dict):
            names.update(dependencies)
    return sorted(names)


def replace_lock_package_dependencies(lock_text: str, package_name: str, dependencies: list[str]) -> str:
    name_marker = f'name = "{package_name}"'
    name_start = lock_text.index(name_marker)
    block_start = lock_text.rfind("[[package]]", 0, name_start)
    next_block = lock_text.find("[[package]]", name_start)
    block_end = len(lock_text) if next_block == -1 else next_block
    block = lock_text[block_start:block_end]
    replacement = "dependencies = [\n" + "".join(f' "{dependency}",\n' for dependency in dependencies) + "]"
    if "dependencies = [" not in block:
        insert_at = block.find("\n", block.find("version = ")) + 1
        block = block[:insert_at] + replacement + "\n" + block[insert_at:]
    else:
        deps_start = block.index("dependencies = [")
        deps_end = block.index("]\n", deps_start) + 1
        block = block[:deps_start] + replacement + block[deps_end:]
    return lock_text[:block_start] + block + lock_text[block_end:]


def write_staging_lock_from_source(stage_crate: Path, evidence_dir: Path) -> None:
    source_lock = (SOURCE_ROOT / "Cargo.lock").read_text(encoding="utf-8")
    direct_dependencies = stage_direct_dependency_names(stage_crate)
    candidate_lock = replace_lock_package_dependencies(source_lock, "dms-client", direct_dependencies)
    (stage_crate / "Cargo.lock").write_text(candidate_lock, encoding="utf-8")
    (evidence_dir / "04-staging-lock-prepared.log").write_text(
        "dms-client direct dependencies:\n"
        + "\n".join(f"- {dependency}" for dependency in direct_dependencies)
        + "\n",
        encoding="utf-8",
    )


def write_consumer_lock_from_staging(
    consumer: Path,
    consumer_name: str,
    staging_lock: Path,
    evidence_dir: Path,
    log_name: str,
) -> None:
    lock_text = staging_lock.read_text(encoding="utf-8")
    header_end = lock_text.index("[[package]]")
    consumer_block = (
        f"[[package]]\nname = \"{consumer_name}\"\nversion = \"0.0.0\"\n"
        "dependencies = [\n \"dms-client\",\n]\n\n"
    )
    consumer_lock = lock_text[:header_end] + consumer_block + lock_text[header_end:]
    (consumer / "Cargo.lock").write_text(consumer_lock, encoding="utf-8")
    (evidence_dir / log_name).write_text(
        f"prepared consumer lock for {consumer_name} from {staging_lock}\n",
        encoding="utf-8",
    )


def rewrite_internal_crate_paths(text: str) -> str:
    for crate_name, module_name in INTERNAL_CRATES.items():
        text = re.sub(
            rf"(?<![\w:]){re.escape(crate_name.replace('-', '_'))}::",
            f"crate::{module_name}::",
            text,
        )
        text = re.sub(
            rf"(?<![\w:]){re.escape(crate_name)}::",
            f"crate::{module_name}::",
            text,
        )
    return text


def relocate_crate_self_paths(text: str, module_name: str) -> str:
    return re.sub(r"(?<![\w:])crate::", f"crate::{module_name}::", text)


def transform_text(text: str, *, owner_module: str | None) -> str:
    if owner_module is not None:
        text = relocate_crate_self_paths(text, owner_module)
    text = rewrite_internal_crate_paths(text)
    # The packaged SDK mirrors the source dms-client default dependency graph:
    # dms-tracing is internalized with its default features only. Runtime and
    # test-support are process/component features in the source workspace, not
    # public SDK package features, so their cfg blocks are made permanently
    # inactive in the disposable staging crate.
    text = text.replace('cfg(feature = "runtime")', "cfg(any())")
    text = text.replace('cfg(feature = "test-support")', "cfg(any())")
    return text


def copy_transformed_tree(source: Path, destination: Path, *, owner_module: str | None) -> None:
    for path in source.rglob("*"):
        relative = path.relative_to(source)
        target = destination / relative
        if path.is_dir():
            target.mkdir(parents=True, exist_ok=True)
            continue
        target.parent.mkdir(parents=True, exist_ok=True)
        if path.suffix == ".rs":
            text = path.read_text(encoding="utf-8")
            text = transform_text(text, owner_module=owner_module)
            target.write_text(text, encoding="utf-8")
        else:
            shutil.copy2(path, target)


def write_package_cargo_toml(stage_crate: Path, version: str, package_files: list[str]) -> None:
    include_entries = "\n".join(f'    "{name}",' for name in package_files)
    cargo_toml = f"""[package]
name = "dms-client"
version = "{version}"
edition = "2024"
rust-version = "1.95.0"
license = "Apache-2.0"
description = "Rust client SDK for the DMS near-compute distributed memory store"
readme = "README.md"
include = [
{include_entries}
]

[dependencies]
hyper-util = {{ version = "0.1.20", features = ["tokio"] }}
libc = "0.2.178"
log = "0.4.28"
opentelemetry = {{ version = "0.32.0", default-features = false, features = ["trace"] }}
opentelemetry_sdk = {{ version = "0.32.0", default-features = false, features = ["trace"] }}
prometheus = {{ version = "0.14.0", default-features = false }}
prost = "0.14.1"
thiserror = "2.0.16"
tokio = {{ version = "1.47.1", features = ["net", "rt-multi-thread", "sync", "time"] }}
tokio-stream = "0.1.17"
tonic = {{ version = "0.14.6", features = ["tls-ring"] }}
tonic-prost = "0.14.6"
tower = {{ version = "0.5.2", features = ["util"] }}
tracing = {{ version = "0.1.41", features = ["log"] }}
tracing-opentelemetry = {{ version = "0.33.0", default-features = false }}
uuid = {{ version = "1.18.1", features = ["v4"] }}

[dev-dependencies]
toml = "0.9.8"

[lints.rust]
unsafe_code = "allow"

[lints.clippy]
all = "deny"

[workspace]
"""
    (stage_crate / "Cargo.toml").write_text(cargo_toml, encoding="utf-8")


def write_package_readme(stage_crate: Path, version: str) -> None:
    (stage_crate / "README.md").write_text(
        f"""# dms-client

Rust SDK for DMS applications.

This package is a local {version} release candidate. It contains the private
DMS common/protocol implementation as crate-internal modules.

For the current candidate build, use the maintainer-provided local sparse
registry instead of a public registry lookup. The maintainer starts that
registry with `candidate_registry.py` on `localhost:26880`.

`Cargo.toml`:

```toml
[dependencies]
dms-client = {{ version = "{version}", registry = "dms-candidate" }}
```

`.cargo/config.toml`:

```toml
[registries.dms-candidate]
index = "sparse+http://127.0.0.1:26880/"
```

After the SDK is publicly published, applications can use the plain dependency
form `dms-client = "{version}"`. Do not use that bare form for this local
candidate unless your Cargo registry configuration deliberately maps it to the
candidate package.

```rust
use dms_client::{{ClientOptions, DmsClient}};

fn main() -> Result<(), Box<dyn std::error::Error>> {{
    let client = DmsClient::connect("http://127.0.0.1:25200", ClientOptions::default())?;
    client.set("example/key", b"hello")?;
    assert_eq!(client.get("example/key")?.as_deref(), Some(&b"hello"[..]));
    Ok(())
}}
```

- [Client API](src/client.rs)
- [Public value types](src/types.rs)
- [Change log](CHANGELOG.md)

`get` returns `Ok(None)` when a key is missing; operational failures return
`Err(DmsError)`. A successful synchronous write is immediately visible to a
following read without sleeping.

The package intentionally defines no extra public Cargo features. It internalizes
the source SDK's default dependency graph only; process/component tracing runtime
setup remains outside the SDK package API.
""",
        encoding="utf-8",
    )


def rewrite_package_changelog(stage_crate: Path) -> None:
    changelog = stage_crate / "CHANGELOG.md"
    if changelog.is_file():
        text = changelog.read_text(encoding="utf-8")
        text = text.replace("(docs/product.md)", "(#使用限制)")
        changelog.write_text(text, encoding="utf-8")


def build_protocol_bindings(source_root: Path, target_dir: Path, evidence_dir: Path) -> Path:
    env = os.environ.copy()
    env["CARGO_TARGET_DIR"] = str(target_dir / "source-protocol")
    run_checked(
        ["cargo", "build", "-p", "dms-protocol", "--locked"],
        cwd=source_root,
        env=env,
        log_path=evidence_dir / "01-generate-protocol.log",
    )
    generated = sorted((target_dir / "source-protocol").glob("debug/build/dms-protocol-*/out/dms.v1.rs"))
    if not generated:
        raise RuntimeError("generated dms.v1.rs not found after building dms-protocol")
    return generated[-1]


def stage_sdk_crate(layout: PackageLayout, generated_protocol: Path, version: str) -> None:
    stage = layout.stage_crate
    if stage.exists():
        raise RuntimeError(f"refusing to overwrite staging crate: {stage}")
    stage.mkdir(parents=True)
    (stage / "src").mkdir()

    package_files = [
        "Cargo.toml",
        "README.md",
        "error-codes.toml",
        "src/**",
    ]
    for legal_file in ["LICENSE", "NOTICE", "CHANGELOG.md"]:
        source_file = SOURCE_ROOT / legal_file
        if source_file.is_file():
            shutil.copy2(source_file, stage / legal_file)
            package_files.append(legal_file)

    write_package_cargo_toml(stage, version, package_files)
    verify_dependency_manifest_consistency(stage, layout.evidence_dir)
    write_package_readme(stage, version)
    rewrite_package_changelog(stage)
    shutil.copy2(SOURCE_ROOT / "error-codes.toml", stage / "error-codes.toml")
    shutil.copy2(SOURCE_ROOT / "Cargo.lock", stage / "Cargo.lock")

    client_src = SOURCE_ROOT / "sdk/rust/dms-client/src"
    copy_transformed_tree(client_src, stage / "src", owner_module=None)

    internal_sources = [
        ("dms_error", SOURCE_ROOT / "common/error/src"),
        ("dms_metrics", SOURCE_ROOT / "common/metrics/src"),
        ("dms_shm", SOURCE_ROOT / "common/shm/src"),
        ("dms_tracing", SOURCE_ROOT / "common/tracing/src"),
        ("dms_transport", SOURCE_ROOT / "common/transport/src"),
    ]
    for module_name, source in internal_sources:
        destination = stage / "src" / module_name
        copy_transformed_tree(source, destination, owner_module=module_name)
        if module_name == "dms_tracing":
            for runtime_only in ["init.rs", "server.rs"]:
                runtime_only_file = destination / runtime_only
                if runtime_only_file.exists():
                    runtime_only_file.unlink()
        lib_rs = destination / "lib.rs"
        if lib_rs.exists():
            mod_rs = destination / "mod.rs"
            lib_rs.rename(mod_rs)
            if module_name == "dms_tracing":
                text = mod_rs.read_text(encoding="utf-8")
                text = re.sub(
                    r'(?m)^#\[cfg\(any\(\)\)\]\n(?:mod (?:init|server);|pub use (?:init|server)::[^\n]+;)\n?',
                    "",
                    text,
                )
                mod_rs.write_text(text, encoding="utf-8")
            if module_name == "dms_error":
                text = mod_rs.read_text(encoding="utf-8")
                text = text.replace('../../error-codes.toml"', 'error-codes.toml"')
                mod_rs.write_text(text, encoding="utf-8")

    protocol_module = stage / "src" / "dms_protocol.rs"
    generated_destination = stage / "src" / "generated" / "dms.v1.rs"
    generated_destination.parent.mkdir(parents=True)
    shutil.copy2(generated_protocol, generated_destination)
    protocol_module.write_text(
        """//! Generated Rust view of the versioned DMS wire contract.
//!
//! The `.proto` files remain the language-neutral source of truth. In this
//! packaged SDK crate the generated bindings are private implementation
//! details, so SDK users do not need `prost`, `tonic` DTO imports, or `protoc`.

pub mod v1 {
    include!("generated/dms.v1.rs");
}
""",
        encoding="utf-8",
    )

    lib_rs = stage / "src/lib.rs"
    original_lib = lib_rs.read_text(encoding="utf-8")
    prefix = """#[allow(dead_code, unused_imports)]
mod dms_error;
#[allow(dead_code, unused_imports)]
mod dms_metrics;
#[allow(dead_code, unused_imports)]
mod dms_protocol;
#[allow(dead_code, unused_imports)]
mod dms_shm;
#[allow(dead_code, unused_imports)]
mod dms_tracing;
#[allow(dead_code, unused_imports)]
mod dms_transport;

"""
    insertion_point = original_lib.find("mod client;")
    if insertion_point < 0:
        raise RuntimeError("could not find module insertion point in dms-client lib.rs")
    lib_rs.write_text(
        original_lib[:insertion_point] + prefix + original_lib[insertion_point:],
        encoding="utf-8",
    )


def package_and_verify(layout: PackageLayout) -> None:
    env = os.environ.copy()
    env["CARGO_TARGET_DIR"] = str(layout.target_dir / "staging")
    run_checked(
        ["cargo", "fmt", "--manifest-path", str(layout.stage_crate / "Cargo.toml")],
        cwd=layout.stage_crate,
        env=env,
        log_path=layout.evidence_dir / "02-staging-fmt-apply.log",
    )
    run_checked(
        ["cargo", "fmt", "--manifest-path", str(layout.stage_crate / "Cargo.toml"), "--", "--check"],
        cwd=layout.stage_crate,
        env=env,
        log_path=layout.evidence_dir / "03-staging-fmt-check.log",
    )
    stabilize_lock_to_source_versions(layout.stage_crate, env, layout.evidence_dir, "04-staging")
    verify_lock_uses_source_versions(
        layout.stage_crate / "Cargo.lock",
        layout.evidence_dir,
        "04b-staging-lock-version-consistency.log",
    )
    run_checked(
        ["cargo", "test", "--manifest-path", str(layout.stage_crate / "Cargo.toml"), "--locked"],
        cwd=layout.stage_crate,
        env=env,
        log_path=layout.evidence_dir / "05-staging-test.log",
    )
    run_checked(
        ["cargo", "package", "--manifest-path", str(layout.stage_crate / "Cargo.toml"), "--locked"],
        cwd=layout.stage_crate,
        env=env,
        log_path=layout.evidence_dir / "06-cargo-package.log",
    )
    produced = layout.target_dir / "staging/package" / layout.crate_file.name
    if not produced.is_file():
        raise RuntimeError(f"cargo package did not produce {produced}")
    layout.crate_file.parent.mkdir(parents=True)
    shutil.copy2(produced, layout.crate_file)


def verify_consumer(layout: PackageLayout, version: str) -> None:
    consumer = layout.package_root / "consumer-check"
    vendor = consumer / "vendor"
    unpacked = vendor / f"dms-client-{version}"
    src = consumer / "src"
    src.mkdir(parents=True)
    unpacked.parent.mkdir(parents=True)
    run_checked(
        ["tar", "-xzf", str(layout.crate_file), "-C", str(vendor)],
        cwd=layout.package_root,
        log_path=layout.evidence_dir / "07-unpack-crate.log",
    )
    if not unpacked.is_dir():
        raise RuntimeError(f"unpacked crate not found: {unpacked}")
    write_cargo_directory_checksum(unpacked, layout.crate_file)
    (consumer / "Cargo.toml").write_text(
        f"""[package]
name = "dms-sdk-consumer-check"
version = "0.0.0"
edition = "2024"

[dependencies]
dms-client = "{version}"

[patch.crates-io]
dms-client = {{ path = "vendor/dms-client-{version}" }}

[workspace]
""",
        encoding="utf-8",
    )
    # 与隔离安装 E2E 共用消费者：这里仅编译，连接/读写由 E2E 在真实服务上执行。
    # 消费者包括 KV、Hash 与公开错误类型，避免另建源码 path 依赖测试工程。
    shutil.copy2(SOURCE_ROOT / "scripts/release/consumer.rs", src / "main.rs")
    env = os.environ.copy()
    env["CARGO_TARGET_DIR"] = str(layout.target_dir / "consumer")
    stabilize_lock_to_source_versions(consumer, env, layout.evidence_dir, "08-consumer")
    verify_lock_uses_source_versions(
        consumer / "Cargo.lock",
        layout.evidence_dir,
        "08b-consumer-lock-version-consistency.log",
    )
    run_checked(
        ["cargo", "check", "--locked"],
        cwd=consumer,
        env=env,
        log_path=layout.evidence_dir / "09-consumer-check.log",
    )


def write_cargo_directory_checksum(unpacked: Path, crate_file: Path) -> None:
    files: dict[str, str] = {}
    for path in sorted(unpacked.rglob("*")):
        if path.is_dir() or path.name == ".cargo-checksum.json":
            continue
        relative = path.relative_to(unpacked).as_posix()
        files[relative] = hashlib.sha256(path.read_bytes()).hexdigest()
    checksum = {
        "files": files,
        "package": hashlib.sha256(crate_file.read_bytes()).hexdigest(),
    }
    (unpacked / ".cargo-checksum.json").write_text(
        json.dumps(checksum, sort_keys=True, indent=2) + "\n",
        encoding="utf-8",
    )


def write_manifest(layout: PackageLayout, version: str) -> None:
    crate_sha = subprocess.check_output(["sha256sum", str(layout.crate_file)], text=True).split()[0]
    packaged_files = subprocess.check_output(
        ["tar", "-tzf", str(layout.crate_file)],
        text=True,
    )
    crate_path = layout.crate_file.relative_to(SOURCE_ROOT).as_posix()
    staging_path = layout.stage_crate.relative_to(SOURCE_ROOT).as_posix()
    evidence_path = os.path.relpath(layout.evidence_dir, SOURCE_ROOT)
    manifest = {
        "package": "dms-client",
        "version": version,
        "crate": crate_path,
        "sha256": crate_sha,
        "staging_crate": staging_path,
        "evidence": evidence_path,
        "remote_publish": False,
        "consumer_dependency": f'dms-client = "{version}"',
        "internalized_crates": sorted(INTERNAL_CRATES),
        "pre_generated_protocol": "src/generated/dms.v1.rs",
        "features": {},
        "trace_reexport": None,
        "internalized_feature_policy": "source dms-client default graph only; dms-tracing runtime/test-support cfg blocks are inactive in the staging package",
    }
    (layout.package_root / "sdk-package-manifest.json").write_text(
        json.dumps(manifest, indent=2, ensure_ascii=False) + "\n",
        encoding="utf-8",
    )
    (layout.package_root / "README.md").write_text(
        f"""# dms-client SDK package candidate

- package: `{layout.crate_file.name}`
- version: `{version}`
- sha256: `{crate_sha}`
- staging crate: `{layout.stage_crate}`
- evidence: `{layout.evidence_dir}`
- machine manifest: `{layout.package_root / "sdk-package-manifest.json"}`

This is a local release candidate only. It was not uploaded to a remote registry.
SDK consumers should depend on `dms-client = "{version}"`; the private common
and protocol implementation is relocated into the packaged crate.
""",
        encoding="utf-8",
    )
    (layout.package_root / "package-files.txt").write_text(packaged_files, encoding="utf-8")


def build_package() -> PackageLayout:
    version = workspace_version()
    layout = unique_layout(version)
    layout.evidence_dir.mkdir(parents=True, exist_ok=True)
    generated = build_protocol_bindings(SOURCE_ROOT, layout.target_dir, layout.evidence_dir)
    stage_sdk_crate(layout, generated, version)
    package_and_verify(layout)
    verify_consumer(layout, version)
    write_manifest(layout, version)
    return layout


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--print-version", action="store_true", help="print the workspace SDK version and exit")
    args = parser.parse_args(argv)
    if args.print_version:
        print(workspace_version())
        return 0
    layout = build_package()
    print(f"SDK crate: {layout.crate_file}")
    print(f"Staging crate: {layout.stage_crate}")
    print(f"Evidence: {layout.evidence_dir}")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main(sys.argv[1:]))
    except Exception as exc:  # pragma: no cover - command-line failure path
        print(f"package_sdk.py: error: {exc}", file=sys.stderr)
        raise SystemExit(1)
