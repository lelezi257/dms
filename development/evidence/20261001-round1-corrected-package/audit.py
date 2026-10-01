#!/usr/bin/env python3
"""Check retained round-1 evidence on Linux; not formal acceptance."""
import argparse
import json
import pathlib
import platform
import tomllib

NODE = "d28d17faa8bb71eb8c0a28241e8556396e440b204cf0242d2af8133700f6a494"
META = "64d85fc3933637ccd7fdf4eafded5e5c8cb2354e73b4c8d1a27b2b8d43f4c7e7"
CTL = "01f38d32f34994f89bea3c75f57eecc506db6157309e74f3fe5425f64775601e"
PACKAGE = "e13bc12e2d51ea70287a935c4f2a92b6b5313979ac94a2b080e0ec89fbaac8b5"
PAYLOAD = "7fa1366968edf9a8490fffac3eab7e0b1619480a152d8e75c7471f753a8c1231"


def evaluate(root):
    checks = []

    def need(name, condition):
        checks.append({"name": name, "ok": bool(condition)})
        if not condition:
            raise AssertionError(name)

    def read(path):
        return json.loads((root / path).read_text())

    def metrics(path, family, required):
        values = []
        for line in (root / path).read_text().splitlines():
            if line.startswith(family + "{") and all(v in line for v in required):
                values.append(float(line.rsplit(" ", 1)[1]))
        need(path + ": metric exists " + str(required), bool(values))
        return sum(values)

    for cohort, copies, required in (("r1", 1, 1), ("async", 2, 1)):
        for node in ("ctl", "a", "b", "c"):
            base = f"{cohort}/{node}/evidence/"
            install = read(base + "install.json")
            need(base + "archive", install["package_sha256"] == PACKAGE)
            need(base + "source", install["manifest"]["source_commit"] == "370cba91a15999a7edd7edb644a852c2b2dfd89b")
            need(base + "binaries", {k: v["sha256"] for k, v in install["manifest"]["binaries"].items()} == {"afs-node": NODE, "afs-meta": META})
            installed = (root / (base + "installed-binaries.txt")).read_text()
            need(base + "installed controller", CTL in installed and NODE in installed and META in installed)
            pre = read(base + "preflight.json")
            need(base + "environment", pre["volume"]["fstype"] == "ext4" and pre["cpu"] == 2 and bool(pre["boot_id"]) and pre["available_bytes"] >= 4 * 1024**3)
            role = "meta" if node == "ctl" else "node"
            config = tomllib.loads((root / (base + role + ".toml")).read_text())
            need(base + "policy", config["dfs_desired_copies"] == copies and config["dfs_sync_required_copies"] == required)
            need(base + "readiness", "ready=true" in (root / (base + "start.log")).read_text())
            if node == "ctl":
                need(base + "memory authority", config["meta_store"] == "memory")
            else:
                need(base + "independent mounts/Auto", config["ownerfs_mount"] != config["dfs_mount"] and config["data_mode"] == "auto" and config["rdma_device"] == "rxe0")
            if cohort == "r1":
                stopped = read(base + "stopped-status.json")
                need(base + "normal stop", stopped["state"] == "stopped" and stopped["exit_code"] == "0")
            else:
                identity = read(base + "process-identity.json")["identity"]
                process = identity["expected_processes"][role]
                need(base + "live process", identity["platform"]["system"] == "Linux" and identity["platform"]["machine"] == "aarch64" and process["sha256_ok"] and process["sha256"] == (META if role == "meta" else NODE))

    reused = read("reuse-inputs.json")
    need("frozen compiler inputs", reused["status"] == "PASS" and len(reused["compiler_inputs"]) == 143)
    need("controller binding", reused["controller"] == CTL)
    need("AGENTS and handoff unchanged", reused["protected"] == {"AGENTS.md": "539ff7119bf3cde494478921db7967e8c152c8ef64c9c43c3ee3a64e1bdb7f6f", "docs/handoff.md": "8941901e6b6f8b8b10e057a7477695dfa9b1fc4876b5762e71b66d8930eb7216"})

    for cohort in ("r1", "async"):
        home = read(f"{cohort}/a/evidence/workspace.json")["root"]
        need(cohort + " Home", home["home_node_id"] == "round1-a" and home["home_serving"])
        for node in ("b", "c"):
            for kind in ("owner", "dfs"):
                result = read(f"{cohort}/{node}/evidence/{kind}-cold-read.json")
                need(f"{cohort}/{node}/{kind} bytes", result["actual_sha256"] == PAYLOAD and result["bytes"] == 4194321)
    for kind in ("owner", "dfs"):
        result = read(f"r1/b/evidence/{kind}-restart-read.json")
        need(kind + " restart read", result["actual_sha256"] == PAYLOAD)
    need("normal restart", "stopped exit_code=0" in (root / "r1/b/evidence/restart.log").read_text() and "ready=true" in (root / "r1/b/evidence/restart.log").read_text())
    local = "r1/a/evidence/metrics-only-local.txt"
    for family in ("afs_dfs_payload_bytes_total", "afs_ownerfiles_payload_bytes_total"):
        need("R1 zero peer " + family, metrics(local, family, []) == 0)
    one = read("r1/a/evidence/r1-replicas.json")
    need("actual R1 placement", one["placement"]["desired_copies"] == 1 and one["available_copies"] == 1 and one["health"] == "Satisfied" and one["copies"][0]["record"]["location"]["Node"]["node_id"] == "round1-a")
    before = read("async/a/evidence/replication-before-join.json")
    need("async acknowledged local copy plus debt", before["placement"]["desired_copies"] == 2 and before["available_copies"] == 1 and before["health"] == "UnderReplicated" and any(t["state"] == "Pending" for t in before["tasks"]))
    repaired = read("async/a/evidence/repair-completed.json")
    need("two chunks repaired", len(repaired) == 2)
    for row in repaired:
        ready = [c["record"] for c in row["copies"] if c["available"]]
        names = {c["location"]["Node"]["node_id"] for c in ready}
        need(row["chunk_id"] + " receipts", len(names) >= 2 and row["available_copies"] >= 2 and row["health"] == "Satisfied" and all(c["role"] == "DurableReplica" and c["state"] == "Ready" for c in ready) and any(t["state"] == "Completed" for t in row["tasks"]))
        physical = []
        for name in names:
            matches = [p for p in read(f"async/{name.removeprefix('round1-')}/evidence/physical-chunks.json") if p["name"] == row["chunk_id"]]
            need(name + " physical receipt", len(matches) == 1 and matches[0]["size"] == next(c["persisted_bytes"] for c in ready if c["location"]["Node"]["node_id"] == name))
            physical.append(matches[0]["sha256"])
        need(row["chunk_id"] + " physical copies match", len(set(physical)) == 1)
    repair_bytes = sum(metrics(f"async/{n}/evidence/metrics-final.txt", "afs_dfs_payload_bytes_total", ['operation="replica"', 'transport="rdma"']) for n in "abc")
    need("actual RXE repaired bytes", repair_bytes >= 4194321)
    need("zero DFS gRPC fallback", sum(metrics(f"async/{n}/evidence/metrics-final.txt", "afs_dfs_payload_bytes_total", ['transport="grpc"']) for n in "abc") == 0)
    integration = read("consistency/report.json")
    need("whole-system consistency", integration["status"] == "PASS" and integration["summary"]["passed"] == 10 and integration["summary"]["failed"] == 0 and integration["identity"]["cross_mount_qualified"])
    for key in ("meta_before_suite", "meta_after_suite"):
        meta = integration["identity"][key]["identity"]["expected_processes"]["meta"]
        need(key, meta["sha256"] == META and meta["sha256_ok"])
    tests = read("r1/ctl/evidence/consistency-selftest.json")
    need("Linux probe regressions", tests["status"] == "PASS" and all(r["ok"] for r in tests["results"]))
    return checks


if __name__ == "__main__":
    assert platform.system() == "Linux", "Linux audit required"
    parser = argparse.ArgumentParser()
    parser.add_argument("root", type=pathlib.Path)
    args = parser.parse_args()
    result = evaluate(args.root)
    print(json.dumps({"status": "PASS", "level": "identified round-1 mainline", "checks": result, "formal_acceptance": "NOT_RUN", "environment": "PREPARING"}, indent=2))
