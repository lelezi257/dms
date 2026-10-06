#!/usr/bin/env bash
set -euo pipefail

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
TMP=${TMPDIR:-/tmp}/afs-trial-config-test-$$
cleanup() {
  rm -rf "$TMP"
}
trap cleanup EXIT

pass() { printf 'ok - %s\n' "$1"; }

command -v openssl >/dev/null 2>&1 || {
  echo 'skip - afs-trial-config TLS test requires openssl'
  exit 0
}
command -v python3 >/dev/null 2>&1 || {
  echo 'skip - afs-trial-config structure test requires python3'
  exit 0
}

mkdir -p "$TMP"

SINGLE_CONFIG="$TMP/single/etc"
"$ROOT/afs-trial-config" single \
  --backend memory \
  --config-dir "$SINGLE_CONFIG" \
  --state-dir "$TMP/single/state" \
  --run-dir "$TMP/single/run" \
  --mount-root "$TMP/single/mnt" \
  --force >"$TMP/single.out"

CLUSTER_CONFIG="$TMP/install/etc"
CLUSTER_OUTPUT="$TMP/cluster"
"$ROOT/afs-trial-config" cluster \
  --backend local-file \
  --config-dir "$CLUSTER_CONFIG" \
  --state-dir "$TMP/cluster-state" \
  --run-dir "$TMP/cluster-run" \
  --mount-root "$TMP/cluster-mnt" \
  --meta-host 192.0.2.10 \
  --node node-a=192.0.2.11 \
  --node node-b=192.0.2.12 \
  --output "$CLUSTER_OUTPUT" \
  --force >"$TMP/cluster.out"

python3 - "$SINGLE_CONFIG" "$CLUSTER_CONFIG" "$CLUSTER_OUTPUT" <<'PY'
from __future__ import annotations

import sys
import tomllib
from pathlib import Path

single_config = Path(sys.argv[1])
cluster_config = Path(sys.argv[2])
cluster_output = Path(sys.argv[3])


def load_toml(path: Path) -> dict:
    with path.open("rb") as fh:
        return tomllib.load(fh)


def mapped_tls_path(configured: str, role_etc: Path, config_dir: Path) -> Path:
    path = Path(configured)
    try:
        rel = path.relative_to(config_dir)
    except ValueError:
        return path
    return role_etc / rel


def assert_exists(configured: str, role_etc: Path, config_dir: Path, label: str) -> None:
    mapped = mapped_tls_path(configured, role_etc, config_dir)
    if not mapped.exists():
        raise AssertionError(f"{label} references missing TLS file: {configured} -> {mapped}")


def check_trusted(cfg: dict, role_etc: Path, config_dir: Path, expected_nodes: set[str], label: str) -> None:
    trusted = cfg.get("trusted_node_certs")
    if set(trusted or {}) != expected_nodes:
        raise AssertionError(f"{label} trusted_node_certs mismatch: {trusted!r}")
    for node_id, configured in trusted.items():
        assert_exists(configured, role_etc, config_dir, f"{label} trusted {node_id}")


def check_policy(cfg: dict, expected: dict[str, object], label: str) -> None:
    keys = (
        "dfs_desired_copies",
        "dfs_sync_required_copies",
        "dfs_min_distinct_nodes",
        "dfs_min_distinct_failure_domains",
        "dfs_local_copy",
    )
    actual = {key: cfg.get(key) for key in keys}
    if actual != expected:
        raise AssertionError(f"{label} DFS policy mismatch: got {actual!r}, want {expected!r}")


def check_meta(role_etc: Path, config_dir: Path, expected_nodes: set[str], label: str) -> None:
    cfg = load_toml(role_etc / "meta.toml")
    if cfg.get("id") != "meta-ctl":
        raise AssertionError(f"{label} meta id mismatch: {cfg.get('id')!r}")
    assert_exists(cfg["tls_ca_certificate"], role_etc, config_dir, f"{label} meta ca")
    assert_exists(cfg["tls_identity_certificate"], role_etc, config_dir, f"{label} meta cert")
    assert_exists(cfg["tls_identity_private_key"], role_etc, config_dir, f"{label} meta key")
    if Path(cfg["tls_identity_private_key"]).name != "meta-key.pem":
        raise AssertionError(f"{label} meta private key mismatch: {cfg['tls_identity_private_key']}")
    check_trusted(cfg, role_etc, config_dir, expected_nodes, f"{label} meta")
    return cfg


def check_node(role_etc: Path, config_dir: Path, expected_id: str, expected_nodes: set[str], label: str, meta_key_must_be_absent: bool) -> dict:
    cfg = load_toml(role_etc / "node.toml")
    if cfg.get("id") != expected_id:
        raise AssertionError(f"{label} node id mismatch: got {cfg.get('id')!r}, want {expected_id!r}")
    assert_exists(cfg["tls_ca_certificate"], role_etc, config_dir, f"{label} node ca")
    assert_exists(cfg["tls_identity_certificate"], role_etc, config_dir, f"{label} node cert")
    assert_exists(cfg["tls_identity_private_key"], role_etc, config_dir, f"{label} node key")
    if Path(cfg["tls_identity_certificate"]).name != f"{expected_id}.pem":
        raise AssertionError(f"{label} node cert does not match id: {cfg['tls_identity_certificate']}")
    if Path(cfg["tls_identity_private_key"]).name != f"{expected_id}-key.pem":
        raise AssertionError(f"{label} node private key does not match id: {cfg['tls_identity_private_key']}")
    if "meta-key.pem" in cfg["tls_identity_private_key"]:
        raise AssertionError(f"{label} node references Meta private key")
    if meta_key_must_be_absent and (role_etc / "tls" / "meta-key.pem").exists():
        raise AssertionError(f"{label} node TLS directory contains Meta private key")
    check_trusted(cfg, role_etc, config_dir, expected_nodes, f"{label} node")
    return cfg


single_policy = {
    "dfs_desired_copies": 1,
    "dfs_sync_required_copies": 1,
    "dfs_min_distinct_nodes": 1,
    "dfs_min_distinct_failure_domains": 1,
    "dfs_local_copy": "required",
}
cluster_policy = {
    "dfs_desired_copies": 2,
    "dfs_sync_required_copies": 2,
    "dfs_min_distinct_nodes": 2,
    "dfs_min_distinct_failure_domains": 2,
    "dfs_local_copy": "required",
}

single_meta = check_meta(single_config, single_config, {"node-a"}, "single")
single_node = check_node(single_config, single_config, "node-a", {"node-a"}, "single", False)
check_policy(single_meta, single_policy, "single meta")
check_policy(single_node, single_policy, "single node")
if single_node.get("allow_volatile_meta") is not True:
    raise AssertionError("single memory config must set allow_volatile_meta=true")

meta_etc = cluster_output / "meta" / "etc"
node_a_etc = cluster_output / "node-a" / "etc"
node_b_etc = cluster_output / "node-b" / "etc"
expected = {"node-a", "node-b"}
cluster_meta = check_meta(meta_etc, cluster_config, expected, "cluster")
node_a = check_node(node_a_etc, cluster_config, "node-a", expected, "cluster node-a", True)
node_b = check_node(node_b_etc, cluster_config, "node-b", expected, "cluster node-b", True)
check_policy(cluster_meta, cluster_policy, "cluster meta")
check_policy(node_a, cluster_policy, "cluster node-a")
check_policy(node_b, cluster_policy, "cluster node-b")
if node_a["id"] == node_b["id"]:
    raise AssertionError("cluster node-a and node-b must have distinct ids")
if node_a["tls_identity_private_key"] == node_b["tls_identity_private_key"]:
    raise AssertionError("cluster node-a and node-b must have distinct private keys")
if node_a.get("allow_volatile_meta") is not False or node_b.get("allow_volatile_meta") is not False:
    raise AssertionError("cluster local-file config must keep allow_volatile_meta=false")
PY

pass "afs-trial-config single and cluster TLS/id structure"
