#!/usr/bin/env python3
"""Repeat the source-loss case with a distinct cold file after controller repair."""
import importlib.util
import hashlib
import json
import pathlib
import uuid

spec = importlib.util.spec_from_file_location(
    "repair_fault_candidate", pathlib.Path(__file__).with_name("repair-fault-v60.py")
)
assert spec is not None and spec.loader is not None
wrapper = importlib.util.module_from_spec(spec)
spec.loader.exec_module(wrapper)
fault = wrapper.fault
previous_out = fault.OUT
fault.OUT = fault.ROOT / "evidence/afs-delivery/repair-fault-v60-r2"
fault.v55.OUT = fault.OUT
fault.SOURCE_LOSS_FILE = "repair-v60-source-loss-r2.bin"
assert fault.CREATE_SOURCE_FILE_CODE.count("((i*193)+41)%251") == 1
fault.CREATE_SOURCE_FILE_CODE = fault.CREATE_SOURCE_FILE_CODE.replace(
    "((i*193)+41)%251", "((i*193)+67)%251"
)


def current_preflight(phase):
    a = fault.live_identity("a", ("meta", "node"), f"identity-a-{phase}.json")
    b = fault.live_identity("b", ("node",), f"identity-b-{phase}.json")
    recovered = fault.ROOT / "evidence/afs-delivery/repair-v60/controller-recovery/source-outage-identity-a-after-restore.json"
    fault.assert_identity_matches(a, json.loads(recovered.read_text()), "A recovered baseline")
    peer_recovery = fault.ROOT / "evidence/afs-delivery/repair-v60/session-expiry-recovery/session-expiry-identity-b-after-start.json"
    fault.assert_identity_matches(b, json.loads(peer_recovery.read_text()), "B recovered session baseline")
    fault.assert_old_v51_preserved(phase)


fault.compare_live_with_baseline = current_preflight

# Historical Nodes exited on lease expiry while the previous failed run was
# being diagnosed. Do not claim their old PIDs are still alive or restart them.
# Bind this fresh run to their observed state, configs and identity artifacts.
UNRELATED_CODE = r"""
import pathlib,json,hashlib,os
run=pathlib.Path(RUN); result={'run':str(run),'roles':{}}
for role in ROLES:
    pid=int((run/'run'/(role+'.pid')).read_text()); proc=pathlib.Path('/proc',str(pid))
    entry={'pid':pid,'alive':proc.exists()}
    if proc.exists():
        fields=(proc/'stat').read_text().rsplit(') ',1)[1].split()
        entry.update(exe=os.readlink(proc/'exe'),ticks=fields[19],sha256=hashlib.sha256((proc/'exe').read_bytes()).hexdigest())
    entry['files']={}
    for part in ['etc/'+role+'.toml','prefix/bin/afs-'+role,'run/'+role+'.pid','run/'+role+'.identity']:
        p=run/part; entry['files'][part]=hashlib.sha256(p.read_bytes()).hexdigest() if p.exists() else None
    result['roles'][role]=entry
print(json.dumps(result))
"""


def assert_unrelated_snapshot_unchanged(phase):
    baseline_path = fault.ROOT / "evidence/afs-delivery/repair-v60/session-expiry-diagnostic/unrelated-before-r2.json"
    baseline = json.loads(baseline_path.read_text())
    current = {}
    for which, rows in baseline.items():
        current[which] = []
        for row in rows:
            item = json.loads(fault.guest_py(which, fault.py_assignment(
                RUN=row["run"], ROLES=list(row["roles"])) + UNRELATED_CODE, guest_timeout=15))
            if item != row:
                fault.dump_unique("unrelated-snapshot-mismatch-" + phase + ".json", {"before": row, "after": item})
                raise SystemExit("unrelated runtime state/artifacts changed during " + phase)
            current[which].append(item)
    fault.dump_unique("unrelated-snapshot-" + phase + ".json", {"status": "PASS", "snapshot": current,
        "scope": "fresh pre-r2 state unchanged; historical Nodes already exited, not a claim they remained alive"})


fault.assert_old_v51_preserved = assert_unrelated_snapshot_unchanged

if __name__ == "__main__":
    args = fault.parser().parse_args()
    if args.cmd == "plan":
        args.fn(args)
        raise SystemExit(0)
    if args.cmd == "preflight":
        if fault.OUT.exists():
            raise SystemExit("r2 evidence directory already exists; use a new run, never append a second preflight")
        fault.ensure_out()
        run_id = str(uuid.uuid4())
    elif args.cmd == "source-outage":
        preflight = json.loads((fault.OUT / "preflight-run.json").read_text())
        if preflight["status"] != "PASS" or list(fault.OUT.glob("source-outage*")):
            raise SystemExit("source-outage requires one successful fresh preflight and no previous attempt")
        run_id = preflight["run_id"]
    else:
        raise SystemExit("r2 supports only plan, preflight and source-outage")
    fault.dump_unique(args.cmd + "-run-start.json", {"run_id": run_id, "status": "RUNNING"})
    try:
        args.fn(args)
    except BaseException as exc:
        fault.dump_unique(args.cmd + "-run.json", {"run_id": run_id, "status": "FAIL", "error": repr(exc)})
        raise
    files = {p.name: {"sha256": hashlib.sha256(p.read_bytes()).hexdigest(), "bytes": p.stat().st_size}
             for p in sorted(fault.OUT.iterdir()) if p.is_file()}
    fault.dump_unique(args.cmd + "-run.json", {"run_id": run_id, "status": "PASS", "files": files})
