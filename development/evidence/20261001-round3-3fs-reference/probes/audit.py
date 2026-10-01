"""Linux-only semantic audit of the bounded patched-reference packet."""
import hashlib
import json
import pathlib
import re
import struct
import sys
import tarfile

assert sys.platform == "linux"
source = pathlib.Path(sys.argv[1])
packet = source / "development/evidence/20261001-round3-3fs-reference"
out = pathlib.Path(sys.argv[2])
out.mkdir(parents=True, exist_ok=True)
checks = []


def check(name, value):
    assert value, name
    checks.append(name)


def load(path):
    return json.loads(path.read_text())


data = b"".join(hashlib.sha256(f"round3-3fs-v84-{i}".encode()).digest() * 32768 for i in range(32))
digest = hashlib.sha256(data).hexdigest()
write = load(packet / "a/v84-write-r2.json")
check("write content", write["status"] == "PASS" and write["bytes"] == len(data) and write["observed_sha256"] == digest)
check("32 exact writes", len(write["writes_1mib"]) == 32 and all(w["status"] == "OK" and w["value"] == 1048576 and w["offset"] == i * 1048576 for i, w in enumerate(write["writes_1mib"])))
for label in ("fdatasync", "fsync_file", "directory_fsync"):
    check("syscall returned " + label, write[label]["status"] == "OK")
for rel in ("a/v84-read.json", "b/v84-read.json", "b/v84-read-after-restart.json"):
    r = load(packet / rel)
    check(rel + " full", r["status"] == "PASS" and r["bytes"] == len(data) and r["sha256"] == digest)
    check(rel + " count", len(r["fixed_ranges_exact"]) == 64)
    for i, v in enumerate(r["fixed_ranges_exact"]):
        size = 4096 if i % 2 == 0 else 65536
        offset = (i * 104729) % (len(data) - size)
        check(rel + f" range{i}", v["index"] == i and v["offset"] == offset and v["size"] == size and v["sha256"] == hashlib.sha256(data[offset:offset + size]).hexdigest())
resources = {}
for role in ("ctl", "a", "b", "c"):
    role_dir = packet / role
    stop = load(role_dir / "v84-stop-final.json")
    check(role + " stopped", stop["status"] == "PASS")
    owned = set()
    for item in stop["stopped"]:
        if "pid" in item:
            owned.add(item["pid"])
            check(role + " terminated " + str(item["pid"]), item.get("terminated") is True or item.get("already_terminated") is True)
            check(role + " no forced signal " + str(item["pid"]), not item.get("forced_kill", False) and "SIGKILL" not in item.get("signals_sent", []))
        if "mount" in item:
            unmount = item["umount"]
            check(role + " unmounted", unmount == "not_mounted" or isinstance(unmount, dict) and unmount["exit"] == 0)
    snapshot = load(role_dir / "v84-resource-stopped.json")
    check(role + " no requested processes", snapshot["status"] == "NO_PROCESS_TARGETS" and snapshot["processes"] == {})
    check(role + " verbs observed", snapshot["rdma"]["status"] == "OBSERVED")
    for kind, row in snapshot["rdma"]["resources"].items():
        check(role + " owned verbs gone " + kind, not any(v.get("pid") in owned for v in row["parsed"]))
    after = load(role_dir / "v84-resource-after.json")
    check(role + " snapshot stable", after["status"] == "PASS" and all(v["identity"]["stable_pre_post"] for v in after["processes"].values()))
    resources[role] = {k: v["status_fields"] for k, v in after["processes"].items()}
    with tarfile.open(role_dir / "runtime-raw.tar.gz", "r:gz") as archive:
        names = archive.getnames()
        check(role + " archive safe", all(not pathlib.PurePosixPath(n).is_absolute() and ".." not in pathlib.PurePosixPath(n).parts for n in names))
        for kind in ("config/", "evidence/", "log/", "run/"):
            check(role + " raw " + kind, any(kind in n for n in names))
        check(role + " no AppleDouble", not any(pathlib.PurePosixPath(n).name.startswith("._") for n in names))
    if role == "ctl":
        continue
    for stage, plan_rel in (("before", "preparation/physical-plan-before.json"), ("after", "preparation/physical-plan-after-r3.json")):
        plan = load(packet / plan_rel)
        check(stage + " layout", plan["chunk_size"] == 524288 and plan["inode"] == 75777 and len(plan["queries"]) == 64 and all(q["exit"] == 0 for q in plan["queries"]))
        slots = plan["roles"][role]
        proof = load(role_dir / f"v84-physical-{stage}.json")
        check(role + stage + " plan identity", proof["plan_sha256"] == hashlib.sha256((packet / plan_rel).read_bytes()).hexdigest())
        check(role + stage + " physical exact", proof["status"] == "PASS" and proof["replica_chunks_exact"] == 64 and proof["bytes_exact"] == len(data) and len(proof["receipts"]) == 64)
        check(role + stage + " unique slots", len({(e["path"], e["offset"]) for e in slots}) == 64)
        for i, (slot, receipt) in enumerate(zip(slots, proof["receipts"])):
            expected_id = struct.pack(">BBQHI", 0, 0, 75777, 0, i).hex()
            check(role + stage + f" chunk{i}", slot["index"] == i and slot["chunk_id"] == expected_id and all(receipt[k] == v for k, v in slot.items()) and slot["size"] == 524288 and slot["commit_version"] == slot["update_version"] and receipt["observed_sha256"] == receipt["expected_sha256"] == hashlib.sha256(data[i * 524288:(i + 1) * 524288]).hexdigest())
for rel in ("ctl/v84-fdb-status.json", "ctl/v84-fdb-status-after.json"):
    db = load(packet / rel)
    config = db["cluster"]["configuration"]
    check(rel + " durable engines", config["storage_engine"] == "ssd-2" and config["log_engine"] == "ssd-2" and config["redundancy_mode"] == "single" and db["client"]["database_status"]["available"])
    check(rel + " bounded process", all(p["memory"]["limit_bytes"] == 1073741824 for p in db["cluster"]["processes"].values()))
check("9 Linux helper regressions", (packet / "ctl/v84-driver-tests-r3.exit").read_text().strip() == "0" and re.search(r"Ran 9 tests.*\n\nOK", (packet / "ctl/v84-driver-tests-r3.log").read_text(), re.S))
check("4 negative audit regressions", (packet / "ctl/packet-audit-tests.exit").read_text().strip() == "0" and re.search(r"Ran 4 tests.*\n\nOK", (packet / "ctl/packet-audit-tests.log").read_text(), re.S))
inputs = load(source / "development/evidence/20261001-round2-closure/build/dfs-deadline-v82-final/compile-inputs-after.json")["files"]
check("143 unchanged compiler inputs", len(inputs) == 143 and all(hashlib.sha256((source / p).read_bytes()).hexdigest() == h for p, h in inputs.items()))
for p, digest_expected in {"AGENTS.md": "539ff7119bf3cde494478921db7967e8c152c8ef64c9c43c3ee3a64e1bdb7f6f", "docs/handoff.md": "8941901e6b6f8b8b10e057a7477695dfa9b1fc4876b5762e71b66d8930eb7216"}.items():
    check(p + " unchanged", hashlib.sha256((source / p).read_bytes()).hexdigest() == digest_expected)
links = 0
for md in [packet / "README.md"] + [source / p for p in ("docs/status.md", "development/plan.md", "development/issues.md")]:
    for target in re.findall(r"\]\(([^)]+)\)", md.read_text()):
        if "://" in target:
            continue
        name, _, anchor = target.partition("#")
        dest = (md.parent / name).resolve() if name else md
        if dest in (packet / "audit.json", packet / "artifact-hashes.json") and not dest.exists():
            continue
        check("link " + target, dest.exists())
        links += 1
        if anchor:
            headings = re.findall(r"^#+\s+(.*)$", dest.read_text(), re.M)
            check("anchor " + target, anchor in {re.sub(r"[^\w\- ]", "", h.lower()).replace(" ", "-") for h in headings})
files = {}
for p in sorted(packet.rglob("*")):
    check("no AppleDouble " + str(p.relative_to(packet)), not p.name.startswith("._"))
    if p.suffix == ".py":
        compile(p.read_bytes(), str(p), "exec")
    if p.is_file() and p.name not in ("audit.json", "artifact-hashes.json"):
        files[str(p.relative_to(packet))] = hashlib.sha256(p.read_bytes()).hexdigest()
(out / "artifact-hashes.json").write_text(json.dumps({"file_count": len(files), "files": files}, indent=2) + "\n")
result = {"status": "PASS", "scope": "patched-reference normal IO/physical copies/retained-state restart only", "checks": len(checks), "physical_slots_before": 192, "physical_slots_after": 192, "range_checks": 192, "artifacts": len(files), "local_links": links, "compiler_inputs_reused": 143, "process_resources": resources, "strong_durable_comparison": "BLOCKED", "formal_acceptance": "NOT_RUN", "environment": "PREPARING"}
(out / "audit.json").write_text(json.dumps(result, indent=2) + "\n")
print(json.dumps(result))
