#!/usr/bin/env python3
"""Host coordinator for v56 DFS repair fault probes.

This script reuses the isolated repair-v55 A/B runtime and the helper
constants from repair-runtime-v55.py, but writes only new immutable evidence
under evidence/afs-delivery/repair-fault-v56.

It is intentionally a host-side orchestrator.  Product processes,
filesystem reads/writes, process faults, process identity checks, local
chunk checksums, and REST observations all run inside the Linux Lima VMs.
"""

from __future__ import annotations

import argparse
import importlib.util
import json
import pathlib
import subprocess
import textwrap
import time
from typing import Any


ROOT = pathlib.Path(__file__).resolve().parents[2]
OUT = ROOT / "evidence/afs-delivery/repair-fault-v56"
BASELINE_OUT = ROOT / "evidence/afs-delivery/repair-runtime-v55"
V55_PATH = pathlib.Path(__file__).with_name("repair-runtime-v55.py")

NODE_SHA = "8cfea1e4a3b3df1498620f4d3b1420d0b9621fe8415bc8b7df2be838af388023"
META_SHA = "f08de71b873f9e9f5564acdd09ce58ab62b62472b36db7a8b0a514c2d47849fb"
ORIGINAL_FILE = "repair-v55-deterministic-1m.bin"
SOURCE_LOSS_FILE = "repair-v56-source-loss.bin"
TEST_SIZE = 1024 * 1024


def load_v55() -> Any:
    spec = importlib.util.spec_from_file_location("repair_runtime_v55", V55_PATH)
    if spec is None or spec.loader is None:
        raise SystemExit(f"cannot import {V55_PATH}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    module.OUT = OUT
    return module


v55 = load_v55()


def ensure_out() -> None:
    OUT.mkdir(parents=True, exist_ok=True)


def unique_path(name: str) -> pathlib.Path:
    ensure_out()
    path = OUT / name
    if not path.exists():
        return path
    stem = path.name[: -len(path.suffix)] if path.suffix else path.name
    suffix = path.suffix
    for index in range(2, 10000):
        candidate = path.with_name(f"{stem}.{index}{suffix}")
        if not candidate.exists():
            return candidate
    raise RuntimeError(f"too many evidence files for {name}")


def dump_unique(name: str, obj: Any) -> pathlib.Path:
    path = unique_path(name)
    path.write_text(json.dumps(obj, indent=2, sort_keys=True) + "\n")
    return path


def write_unique(name: str, text: str) -> pathlib.Path:
    path = unique_path(name)
    path.write_text(text)
    return path


def call(argv: list[str], timeout: int) -> str:
    ensure_out()
    started = time.time()
    try:
        proc = subprocess.run(argv, text=True, capture_output=True, timeout=timeout)
    except subprocess.TimeoutExpired as exc:
        entry = {
            "time_unix_ms": int(started * 1000),
            "duration_ms": int((time.time() - started) * 1000),
            "argv": argv,
            "timeout_seconds": timeout,
            "returncode": None,
            "stdout": exc.stdout.decode(errors="replace") if isinstance(exc.stdout, bytes) else (exc.stdout or ""),
            "stderr": exc.stderr.decode(errors="replace") if isinstance(exc.stderr, bytes) else (exc.stderr or ""),
            "timeout_expired": True,
        }
        dump_unique(f"command-timeout-{int(started * 1000)}.json", entry)
        raise
    entry = {
        "time_unix_ms": int(started * 1000),
        "duration_ms": int((time.time() - started) * 1000),
        "argv": argv,
        "timeout_seconds": timeout,
        "returncode": proc.returncode,
        "stdout": proc.stdout,
        "stderr": proc.stderr,
    }
    dump_unique(f"command-{int(started * 1000)}.json", entry)
    if proc.returncode:
        dump_unique(f"command-failure-{int(started * 1000)}.json", entry)
        raise RuntimeError(json.dumps({**entry, "stdout": proc.stdout[-4000:], "stderr": proc.stderr[-4000:]}, indent=2))
    return proc.stdout


def guest_py(which: str, code: str, *, guest_timeout: int, outer_timeout: int | None = None) -> str:
    outer = outer_timeout if outer_timeout is not None else guest_timeout + 15
    return call(
        [
            "limactl",
            "shell",
            v55.VM[which],
            "--",
            "sudo",
            "timeout",
            "--kill-after=2s",
            f"{guest_timeout}s",
            "python3",
            "-c",
            code,
        ],
        timeout=outer,
    )


def ctl(which: str, *args: str, timeout: int = 90) -> str:
    run = v55.RUN[which]
    return call(
        [
            "limactl",
            "shell",
            v55.VM[which],
            "--",
            "sudo",
            f"{run}/prefix/bin/afs-processctl",
            "--prefix",
            f"{run}/prefix",
            "--config-dir",
            f"{run}/etc",
            "--run-dir",
            f"{run}/run",
            "--log-dir",
            f"{run}/log",
            "--timeout",
            "20",
            *args,
        ],
        timeout=timeout,
    )


def read_baseline(name: str) -> Any:
    return json.loads((BASELINE_OUT / name).read_text())


IDENTITY_CODE = r"""
import hashlib, json, os, pathlib, platform, subprocess
run=pathlib.Path(RUN)
roles=ROLES
expected=EXPECTED
result={'runtime':str(run),'platform':platform.platform(),'boot_id':pathlib.Path('/proc/sys/kernel/random/boot_id').read_text().strip(),'processes':{},'config_sha256':{},'mounts':{}}
for role in roles:
    pid=int((run/'run'/(role+'.pid')).read_text())
    proc=pathlib.Path('/proc')/str(pid)
    fields=(proc/'stat').read_text().rsplit(') ',1)[1].split()
    exe=os.readlink(proc/'exe')
    sha=hashlib.sha256((proc/'exe').read_bytes()).hexdigest()
    expected_exe=str(run/'prefix/bin'/('afs-'+role))
    assert exe == expected_exe, {'role':role,'exe':exe,'expected':expected_exe}
    assert sha == expected[role], {'role':role,'sha256':sha,'expected':expected[role]}
    result['processes'][role]={'pid':pid,'start_ticks':fields[19],'state':fields[0],'exe':exe,'sha256':sha}
    result['config_sha256'][role]=hashlib.sha256((run/'etc'/(role+'.toml')).read_bytes()).hexdigest()
for name in ('mount-ownerfs','mount-dfs'):
    target=str(run/name)
    data=json.loads(subprocess.check_output(['findmnt','-J','-M',target,'-o','TARGET,SOURCE,FSTYPE,OPTIONS'],text=True))
    mount=data['filesystems'][0]
    expected_source='afs-ownerfs' if name == 'mount-ownerfs' else 'afs-dfs'
    assert mount['target'] == target, mount
    assert mount['source'] == expected_source, mount
    assert str(mount['fstype']).startswith('fuse'), mount
    result['mounts'][name]=mount
result['data_volume']=json.loads(subprocess.check_output(['findmnt','-J','-T',str(run),'-o','TARGET,SOURCE,FSTYPE'],text=True))['filesystems'][0]
assert result['data_volume']['fstype'] == 'ext4', result['data_volume']
print(json.dumps(result))
"""


OLD_IDENTITY_CODE = r"""
import hashlib, json, os, pathlib
run=pathlib.Path(RUN)
roles=ROLES
result={'runtime':str(run),'processes':{},'config_sha256':{}}
for role in roles:
    pid=int((run/'run'/(role+'.pid')).read_text())
    proc=pathlib.Path('/proc')/str(pid)
    fields=(proc/'stat').read_text().rsplit(') ',1)[1].split()
    exe=os.readlink(proc/'exe')
    sha=hashlib.sha256((proc/'exe').read_bytes()).hexdigest()
    result['processes'][role]={'pid':pid,'start_ticks':fields[19],'exe':exe,'sha256':sha}
    result['config_sha256'][role]=hashlib.sha256((run/'etc'/(role+'.toml')).read_bytes()).hexdigest()
print(json.dumps(result))
"""


def py_assignment(**items: Any) -> str:
    return "".join(f"{key}={value!r}\n" for key, value in items.items())


def expected_hashes(roles: tuple[str, ...]) -> dict[str, str]:
    return {role: (META_SHA if role == "meta" else NODE_SHA) for role in roles}


def live_identity(which: str, roles: tuple[str, ...], name: str) -> dict[str, Any]:
    code = py_assignment(RUN=v55.RUN[which], ROLES=list(roles), EXPECTED=expected_hashes(roles)) + IDENTITY_CODE
    data = json.loads(guest_py(which, code, guest_timeout=20, outer_timeout=35))
    dump_unique(name, data)
    return data


def old_identity(which: str, roles: tuple[str, ...], name: str) -> dict[str, Any]:
    code = py_assignment(RUN=v55.TEMPLATE[which], ROLES=list(roles)) + OLD_IDENTITY_CODE
    data = json.loads(guest_py(which, code, guest_timeout=15, outer_timeout=30))
    dump_unique(name, data)
    return data


def assert_identity_matches(current: dict[str, Any], baseline: dict[str, Any], label: str) -> None:
    for role, proc in baseline["processes"].items():
        for key in ("pid", "start_ticks", "exe", "sha256"):
            if current["processes"][role][key] != proc[key]:
                raise SystemExit(f"{label} {role} {key} changed: {current['processes'][role][key]} != {proc[key]}")
    if current["config_sha256"] != baseline["config_sha256"]:
        raise SystemExit(f"{label} config SHA changed")
    for name, mount in baseline.get("mounts", {}).items():
        current_mount = current["mounts"][name]
        for key in ("target", "source", "fstype"):
            if current_mount[key] != mount[key]:
                raise SystemExit(f"{label} mount {name} {key} changed")


def assert_old_v51_preserved(phase: str) -> None:
    checks = {
        "a": (("meta", "node"), "old-v51-a-before.json"),
        "b": (("node",), "old-v51-b-before.json"),
    }
    report: dict[str, Any] = {}
    for which, (roles, baseline_name) in checks.items():
        baseline = read_baseline(baseline_name)
        current = old_identity(which, roles, f"old-v51-{which}-{phase}.json")
        if current["processes"] != baseline["processes"] or current["config_sha256"] != baseline["config_sha256"]:
            dump_unique(f"old-v51-{which}-mismatch-{phase}.json", {"baseline": baseline, "current": current})
            raise SystemExit(f"old v51 {which} changed during {phase}")
        report[which] = current
    dump_unique(f"old-v51-preserved-{phase}.json", report)


def compare_live_with_baseline(phase: str) -> None:
    a = live_identity("a", ("meta", "node"), f"identity-a-{phase}.json")
    b = live_identity("b", ("node",), f"identity-b-{phase}.json")
    assert_identity_matches(a, read_baseline("identity-a-after-verify.2.json"), "A live baseline")
    assert_identity_matches(b, read_baseline("identity-b-after-verify.2.json"), "B live baseline")
    assert_old_v51_preserved(phase)


REST_WAIT_CODE = r"""
import json, time, urllib.parse, urllib.request
chunk_id=CHUNK_ID
deadline=time.monotonic()+BOUND
polls=[]
def task_states(payload):
    return [task.get('state') for task in payload.get('tasks',[]) if isinstance(task,dict) and task.get('chunk_id') == chunk_id]
def ok(payload):
    states=task_states(payload)
    if MODE == 'available1_not_completed':
        return payload.get('available_copies') == 1 and any(state != 'Completed' for state in states)
    if MODE == 'available2_completed':
        return payload.get('available_copies') == 2 and any(state == 'Completed' for state in states)
    raise AssertionError(MODE)
while True:
    try:
        url='http://127.0.0.1:'+str(META_REST)+'/v1/dfs/chunks/'+urllib.parse.quote(chunk_id, safe='')+'/replication'
        with urllib.request.urlopen(url, timeout=3) as resp:
            payload=json.loads(resp.read().decode())
            item={'time_unix_ms':int(time.time()*1000),'status':resp.status,'json':payload}
            polls.append(item)
            if ok(payload):
                print(json.dumps({'status':'satisfied','mode':MODE,'chunk_id':chunk_id,'bound_seconds':BOUND,'polls':polls}))
                raise SystemExit(0)
    except Exception as exc:
        polls.append({'time_unix_ms':int(time.time()*1000),'error':repr(exc)})
    if time.monotonic() >= deadline:
        raise SystemExit(json.dumps({'status':'timeout','mode':MODE,'chunk_id':chunk_id,'bound_seconds':BOUND,'polls':polls}, indent=2))
    time.sleep(1)
"""


def wait_rest(chunk_id: str, mode: str, bound: int, name: str) -> dict[str, Any]:
    code = py_assignment(CHUNK_ID=chunk_id, MODE=mode, BOUND=bound, META_REST=v55.PORTS["meta_rest"]) + REST_WAIT_CODE
    result = json.loads(guest_py("a", code, guest_timeout=bound + 5, outer_timeout=bound + 20))
    dump_unique(name, result)
    return result


READ_FILE_CODE = r"""
import hashlib, json, pathlib
target=pathlib.Path(RUN)/'mount-dfs'/FILE_NAME
data=target.read_bytes()
sha=hashlib.sha256(data).hexdigest()
assert len(data) == EXPECTED_SIZE, {'path':str(target),'size':len(data),'expected_size':EXPECTED_SIZE}
if EXPECTED_SHA:
    assert sha == EXPECTED_SHA, {'path':str(target),'sha256':sha,'expected_sha256':EXPECTED_SHA}
print(json.dumps({'path':str(target),'bytes':len(data),'sha256':sha}))
"""


def read_fuse(which: str, file_name: str, expected_sha: str, name: str, bound: int = 30) -> dict[str, Any]:
    code = py_assignment(
        RUN=v55.RUN[which],
        FILE_NAME=file_name,
        EXPECTED_SIZE=TEST_SIZE,
        EXPECTED_SHA=expected_sha,
    ) + READ_FILE_CODE
    result = json.loads(guest_py(which, code, guest_timeout=bound, outer_timeout=bound + 15))
    dump_unique(name, result)
    return result


CREATE_SOURCE_FILE_CODE = r"""
import hashlib, json, os, pathlib
run=pathlib.Path(RUN)
target=run/'mount-dfs'/FILE_NAME
data=bytes(((i*193)+41)%251 for i in range(SIZE))
with open(target,'xb') as fh:
    fh.write(data)
    fh.flush()
    os.fsync(fh.fileno())
digest=hashlib.sha256(data).hexdigest()
chunk_root=run/'state/node/dfs/chunks'
inventory=[]
matches=[]
for path in sorted(chunk_root.iterdir()):
    if not path.is_file():
        continue
    size=path.stat().st_size
    if size <= 0 or size > 128*1024*1024:
        continue
    sha=hashlib.sha256(path.read_bytes()).hexdigest()
    item={'path':str(path.relative_to(run)),'name':path.name,'size':size,'sha256':sha}
    inventory.append(item)
    if size == len(data) and sha == digest:
        matches.append(item)
if len(matches) != 1:
    raise SystemExit(json.dumps({'error':'expected exactly one physical chunk match for new source-loss file','file_sha256':digest,'matches':matches,'inventory':inventory}, indent=2))
print(json.dumps({'file':str(target),'size':len(data),'sha256':digest,'chunk':matches[0],'inventory':inventory}))
"""


def create_source_loss_file() -> dict[str, Any]:
    code = py_assignment(RUN=v55.RUN["a"], FILE_NAME=SOURCE_LOSS_FILE, SIZE=TEST_SIZE) + CREATE_SOURCE_FILE_CODE
    result = json.loads(guest_py("a", code, guest_timeout=30, outer_timeout=45))
    dump_unique("source-outage-create-a-exclusive.json", result)
    dump_unique("source-outage-chunk-id.json", {"chunk_id": result["chunk"]["name"], "file_sha256": result["sha256"]})
    return result


KILL_A_CODE = r"""
import hashlib, json, os, pathlib, signal, time
run=pathlib.Path(RUN)
pid=int((run/'run/node.pid').read_text())
proc=pathlib.Path('/proc')/str(pid)
fields=(proc/'stat').read_text().rsplit(') ',1)[1].split()
exe=os.readlink(proc/'exe')
sha=hashlib.sha256((proc/'exe').read_bytes()).hexdigest()
receipt={'pid':pid,'start_ticks':fields[19],'exe':exe,'sha256':sha,'expected':EXPECTED}
assert pid == EXPECTED['pid'], receipt
assert fields[19] == EXPECTED['start_ticks'], receipt
assert exe == EXPECTED['exe'], receipt
assert sha == EXPECTED['sha256'], receipt
os.kill(pid, signal.SIGKILL)
deadline=time.monotonic()+2
absent=False
while time.monotonic() < deadline:
    if not proc.exists():
        absent=True
        break
    time.sleep(0.05)
receipt['signal']='SIGKILL'
receipt['proc_absent_within_2s']=absent
if not absent:
    raise SystemExit(json.dumps(receipt, indent=2))
print(json.dumps(receipt))
"""


def kill_a_node_exact(before: dict[str, Any]) -> dict[str, Any]:
    expected = before["processes"]["node"]
    code = py_assignment(RUN=v55.RUN["a"], EXPECTED=expected) + KILL_A_CODE
    result = json.loads(guest_py("a", code, guest_timeout=5, outer_timeout=20))
    dump_unique("source-outage-a-sigkill-receipt.json", result)
    return result


def start_b_after_stop(before: dict[str, Any], phase: str) -> dict[str, Any]:
    out = ctl("b", "start", "node", timeout=90)
    write_unique(f"{phase}-start-b.stdout", out)
    after = live_identity("b", ("node",), f"{phase}-identity-b-after-start.json")
    b0 = before["processes"]["node"]
    b1 = after["processes"]["node"]
    if (b0["pid"], b0["start_ticks"]) == (b1["pid"], b1["start_ticks"]):
        raise SystemExit(f"{phase}: B process identity did not change")
    if b0["sha256"] != b1["sha256"] or before["config_sha256"]["node"] != after["config_sha256"]["node"]:
        raise SystemExit(f"{phase}: B binary/config changed")
    dump_unique(f"{phase}-b-identitychange-samebinarycfg.json", {"before": before, "after": after})
    return after


def restore_a_after_kill(before: dict[str, Any], chunk_id: str) -> dict[str, Any]:
    start_out = ctl("a", "start", "node", timeout=90)
    write_unique("source-outage-restore-a-start-node.stdout", start_out)
    after = live_identity("a", ("meta", "node"), "source-outage-identity-a-after-restore.json")
    node0 = before["processes"]["node"]
    node1 = after["processes"]["node"]
    meta0 = before["processes"]["meta"]
    meta1 = after["processes"]["meta"]
    checks = {
        "node_identity_changed": (node0["pid"], node0["start_ticks"]) != (node1["pid"], node1["start_ticks"]),
        "node_same_binary": node0["sha256"] == node1["sha256"],
        "node_same_config": before["config_sha256"]["node"] == after["config_sha256"]["node"],
        "meta_identity_unchanged": (meta0["pid"], meta0["start_ticks"], meta0["sha256"]) == (meta1["pid"], meta1["start_ticks"], meta1["sha256"]),
        "meta_same_config": before["config_sha256"]["meta"] == after["config_sha256"]["meta"],
    }
    dump_unique("source-outage-a-restore-identitychange-samecfgsha.json", {"before": before, "after": after, "checks": checks})
    if not all(checks.values()):
        raise SystemExit(f"A restore identity/config checks failed: {checks}")
    rest = wait_rest(chunk_id, "available2_completed", 120, "source-outage-rest-available2-after-a-restore.json")
    dump_unique("source-outage-recoveredcopy-currentservingepoch.json", {"rest": rest["polls"][-1]["json"], "identity": after})
    return after


def cmd_preflight(_: argparse.Namespace) -> None:
    ensure_out()
    compare_live_with_baseline("preflight")
    dump_unique(
        "preflight.complete.json",
        {
            "status": "PASS",
            "runtime": v55.RUN,
            "ports": v55.PORTS,
            "expected_sha256": {"node": NODE_SHA, "meta": META_SHA},
            "baseline_evidence": str(BASELINE_OUT),
            "new_evidence": str(OUT),
            "note": "all live product identities, mounts, old-v51 identities, and checksums were verified inside Linux guests",
        },
    )


def cmd_target_outage(_: argparse.Namespace) -> None:
    chunk = read_baseline("repair-chunk-id.json")
    chunk_id = chunk["chunk_id"]
    expected_sha = chunk["file_sha256"]
    failure: BaseException | None = None
    before = live_identity("b", ("node",), "target-outage-identity-b-before-stop.json")
    try:
        write_unique("target-outage-stop-b.stdout", ctl("b", "stop", "node", timeout=90))
        wait_rest(chunk_id, "available1_not_completed", 60, "target-outage-rest-degraded-available1.json")
        read_fuse("a", ORIGINAL_FILE, expected_sha, "target-outage-a-fsynced-read.json", bound=30)
    except BaseException as exc:  # preserve restore path even for SystemExit
        failure = exc
        dump_unique("target-outage.failure.json", {"error": repr(exc)})
    finally:
        try:
            start_b_after_stop(before, "target-outage")
            wait_rest(chunk_id, "available2_completed", 120, "target-outage-rest-available2-after-b-restore.json")
            assert_old_v51_preserved("target-outage-final")
        except BaseException as restore_exc:
            dump_unique("target-outage.restore-failure.json", {"error": repr(restore_exc)})
            if failure is None:
                failure = restore_exc
    if failure is not None:
        raise failure
    dump_unique("target-outage.complete.json", {"status": "PASS", "chunk_id": chunk_id, "file_sha256": expected_sha})


def cmd_source_outage(_: argparse.Namespace) -> None:
    created = create_source_loss_file()
    chunk_id = created["chunk"]["name"]
    expected_sha = created["sha256"]
    wait_rest(chunk_id, "available2_completed", 120, "source-outage-rest-available2-before-fault.json")
    before_a = live_identity("a", ("meta", "node"), "source-outage-identity-a-before-sigkill.json")
    failure: BaseException | None = None
    try:
        kill_a_node_exact(before_a)
        read_fuse("b", SOURCE_LOSS_FILE, expected_sha, "source-outage-b-immediate-fresh-read-after-a-kill.json", bound=30)
    except BaseException as exc:
        failure = exc
        dump_unique("source-outage.failure.json", {"error": repr(exc)})
    finally:
        try:
            restore_a_after_kill(before_a, chunk_id)
            assert_old_v51_preserved("source-outage-final")
        except BaseException as restore_exc:
            dump_unique("source-outage.restore-failure.json", {"error": repr(restore_exc)})
            if failure is None:
                failure = restore_exc
    if failure is not None:
        raise failure
    dump_unique("source-outage.complete.json", {"status": "PASS", "chunk_id": chunk_id, "file_sha256": expected_sha})


def cmd_plan(_: argparse.Namespace) -> None:
    print(
        textwrap.dedent(
            f"""
            v56 repair fault probe stages:
              preflight      compare live isolated repair-v55 A/B runtime against v55 post-verify baseline and old-v51 identities
              target-outage  stop only isolated B Node; require REST degraded available1/task-not-completed; prove A bytes; restore B and require Completed available2
              source-outage  create exclusive {SOURCE_LOSS_FILE}; wait two copies; SIGKILL exact A Node; immediately read from B; restore A with ctl start node and require two copies

            Runtime paths stay unchanged:
              A: {v55.RUN['a']}
              B: {v55.RUN['b']}

            Evidence is written only to:
              {OUT}
            """
        ).strip()
    )


def parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(description=__doc__)
    sub = p.add_subparsers(dest="cmd", required=True)
    for name, fn in (
        ("plan", cmd_plan),
        ("preflight", cmd_preflight),
        ("target-outage", cmd_target_outage),
        ("source-outage", cmd_source_outage),
    ):
        cmd = sub.add_parser(name)
        cmd.set_defaults(fn=fn)
    return p


def main() -> None:
    args = parser().parse_args()
    args.fn(args)


if __name__ == "__main__":
    main()
