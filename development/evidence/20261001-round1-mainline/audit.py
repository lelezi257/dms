#!/usr/bin/env python3
"""Audit the recorded healthy subset on Linux; never promote formal cases."""
import hashlib
import json
import pathlib
import re
import sys
import tomllib

assert sys.platform == "linux"
root = pathlib.Path(__file__).parent
checks = []


def require(ok, label):
    if not ok:
        raise AssertionError(label)
    checks.append(label)


def record(path):
    return json.loads((root / path).read_text())


def metrics(path, transport, prefix):
    text = (root / path).read_text()
    samples = re.findall(r"^(afs_\w+_payload_bytes_total)\{([^}]+)\} (\d+)$", text, re.M)
    selected = [(tags, int(value)) for name, tags, value in samples if name.startswith(prefix) and (f'transport="{transport}"' in tags or f'plane="{transport}"' in tags)]
    require(bool(selected), str(path) + " metric samples exist")
    return sum(value for _, value in selected)


def replicas(path):
    for chunk in record(path)["chunks"]:
        require(chunk["placement"]["desired_copies"] == 2 and chunk["available_copies"] >= 2 and chunk["health"] == "Satisfied", str(path) + " healthy N2")
        copies = [c["record"] for c in chunk["copies"] if c["available"]]
        require(len({c["location"]["Node"]["node_id"] for c in copies}) >= 2, str(path) + " distinct Nodes")
        require(all(c["state"] == "Ready" and c["role"] == "DurableReplica" and c["persisted_bytes"] == int(chunk["chunk_id"].rsplit("-", 1)[1]) for c in copies), str(path) + " durable exact byte receipts")


for node, port in (("ctl", 19980), ("a", 19982), ("b", 19984), ("c", 19986)):
    info = record(f"{node}/install.json")
    require(info["package_sha256"] == "ccd21dcbe1cd0a153a13f06e84e730fc1f44811e973069fe2caffd759a50dad3", node + " initial package identity")
    require(record(f"{node}/preflight.json")["volume"]["fstype"] == "ext4", node + " guest ext4")
    status = record(f"{node}/final-status.json")
    require(status["state"] == "stopped" and status["exit_code"] == "0", node + " normal final stop")
    listeners = (root / node / "final-listeners.txt").read_text()
    require(not re.search(rf":({port}|{port + 1})\s", listeners), node + " owned listeners absent")
    require("round1-mainline-v77-rn/mount/" not in (root / node / "final-mounts.txt").read_text(), node + " owned mounts absent")
    installed = (root / node / "installed-programs.sha256").read_text()
    require("01f38d32f34994f89bea3c75f57eecc506db6157309e74f3fe5425f64775601e" in installed, node + " corrected controller identity")

cfg = tomllib.loads((root / "ctl/meta.toml").read_text())
require(cfg["meta_store"] == "memory" and cfg["dfs_desired_copies"] == cfg["dfs_sync_required_copies"] == 2, "central memory N2 policy")
home = record("a/workspace.json")["root"]
require(home["home_serving"] and home["home_node_id"] == "round1-a" and "192.168.109.12" in home["home_rest_addr"], "serving routable Home A")
replicas("a/replicas-after-write.json")
replicas("b/replicas-after-restart.json")
replicas("ctl/replicas-final.json")
old = record("r1-observed-policy.json")
require(old["placement"]["desired_copies"] == 1, "old R1 observation never qualifies RN")
require(metrics("a/metrics-owner-local.txt", "grpc", "afs_ownerfiles") == metrics("a/metrics-owner-local.txt", "rdma", "afs_ownerfiles") == 0, "Owner Home local zero-peer payload")
require(metrics("b/metrics-after-cold-read.txt", "rdma", "afs_dfs") == 4194321, "actual Auto replica receive bytes")
require(metrics("a/metrics-after-peer-reads.txt", "rdma", "afs_dfs") == 8388642, "actual Auto peer-send bytes")
for node in "abc":
    for prefix in ("afs_ownerfiles", "afs_dfs"):
        require(metrics(f"{node}/auto-final/metrics.txt", "grpc", prefix) == 0, node + " Auto no gRPC file payload")
        require(metrics(f"{node}/rdma-required/metrics.txt", "grpc", prefix) == 0, node + " required no gRPC file payload")
    for phase, mode in (("auto-final", "auto"), ("grpc", "grpc"), ("rdma-required", "rdma")):
        cfg = tomllib.loads((root / node / phase / "node.toml").read_text())
        require(cfg["data_mode"] == mode and cfg["dfs_sync_required_copies"] == 2, node + " " + phase + " config")
require(metrics("c/grpc/metrics.txt", "grpc", "afs_dfs") == 35, "actual gRPC replica receive")
require(metrics("b/grpc/metrics-r2.txt", "grpc", "afs_dfs") > 0, "actual gRPC peer send")
require(metrics("c/rdma-required/metrics.txt", "rdma", "afs_dfs") == 44, "actual required replica receive")
require(metrics("b/rdma-required/metrics.txt", "rdma", "afs_dfs") == 88, "actual required peer send")
require((root / "b/node-before-restart.identity").read_text() != (root / "b/node-after-restart.identity").read_text(), "B restart distinct incarnation")
for node in "ab":
    for kind in ("ownerfs", "dfs"):
        for suffix in ("same-mount", "close-only", "sparse-sync", "remote-close-reopen"):
            require(record(f"{node}/{kind}-{suffix}.json")["ok"], node + " " + kind + " " + suffix)
        suffix = "resize-write-r2" if kind == "ownerfs" else "resize-write"
        require(record(f"{node}/{kind}-{suffix}.json")["ok"], node + " " + kind + " truncate/write")
for path in ("a/owner-local-write.json", "b/owner-remote-write.json", "b/owner-after-restart.json", "b/dfs-after-restart.json", "c/owner-remote-read.json", "c/dfs-cold-read.json"):
    value = record(path)
    require(value["bytes"] == 4194321 and value["sha256"] == value["actual_sha256"] == "7fa1366968edf9a8490fffac3eab7e0b1619480a152d8e75c7471f753a8c1231", path + " exact payload")
require((root / "controller/original-failure.exit").read_text().strip() == "1", "original proxy regression fails")
require((root / "controller/fixed-regression.exit").read_text().strip() == "0" and "ok - processctl probes local readiness directly despite inherited proxy" in (root / "controller/fixed-regression.log").read_text(), "fixed proxy regression passes")
for path in (root / "gate/full").rglob("*.exit"):
    require(path.read_text().strip() == "0", str(path.relative_to(root)) + " passed")
require(len(list((root / "gate/full").rglob("*.exit"))) == 14, "complete 14-command source gate")
artifacts = {str(p.relative_to(root)): hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted(root.rglob("*")) if p.is_file() and p.name != "audit.json" and "__pycache__" not in p.parts}
print(json.dumps({"status": "PASS", "level": "recorded healthy-flow subset audit", "checks": checks, "artifacts": artifacts, "round_1": "INCOMPLETE", "formal_acceptance": "NOT_RUN", "environment": "PREPARING"}, indent=2))
