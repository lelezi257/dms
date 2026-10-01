#!/usr/bin/env python3
"""Linux-only audit of the exact v69 observations; not an ENV qualifier."""
import hashlib
import importlib.util
import json
import platform
import sys
from pathlib import Path

if platform.system() != "Linux":
    raise SystemExit("Linux only")
root = Path(sys.argv[1])
spec = importlib.util.spec_from_file_location("env_verbs", root / "inputs/env_verbs.py")
probe = importlib.util.module_from_spec(spec)
spec.loader.exec_module(probe)
nodes = {"ctl": "192.168.109.11", "a": "192.168.109.12", "b": "192.168.109.13", "c": "192.168.109.14"}
checks = []

def check(name, condition, detail=None):
    checks.append({"name": name, "status": "PASS" if condition else "FAIL", "detail": detail})

def load(path):
    return json.loads(path.read_text())

def local_report(node, guest_path):
    return root / "runtime" / node / Path(guest_path).relative_to("/tmp/afs-verbs-v69")

matrix = load(root / "matrix.json")
check("frozen_probe", hashlib.sha256((root / "inputs/env_verbs.py").read_bytes()).hexdigest() == matrix["probe_sha256"])
runs = [json.loads(line) for line in (root / "runs.jsonl").read_text().splitlines()]
required = {(a, b, 256) for a in nodes for b in nodes if a != b} | {("a", "b", 65535)}
check("complete_matrix", len(runs) == 13 and {(r["client"], r["server"], r["size"]) for r in runs} == required)
probe_pids = {node: set() for node in nodes}
payloads = []
for r in runs:
    endpoints = {}
    for role in ("client", "server"):
        node = r[role]
        path = local_report(node, r[role + "_report"])
        ep = load(path)
        raw_path = path.with_name(path.name.replace(".json", ".raw.log"))
        raw = raw_path.read_bytes()
        check(r["label"] + ":" + role + ":raw", hashlib.sha256(raw).hexdigest() == ep["raw_log_sha256"] and raw.decode() == ep["raw_log"])
        check(r["label"] + ":" + role + ":scope", ep["bind"] == nodes[node] and ep["run_id"] == r["run_id"] and ep["role"] == role and ep["size"] == r["size"] and ep["count"] == 3 and ep["port"] == 19669)
        check(r["label"] + ":" + role + ":guest", ep["host"]["hostname"] == "lima-afs-accept-" + node and ep["host"]["machine"] == "aarch64" and ep["host"]["kernel"] == "6.8.0-142-generic")
        probe_pids[node].add(ep["process"]["pid"])
        endpoints[role] = ep
    result = probe.evaluate_pair(endpoints["client"], endpoints["server"])
    check(r["label"] + ":semantics", result["status"] == "PASS", result)
    payloads.append({"pair": r["label"], "size": r["size"], "read_sha256": endpoints["server"]["parsed"]["payload_sha256"],
                     "write_echo_sha256": endpoints["client"]["parsed"]["payload_sha256"], "control_descriptor_sha256": result["descriptor_sha256"]})

negative = load(root / "negative.json")
ep = load(local_report("a", negative["report"]))
probe_pids["a"].add(ep["process"]["pid"])
check("absent_listener", negative["returncode"] != 0 and ep["returncode"] != 0 and ep["bind"] == nodes["a"] and ep["peer"] == nodes["b"] and ep["port"] == 19669)
check("negative_no_payload", probe.parse_log(ep["raw_log"], "client", 256, 3)["payload_count"] == 0)

for node in nodes:
    before = load(root / "runtime" / node / "protected-before.json")
    after = load(root / "runtime" / node / "protected-after.json")
    check(node + ":guest_preserved", all(before[field] == after[field] for field in ("hostname", "kernel", "boot_id", "machine_id")))
    check(node + ":afs_processes_preserved", before["processes"] == after["processes"])
    check(node + ":afs_mounts_preserved", before["afs_mounts"] == after["afs_mounts"])
    for resource in ("qp", "cm_id"):
        observation = after["commands"][resource]
        entries = json.loads(observation["stdout"])
        check(node + ":no_owned_" + resource, observation["returncode"] == 0 and not any(item.get("pid") in probe_pids[node] for item in entries))

result = {"schema_version": 1, "scope": "standalone cross-VM rping preparation; not full ENV or product acceptance",
          "checks": checks, "payloads": payloads, "summary": {status: sum(c["status"] == status for c in checks) for status in ("PASS", "FAIL")}}
with (root / "audit.json").open("x") as f:
    json.dump(result, f, indent=2)
    f.write("\n")
print(json.dumps(result["summary"]))
raise SystemExit(1 if result["summary"]["FAIL"] else 0)
