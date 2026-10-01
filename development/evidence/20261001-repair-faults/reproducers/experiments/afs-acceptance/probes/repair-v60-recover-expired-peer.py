#!/usr/bin/env python3
"""Recover isolated B after its separately recorded lease-expiry exit."""
import hashlib
import importlib.util
import json
import pathlib

ROOT = pathlib.Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location("fault_v60", ROOT / "experiments/afs-acceptance/repair-fault-v60.py")
assert spec and spec.loader
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
fault = module.fault
previous = fault.OUT
fault.OUT = ROOT / "evidence/afs-delivery/repair-v60/session-expiry-recovery"
assert not fault.OUT.exists()
fault.v55.OUT = fault.OUT
fault.ensure_out()
before = json.loads((previous / "target-outage-identity-b-after-start.json").read_text())
controller = ROOT / "source/scripts/deploy/afs-processctl"
sha = hashlib.sha256(controller.read_bytes()).hexdigest()
gate = json.loads((ROOT / "evidence/afs-delivery/repair-v60/controller-gate/report.json").read_text())
assert gate["status"] == "PASS" and sha == gate["inputs"]["fixed"]["afs-processctl"]
staged = "/home/lzc.guest/afs-processctl-repair-v60-qualified"
fault.call(["limactl", "copy", str(controller), "afs-accept-b:" + staged], timeout=30)
code = r"""
import hashlib,json,os,pathlib,shutil
run=pathlib.Path(RUN)
assert not pathlib.Path('/proc',str(OLD_PID)).exists()
src=pathlib.Path(STAGED); assert hashlib.sha256(src.read_bytes()).hexdigest()==SHA
dst=run/'prefix/bin/afs-processctl'; backup=dst.with_name('afs-processctl.before-stale-recovery')
assert not backup.exists()
shutil.copy2(dst,backup); shutil.copy2(src,dst); os.chmod(dst,0o755)
print(json.dumps({'old_node_proc_absent':True,'new_controller_sha256':SHA,'backup':str(backup)}))
"""
receipt = json.loads(fault.guest_py("b", fault.py_assignment(RUN=fault.v55.RUN["b"],
    OLD_PID=before["processes"]["node"]["pid"], STAGED=staged, SHA=sha) + code, guest_timeout=15))
fault.dump_unique("controller-install-b.json", receipt)
fault.start_b_after_stop(before, "session-expiry")
chunk = json.loads((previous / "source-outage-chunk-id.json").read_text())
fault.wait_rest(chunk["chunk_id"], "available2_completed", 60, "rest-two-copies-restored.json")
fault.dump_unique("recovery.complete.json", {"status": "PASS", "scope": "B session-expiry recovery; prior diagnostic timeout remains failed"})
print(json.dumps({"status": "PASS", "scope": "B session-expiry recovery"}))
