#!/usr/bin/env python3
"""Audit immutable fault receipts and fresh Linux identities/content."""
import hashlib
import importlib.util
import json
import pathlib

ROOT = pathlib.Path(__file__).resolve().parents[3]
EVIDENCE = ROOT / "evidence/afs-delivery"


def read(folder, name):
    return json.loads((EVIDENCE / folder / name).read_text())


spec = importlib.util.spec_from_file_location(
    "repair_fault_r2", ROOT / "experiments/afs-acceptance/repair-fault-v60-r2.py"
)
assert spec is not None and spec.loader is not None
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
fault = module.fault

run = read("repair-fault-v60-r2", "source-outage-run.json")
assert run["status"] == "PASS"
assert run["run_id"] == read("repair-fault-v60-r2", "preflight-run.json")["run_id"]
assert not list((EVIDENCE / "repair-fault-v60-r2").glob("*failure*"))
assert not list((EVIDENCE / "repair-fault-v60-r2").glob("*timeout*"))
assert not list((EVIDENCE / "repair-fault-v60").glob("target-outage*failure*"))
for name, expected in run["files"].items():
    path = EVIDENCE / "repair-fault-v60-r2" / name
    assert path.stat().st_size == expected["bytes"]
    assert hashlib.sha256(path.read_bytes()).hexdigest() == expected["sha256"], name
assert read("repair-runtime-v60", "prepare.complete.json")["candidate"] == "v60"
gate = read("repair-v60", "qualified-linux-clean-r2/qualified-linux-clean-r2/report.json")
assert gate["status"] == "PASS"
mapping = {path: sha for sha, path in (line.split() for line in
           (EVIDENCE / "repair-runtime-v60/build-artifact-sha256.txt").read_text().splitlines())}
staged = read("repair-runtime-v60", "staged-runtime-binary-sha256.json")
for role in ("node", "meta"):
    assert mapping[f"/home/lzc.guest/afs-build/artifacts/v60-qualified/afs-{role}"] == gate["binary_sha256"][f"afs-{role}"]
    assert mapping[f"/home/lzc.guest/afs-build/artifacts/v60-qualified-stripped/afs-{role}"] == staged["a"][role]
assert staged["a"]["node"] == staged["b"]["node"]

target = read("repair-fault-v60", "target-outage.complete.json")
source = read("repair-fault-v60-r2", "source-outage.complete.json")
assert target["status"] == source["status"] == "PASS"
degraded = read("repair-fault-v60", "target-outage-rest-degraded-available1.json")["polls"][-1]["json"]
assert degraded["available_copies"] == 1
assert any(t["state"] == "Pending" for t in degraded["tasks"])
assert not degraded["loss_confirmed"]
assert read("repair-fault-v60", "target-outage-a-fsynced-read.json")["sha256"] == target["file_sha256"]
assert "exit_code=0" in (EVIDENCE / "repair-fault-v60/target-outage-stop-b.stdout").read_text()
kill = read("repair-fault-v60-r2", "source-outage-a-sigkill-receipt.json")
assert kill["signal"] == "SIGKILL" and kill["proc_absent_within_2s"]
assert all(read("repair-fault-v60-r2", "source-outage-a-restore-identitychange-samecfgsha.json")["checks"].values())
starts = [json.loads((EVIDENCE / "repair-fault-v60-r2" / name).read_text())
          for name in run["files"] if name.startswith("command-") and name.endswith(".json")]
starts = [entry for entry in starts if entry["argv"][-2:] == ["start", "node"]]
assert len(starts) == 1 and starts[0]["returncode"] == 0
for backend, suffix in (("dfs", "mount-dfs"), ("ownerfs", "mount-ownerfs")):
    assert f"recovering disconnected {backend} mount at {fault.v55.RUN['a']}/{suffix}" in starts[0]["stderr"]
cold = read("repair-fault-v60-r2", "source-outage-b-immediate-fresh-read-after-a-kill.json")
assert cold["sha256"] == source["file_sha256"] and cold["bytes"] == 1048576
assert source["file_sha256"] != read("repair-fault-v60", "source-outage-create-a-exclusive.json")["sha256"]

identities = {}
for which, roles, expected in [
    ("a", ("meta", "node"), read("repair-fault-v60-r2", "source-outage-identity-a-after-restore.json")),
    ("b", ("node",), read("repair-v60", "session-expiry-recovery/session-expiry-identity-b-after-start.json")),
]:
    identities[which] = fault.live_identity(which, roles, f"root-independent-live-{which}.json")
    fault.assert_identity_matches(identities[which], expected, f"root live {which}")

proof_code = """
import hashlib,json,pathlib,urllib.request
base=pathlib.Path(RUN)
data=(base/'state/node/dfs/chunks'/CHUNK).read_bytes()
assert len(data)==1048576 and hashlib.sha256(data).hexdigest()==SHA
file_bytes=(base/'mount-dfs'/FILE).read_bytes()
assert file_bytes==data
print(json.dumps({'disk_sha256':hashlib.sha256(data).hexdigest(),'fuse_sha256':hashlib.sha256(file_bytes).hexdigest(),'bytes':len(data)}))
"""
proof = json.loads(fault.guest_py("b", fault.py_assignment(
    RUN=fault.v55.RUN["b"], CHUNK=source["chunk_id"], SHA=source["file_sha256"], FILE=fault.SOURCE_LOSS_FILE
) + proof_code, guest_timeout=20))
fault.dump_unique("root-independent-b-disk-and-fuse.json", proof)
fault.wait_rest(source["chunk_id"], "available2_completed", 15, "root-independent-final-rest.json")
fault.assert_old_v51_preserved("root-independent-final")

unrelated = read("repair-fault-v60-r2", "unrelated-snapshot-root-independent-final.json")
assert unrelated["status"] == "PASS"

manifest = read("repair-v60", "qualified-linux-clean-r2/qualified-linux-clean-r2/compile-inputs.json")
assert manifest["file_count"] == 143
for name, sha in manifest["files"].items():
    assert hashlib.sha256((ROOT / "source" / name).read_bytes()).hexdigest() == sha, name
handoff_sha = hashlib.sha256((ROOT / "source/docs/handoff.md").read_bytes()).hexdigest()
assert handoff_sha == "8941901e6b6f8b8b10e057a7477695dfa9b1fc4876b5762e71b66d8930eb7216"
controller = read("repair-v60", "controller-gate/report.json")
assert controller["status"] == "PASS"
controller_sha = hashlib.sha256((ROOT / "source/scripts/deploy/afs-processctl").read_bytes()).hexdigest()
assert controller_sha == controller["inputs"]["fixed"]["afs-processctl"]
install_b = read("repair-v60", "session-expiry-recovery/controller-install-b.json")
assert install_b["old_node_proc_absent"] and install_b["new_controller_sha256"] == controller_sha
restart_b = read("repair-v60", "session-expiry-recovery/session-expiry-b-identitychange-samebinarycfg.json")
b0, b1 = restart_b["before"], restart_b["after"]
assert b0["processes"]["node"]["sha256"] == b1["processes"]["node"]["sha256"]
assert b0["config_sha256"]["node"] == b1["config_sha256"]["node"]
assert (b0["processes"]["node"]["pid"], b0["processes"]["node"]["start_ticks"]) != (b1["processes"]["node"]["pid"], b1["processes"]["node"]["start_ticks"])
controller_code = "import hashlib,json,pathlib; p=pathlib.Path(PATH); print(json.dumps({'path':str(p),'sha256':hashlib.sha256(p.read_bytes()).hexdigest()}))"
staged_controller = {}
for which in ("a", "b"):
    item = json.loads(fault.guest_py(which, fault.py_assignment(PATH=fault.v55.RUN[which] + "/prefix/bin/afs-processctl") + controller_code, guest_timeout=10))
    assert item["sha256"] == controller_sha
    staged_controller[which] = item
fault.dump_unique("root-independent-controller.json", staged_controller)
report = {"status": "PASS", "run_id": run["run_id"], "source_input_count": 143, "target_outage": target, "source_outage": source,
          "live_identities": identities, "b_physical_and_fuse": proof, "unrelated_snapshot_unchanged": unrelated,
          "handoff_sha256": handoff_sha, "controller_sha256": controller_sha,
          "scope": "short memory/TLS/gRPC N2/M1 fault proof, not full release acceptance"}
fault.dump_unique("root-independent-audit-v2.json", report)
print(json.dumps({"status": report["status"], "source_input_count": 143, "handoff_sha256": handoff_sha}))
