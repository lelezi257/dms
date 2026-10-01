#!/usr/bin/env python3
"""Install the qualified controller in isolated A and recover its dead mounts."""
import hashlib
import importlib.util
import json
import pathlib

ROOT = pathlib.Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location("repair_fault_v60", ROOT / "experiments/afs-acceptance/repair-fault-v60.py")
assert spec is not None and spec.loader is not None
wrapper = importlib.util.module_from_spec(spec)
spec.loader.exec_module(wrapper)
fault = wrapper.fault
previous = fault.OUT
fault.OUT = ROOT / "evidence/afs-delivery/repair-v60/controller-recovery"
assert not fault.OUT.exists(), "diagnostic recovery must use a fresh evidence directory"
fault.v55.OUT = fault.OUT
fault.ensure_out()
gate = json.loads((ROOT / "evidence/afs-delivery/repair-v60/controller-gate/report.json").read_text())
controller = ROOT / "source/scripts/deploy/afs-processctl"
controller_sha = hashlib.sha256(controller.read_bytes()).hexdigest()
assert gate["status"] == "PASS" and gate["inputs"]["fixed"]["afs-processctl"] == controller_sha
before = json.loads((previous / "source-outage-identity-a-before-sigkill.json").read_text())
chunk = json.loads((previous / "source-outage-chunk-id.json").read_text())
staged = "/home/lzc.guest/afs-processctl-repair-v60-qualified"
fault.call(["limactl", "copy", str(controller), "afs-accept-a:" + staged], timeout=30)
install = r"""
import hashlib,json,os,pathlib,shutil
root=pathlib.Path(RUN)
assert not pathlib.Path('/proc',str(OLD_PID)).exists()
src=pathlib.Path(STAGED)
assert hashlib.sha256(src.read_bytes()).hexdigest()==EXPECTED
dst=root/'prefix/bin/afs-processctl'
backup=root/'prefix/bin/afs-processctl.before-stale-recovery'
assert not backup.exists()
old_sha=hashlib.sha256(dst.read_bytes()).hexdigest()
shutil.copy2(dst,backup)
shutil.copy2(src,dst)
os.chmod(dst,0o755)
print(json.dumps({'old_sha256':old_sha,'new_sha256':hashlib.sha256(dst.read_bytes()).hexdigest(),'backup':str(backup),'old_node_proc_absent':True}))
"""
receipt = json.loads(fault.guest_py("a", fault.py_assignment(
    RUN=fault.v55.RUN["a"], OLD_PID=before["processes"]["node"]["pid"],
    STAGED=staged, EXPECTED=controller_sha) + install, guest_timeout=15))
fault.dump_unique("controller-install.json", receipt)
fault.restore_a_after_kill(before, chunk["chunk_id"])
fault.assert_old_v51_preserved("diagnostic-controller-recovery")
fault.dump_unique("diagnostic.complete.json", {"status": "PASS", "controller_sha256": controller_sha,
    "scope": "recover the previous failed run; original source-outage remains FAIL; full fresh case is r2"})
print(json.dumps({"status": "PASS", "scope": "diagnostic restore only", "controller_sha256": controller_sha}))
