#!/usr/bin/env python3
"""Host-side orchestration for the isolated DFS asynchronous repair probe.

This script is intentionally a coordinator: it runs from macOS, but every
product process, filesystem operation, checksum proof, and REST probe runs
inside the Linux Lima VMs.  It does not touch the existing v51 runtime paths.

Default candidate artifacts are read from the build VM:
  /home/lzc.guest/afs-build/artifacts/v56-qualified

The isolated runtime paths remain repair-v55-a / repair-v55-b because the
scenario name was allocated before the source gate moved to v56.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import pathlib
import re
import shlex
import subprocess
import tempfile
import textwrap
import time
from typing import Any


ROOT = pathlib.Path(__file__).resolve().parents[2]
SOURCE = ROOT / "source"
OUT = ROOT / "evidence/afs-delivery/repair-runtime-v55"
CTRL = SOURCE / "scripts/deploy/afs-processctl"

VM = {"a": "afs-accept-a", "b": "afs-accept-b", "build": "afs-build"}
IP = {"a": "192.168.109.12", "b": "192.168.109.13"}
RUN = {
    "a": "/mnt/lima-afsadata/afs-delivery/repair-v55-a",
    "b": "/mnt/lima-afsbdata/afs-delivery/repair-v55-b",
}
TEMPLATE = {
    "a": "/mnt/lima-afsadata/afs-delivery/p2-memory-lane-v51",
    "b": "/mnt/lima-afsbdata/afs-delivery/p2-memory-peer-v51",
}
PORTS = {"meta_grpc": 18380, "meta_rest": 18381, "a_grpc": 18382, "a_rest": 18383, "b_grpc": 18384, "b_rest": 18385}
TEST_FILE = "repair-v55-deterministic-1m.bin"
TEST_SIZE = 1024 * 1024


def call(argv: list[str], timeout: int = 120) -> str:
    proc = subprocess.run(argv, text=True, capture_output=True, timeout=timeout)
    if OUT.exists():
        entry = {
            "time_unix_ms": int(time.time() * 1000),
            "argv": argv,
            "returncode": proc.returncode,
            "stdout": proc.stdout,
            "stderr": proc.stderr,
        }
        with (OUT / "commands.jsonl").open("a") as fh:
            fh.write(json.dumps(entry, sort_keys=True) + "\n")
        if proc.returncode:
            failure_name = f"command-failure-{int(time.time() * 1000)}.json"
            (OUT / failure_name).write_text(json.dumps(entry, indent=2, sort_keys=True) + "\n")
    if proc.returncode:
        raise RuntimeError(
            json.dumps(
                {
                    "argv": argv,
                    "returncode": proc.returncode,
                    "stdout": proc.stdout[-4000:],
                    "stderr": proc.stderr[-4000:],
                },
                indent=2,
            )
        )
    return proc.stdout


def guest(which: str, code: str, timeout: int = 120) -> str:
    return call(["limactl", "shell", VM[which], "--", "sudo", "python3", "-c", code], timeout=timeout)


def ctl(which: str, *args: str, timeout: int = 60) -> str:
    run = RUN[which]
    return call(
        [
            "limactl",
            "shell",
            VM[which],
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


def dump(name: str, obj: Any) -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    (OUT / name).write_text(json.dumps(obj, indent=2, sort_keys=True) + "\n")


def write_text(name: str, text: str) -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    (OUT / name).write_text(text)


def unique_name(name: str) -> str:
    path = OUT / name
    if not path.exists():
        return name
    stem = path.name[: -len(path.suffix)] if path.suffix else path.name
    suffix = path.suffix
    for index in range(2, 1000):
        candidate = path.with_name(f"{stem}.{index}{suffix}")
        if not candidate.exists():
            return candidate.name
    raise RuntimeError(f"too many evidence files for {name}")


def dump_unique(name: str, obj: Any) -> str:
    selected = unique_name(name)
    dump(selected, obj)
    return selected


def sha256_file(path: pathlib.Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as fh:
        for chunk in iter(lambda: fh.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def load_gate_report(path: pathlib.Path | None, candidate: str) -> dict[str, Any]:
    remote_default = f"/home/lzc.guest/afs-build/probes/repair-{candidate}/qualified-linux-clean/report.json"
    if path is None:
        path = ROOT / f"evidence/afs-delivery/{candidate}-qualified/report.json"
    if path.exists():
        return json.loads(path.read_text())
    remote = str(path) if str(path).startswith("/home/") else remote_default
    try:
        raw = call(["limactl", "shell", VM["build"], "--", "cat", remote], timeout=60)
    except RuntimeError as exc:
        raise SystemExit(
            "gate report required; pass --gate-report, create "
            f"{ROOT}/evidence/afs-delivery/{candidate}-qualified/report.json, "
            f"or provide build VM report {remote_default}"
        ) from exc
    return json.loads(raw)


def gate_passed(report: dict[str, Any]) -> bool:
    if report.get("status") == "PASS":
        return True
    if report.get("result") == "PASS":
        return True
    gates = report.get("gates")
    if isinstance(gates, list) and gates and all(isinstance(gate, dict) and gate.get("status") == "PASS" for gate in gates):
        return True
    return False


def collect_hashes(obj: Any, role: str, context: tuple[str, ...] = ()) -> list[str]:
    hits: list[str] = []
    if isinstance(obj, str):
        joined = " ".join(context).lower()
        if role in joined and re.fullmatch(r"[0-9a-f]{64}", obj):
            hits.append(obj)
        return hits
    if isinstance(obj, dict):
        for key, value in obj.items():
            hits.extend(collect_hashes(value, role, (*context, str(key))))
    elif isinstance(obj, list):
        for index, value in enumerate(obj):
            hits.extend(collect_hashes(value, role, (*context, str(index))))
    return hits


def expected_hashes(report: dict[str, Any]) -> dict[str, str]:
    result: dict[str, str] = {}
    for key in ("node", "meta"):
        values = sorted(set(collect_hashes(report, key)))
        if len(values) != 1:
            raise SystemExit(f"gate report does not expose a usable {key} sha256")
        result[key] = values[0]
    return result


def artifact_dir(candidate: str) -> str:
    return f"/home/lzc.guest/afs-build/artifacts/{candidate}-qualified"


def strip_artifacts_on_build_vm(candidate: str) -> str:
    src = artifact_dir(candidate)
    dst = f"/home/lzc.guest/afs-build/artifacts/{candidate}-qualified-stripped"
    script = f"""
set -euo pipefail
src={shlex.quote(src)}
dst={shlex.quote(dst)}
mkdir -p "$dst/debug"
for bin in afs-meta afs-node; do
  test -x "$src/$bin"
  rm -f "$dst/$bin" "$dst/debug/$bin.debug"
  cp "$src/$bin" "$dst/$bin"
  if command -v objcopy >/dev/null 2>&1; then
    objcopy --only-keep-debug "$dst/$bin" "$dst/debug/$bin.debug" || true
  fi
  strip --strip-debug "$dst/$bin"
  if command -v objcopy >/dev/null 2>&1 && [ -s "$dst/debug/$bin.debug" ]; then
    objcopy --add-gnu-debuglink="$dst/debug/$bin.debug" "$dst/$bin" || true
  fi
  chmod +x "$dst/$bin"
done
sha256sum "$src/afs-meta" "$src/afs-node" "$dst/afs-meta" "$dst/afs-node"
"""
    out = call(["limactl", "shell", VM["build"], "--", "bash", "-lc", script], timeout=240)
    write_text("build-artifact-sha256.txt", out)
    return dst


def copy_artifacts(candidate: str, hashes: dict[str, str]) -> None:
    stripped = strip_artifacts_on_build_vm(candidate)
    with tempfile.TemporaryDirectory(prefix=f"afs-{candidate}-") as tmp:
        tmpdir = pathlib.Path(tmp)
        originals: dict[str, pathlib.Path] = {}
        stripped_paths: dict[str, pathlib.Path] = {}
        for role in ("meta", "node"):
            original = tmpdir / f"afs-{role}.original"
            stripped_local = tmpdir / f"afs-{role}"
            call(["limactl", "copy", f"{VM['build']}:{artifact_dir(candidate)}/afs-{role}", str(original)], timeout=180)
            call(["limactl", "copy", f"{VM['build']}:{stripped}/afs-{role}", str(stripped_local)], timeout=180)
            if sha256_file(original) != hashes[role]:
                raise RuntimeError(f"{role} original sha does not match gate report")
            originals[role] = original
            stripped_paths[role] = stripped_local
        for role in ("meta", "node"):
            call(["limactl", "copy", str(stripped_paths[role]), f"{VM['a']}:{RUN['a']}/prefix/bin/afs-{role}"], timeout=180)
        call(["limactl", "copy", str(stripped_paths["node"]), f"{VM['b']}:{RUN['b']}/prefix/bin/afs-node"], timeout=180)
    staged: dict[str, Any] = {}
    for which, roles in {"a": ("meta", "node"), "b": ("node",)}.items():
        staged[which] = json.loads(
            guest(
                which,
                f"""
import json, pathlib, os, hashlib
r=pathlib.Path({RUN[which]!r})
result={{}}
for role in {roles!r}:
    p=r/'prefix/bin'/('afs-'+role)
    os.chmod(p,0o755)
    result[role]=hashlib.sha256(p.read_bytes()).hexdigest()
print(json.dumps(result))
""",
            )
        )
    dump("staged-runtime-binary-sha256.json", staged)


def set_toml_scalar(text: str, key: str, literal: str) -> str:
    line = f"{key} = {literal}"
    pattern = re.compile(rf"^{re.escape(key)}\s*=.*$", re.MULTILINE)
    if pattern.search(text):
        return pattern.sub(line, text)
    return text.rstrip() + "\n" + line + "\n"


def render_config(which: str, role: str, template_text: str) -> str:
    text = template_text.replace(TEMPLATE[which], RUN[which])
    replacements = {
        "18280": str(PORTS["meta_grpc"]),
        "18281": str(PORTS["meta_rest"]),
        "18282": str(PORTS["a_grpc"]),
        "18283": str(PORTS["a_rest"]),
        "18284": str(PORTS["b_grpc"]),
        "18285": str(PORTS["b_rest"]),
    }
    for old, new in replacements.items():
        text = text.replace(old, new)
    if role == "meta":
        text = set_toml_scalar(text, "id", "'repair-meta-a'")
    elif which == "a":
        text = set_toml_scalar(text, "id", "'repair-node-a'")
        text = set_toml_scalar(text, "advertise_endpoint", f"'https://{IP['a']}:{PORTS['a_grpc']}'")
        text = set_toml_scalar(text, "rest_listen", f"'0.0.0.0:{PORTS['a_rest']}'")
    else:
        text = set_toml_scalar(text, "id", "'repair-node-b'")
        text = set_toml_scalar(text, "advertise_endpoint", f"'https://{IP['b']}:{PORTS['b_grpc']}'")
        text = set_toml_scalar(text, "rest_listen", f"'0.0.0.0:{PORTS['b_rest']}'")
    if role == "node":
        text = set_toml_scalar(text, "meta_endpoint", f"'https://{IP['a']}:{PORTS['meta_grpc']}'")
    text = re.sub(r"(?<=[{,]\s)memory-node-a\s*=", "repair-node-a =", text)
    text = re.sub(r"(?<=[{,]\s)memory-node-b\s*=", "repair-node-b =", text)
    for key, value in (
        ("dfs_desired_copies", "2"),
        ("dfs_sync_required_copies", "1"),
        ("dfs_min_distinct_nodes", "1"),
        ("dfs_min_distinct_failure_domains", "1"),
    ):
        text = set_toml_scalar(text, key, value)
    return text


PREFLIGHT_GUEST = r"""
import json, os, pathlib, subprocess
run=pathlib.Path(__RUN__)
template=pathlib.Path(__TEMPLATE__)
ports=__PORTS__
roles=__ROLES__
assert template.is_dir(), f"template runtime missing: {template}"
assert not run.exists(), f"isolated runtime already exists: {run}"
volume=json.loads(subprocess.check_output(['findmnt','-J','-T',str(run.parent),'-o','TARGET,SOURCE,FSTYPE'],text=True))['filesystems'][0]
assert volume['fstype'] == 'ext4', volume
listening=subprocess.check_output(['ss','-ltnH'],text=True).splitlines()
for row in listening:
    addr=row.split()[3]
    for port in ports:
        assert not addr.endswith(':'+str(port)), f"port occupied: {row}"
parent=run.parent
stat=os.statvfs(str(parent))
available=stat.f_bavail*stat.f_frsize
assert available > 256*1024*1024, available
print(json.dumps({'runtime':str(run),'template':str(template),'ports_clear':ports,'available_bytes':available,'roles':roles,'data_volume':volume}))
"""


CREATE_RUNTIME_GUEST = r"""
import json, os, pathlib
run=pathlib.Path(__RUN__)
roles=__ROLES__
assert not run.exists(), f"isolated runtime already exists: {run}"
for rel in ('prefix/bin','etc','run','log','state/node','state/meta','mount-ownerfs','mount-dfs'):
    (run/rel).mkdir(parents=True, exist_ok=True)
uid=int(os.environ.get('SUDO_UID','0')); gid=int(os.environ.get('SUDO_GID','0'))
for p in [run]+[x for x in run.rglob('*') if x.is_dir()]:
    os.chown(p, uid, gid)
print(json.dumps({'runtime':str(run),'created':True,'roles':roles}))
"""


IDENTITY_GUEST = r"""
import hashlib, json, os, pathlib, platform, subprocess
run=pathlib.Path(__RUN__)
roles=__ROLES__
expected_hash=__EXPECTED_HASH__
result={'runtime':str(run),'platform':platform.platform(),'boot_id':pathlib.Path('/proc/sys/kernel/random/boot_id').read_text().strip(),'processes':{},'mounts':{}}
for role in roles:
    pid_path=run/'run'/(role+'.pid')
    pid=int(pid_path.read_text())
    proc=pathlib.Path('/proc')/str(pid)
    stat=(proc/'stat').read_text()
    fields=stat.split(') ')[1].split()
    exe=os.readlink(proc/'exe')
    sha=hashlib.sha256((proc/'exe').read_bytes()).hexdigest()
    expected_exe=str(run/'prefix/bin'/('afs-'+role))
    assert exe == expected_exe, {'role':role,'exe':exe,'expected':expected_exe}
    if expected_hash:
        assert sha == expected_hash[role], {'role':role,'sha256':sha,'expected':expected_hash[role]}
    result['processes'][role]={'pid':pid,'start_ticks':fields[19],'state':fields[0],'exe':exe,'sha256':sha}
    cfg=run/'etc'/(role+'.toml')
    result.setdefault('config_sha256',{})[role]=hashlib.sha256(cfg.read_bytes()).hexdigest()
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


OLD_IDENTITY_GUEST = r"""
import hashlib, json, os, pathlib
run=pathlib.Path(__RUN__)
roles=__ROLES__
result={'runtime':str(run),'processes':{},'config_sha256':{}}
for role in roles:
    cfg=run/'etc'/(role+'.toml')
    result['config_sha256'][role]=hashlib.sha256(cfg.read_bytes()).hexdigest()
    pid_path=run/'run'/(role+'.pid')
    pid=int(pid_path.read_text())
    proc=pathlib.Path('/proc')/str(pid)
    stat=(proc/'stat').read_text()
    fields=stat.split(') ')[1].split()
    exe=os.readlink(proc/'exe')
    sha=hashlib.sha256((proc/'exe').read_bytes()).hexdigest()
    expected=str(run/'prefix/bin'/('afs-'+role))
    assert exe == expected, {'role':role,'exe':exe,'expected':expected}
    result['processes'][role]={'pid':pid,'start_ticks':fields[19],'exe':exe,'sha256':sha}
print(json.dumps(result))
"""


def runtime_hashes(which: str, roles: tuple[str, ...]) -> dict[str, str]:
    staged = json.loads((OUT / "staged-runtime-binary-sha256.json").read_text())
    return {role: staged[which][role] for role in roles}


def capture_identity(which: str, roles: tuple[str, ...], name: str) -> dict[str, Any]:
    data = json.loads(
        guest(
            which,
            IDENTITY_GUEST.replace("__RUN__", repr(RUN[which]))
            .replace("__ROLES__", repr(list(roles)))
            .replace("__EXPECTED_HASH__", repr(runtime_hashes(which, roles))),
        )
    )
    dump(name, data)
    return data


def old_identity(which: str, roles: tuple[str, ...], name: str) -> dict[str, Any]:
    data = json.loads(
        guest(
            which,
            OLD_IDENTITY_GUEST.replace("__RUN__", repr(TEMPLATE[which])).replace("__ROLES__", repr(list(roles))),
        )
    )
    dump(name, data)
    return data


def assert_old_preserved(phase: str) -> None:
    before: dict[str, Any] = {}
    after: dict[str, Any] = {}
    for which, roles in {"a": ("meta", "node"), "b": ("node",)}.items():
        before_path = OUT / f"old-v51-{which}-before.json"
        if not before_path.exists():
            continue
        before[which] = json.loads(before_path.read_text())
        after[which] = old_identity(which, roles, unique_name(f"old-v51-{which}-{phase}.json"))
    if before and before != after:
        dump_unique(f"old-v51-identity-mismatch-{phase}.json", {"before": before, "after": after})
        raise SystemExit(f"old v51 process or config identity changed at {phase}")


def prepare(args: argparse.Namespace) -> None:
    if OUT.exists():
        raise SystemExit(f"evidence directory already exists: {OUT}")
    OUT.mkdir(parents=True)
    report = load_gate_report(args.gate_report, args.candidate)
    if not gate_passed(report):
        dump("gate-report.rejected.json", report)
        raise SystemExit("gate report is not PASS")
    hashes = expected_hashes(report)
    dump("gate-report.selected.json", {"candidate": args.candidate, "hashes": hashes, "report_path": str(args.gate_report or "")})
    old_before = {
        which: old_identity(which, roles, f"old-v51-{which}-before.json")
        for which, roles in {"a": ("meta", "node"), "b": ("node",)}.items()
    }
    for which, roles in {"a": ("meta", "node"), "b": ("node",)}.items():
        preflight = PREFLIGHT_GUEST.replace("__RUN__", repr(RUN[which])).replace("__TEMPLATE__", repr(TEMPLATE[which]))
        preflight = preflight.replace("__PORTS__", repr(list(PORTS.values()) if which == "a" else [PORTS["b_grpc"], PORTS["b_rest"]]))
        preflight = preflight.replace("__ROLES__", repr(list(roles)))
        dump(f"preflight-{which}.json", json.loads(guest(which, preflight)))
    for which, roles in {"a": ("meta", "node"), "b": ("node",)}.items():
        create_runtime = CREATE_RUNTIME_GUEST.replace("__RUN__", repr(RUN[which])).replace("__ROLES__", repr(list(roles)))
        dump(f"create-runtime-{which}.json", json.loads(guest(which, create_runtime)))
        call(["limactl", "copy", str(CTRL), f"{VM[which]}:{RUN[which]}/prefix/bin/afs-processctl"], timeout=120)
        guest(which, f"import os; os.chmod({(RUN[which] + '/prefix/bin/afs-processctl')!r},0o755)")
        for role in roles:
            old_text = guest(which, f"import pathlib; print(pathlib.Path({(TEMPLATE[which] + '/etc/' + role + '.toml')!r}).read_text())")
            rendered = render_config(which, role, old_text)
            with tempfile.NamedTemporaryFile("w", delete=False, prefix=f"afs-{which}-{role}-", suffix=".toml") as tmp:
                tmp.write(rendered)
                tmp_path = pathlib.Path(tmp.name)
            try:
                call(["limactl", "copy", str(tmp_path), f"{VM[which]}:{RUN[which]}/etc/{role}.toml"], timeout=120)
            finally:
                tmp_path.unlink(missing_ok=True)
    copy_artifacts(args.candidate, hashes)
    old_after = {
        which: old_identity(which, roles, f"old-v51-{which}-after.json")
        for which, roles in {"a": ("meta", "node"), "b": ("node",)}.items()
    }
    if old_before != old_after:
        dump("old-v51-identity-mismatch.json", {"before": old_before, "after": old_after})
        raise SystemExit("old v51 process or config identity changed during prepare; inspect before continuing")
    dump(
        "prepare.complete.json",
        {
            "status": "prepared",
            "candidate": args.candidate,
            "runtime": RUN,
            "ports": PORTS,
            "replication": {
                "dfs_desired_copies": 2,
                "dfs_sync_required_copies": 1,
                "dfs_min_distinct_nodes": 1,
                "dfs_min_distinct_failure_domains": 1,
            },
            "source_commit": call(["git", "-C", str(SOURCE), "rev-parse", "HEAD"]).strip(),
            "processctl_sha256": sha256_file(CTRL),
        },
    )


def start_a(_: argparse.Namespace) -> None:
    write_text("start-a.stdout", ctl("a", "start", "all", timeout=90))
    capture_identity("a", ("meta", "node"), "identity-a-after-start.json")


CREATE_AND_INSPECT = r"""
import hashlib, json, os, pathlib, re, subprocess, time, urllib.error, urllib.parse, urllib.request
run=pathlib.Path(__RUN__)
mount=run/'mount-dfs'
target=mount/__TEST_FILE__
data=bytes((i*131+17)%251 for i in range(__TEST_SIZE__))
created_this_attempt=not target.exists()
if created_this_attempt:
    with open(target,'xb') as fh:
        fh.write(data)
        fh.flush()
        os.fsync(fh.fileno())
else:
    assert target.read_bytes() == data, 'retained fixture content differs'
digest=hashlib.sha256(data).hexdigest()
chunk_root=run/'state/node/dfs/chunks'
if not chunk_root.is_dir():
    raise SystemExit(json.dumps({'error':'chunk catalog directory missing','path':str(chunk_root)}))
inventory=[]
for path in sorted(chunk_root.iterdir()):
    if not path.is_file():
        continue
    try:
        size=path.stat().st_size
    except OSError:
        continue
    if size == 0:
        continue
    rel=str(path.relative_to(run))
    sha=None
    if size <= 128*1024*1024:
        sha=hashlib.sha256(path.read_bytes()).hexdigest()
    inventory.append({'path':rel,'name':path.name,'size':size,'sha256':sha})
chunk_candidates=[]
for item in inventory:
    if item['size'] == len(data) and item['sha256'] == digest:
        chunk_candidates.append(item['name'])
if not chunk_candidates:
    raise SystemExit(json.dumps({'error':'no source chunk file exactly matched deterministic file bytes','digest':digest,'inventory':inventory}, indent=2))
responses=[]
for chunk_id in chunk_candidates:
    url='http://127.0.0.1:__META_REST__/v1/dfs/chunks/'+urllib.parse.quote(chunk_id, safe='')+'/replication'
    try:
        with urllib.request.urlopen(url, timeout=3) as resp:
            body=resp.read().decode()
            parsed=json.loads(body)
            responses.append({'chunk_id':chunk_id,'status':resp.status,'json':parsed})
    except Exception as exc:
        responses.append({'chunk_id':chunk_id,'error':repr(exc)})
def strings(obj):
    if isinstance(obj,str): return [obj]
    if isinstance(obj,dict):
        out=[]
        for k,v in obj.items():
            out.append(str(k)); out += strings(v)
        return out
    if isinstance(obj,list):
        out=[]
        for v in obj: out += strings(v)
        return out
    return [str(obj)]
accepted=[]
known_initial_states={'Pending','RetryWaiting','BlockedNoSource'}
for response in responses:
    payload=response.get('json',{})
    tasks=payload.get('tasks',[]) if isinstance(payload,dict) else []
    task_states={task.get('state') for task in tasks if isinstance(task,dict) and task.get('chunk_id') == response.get('chunk_id')}
    if response.get('status') == 200 and payload.get('health') == 'UnderReplicated' and payload.get('available_copies') == 1:
        if task_states & known_initial_states:
            accepted.append({**response,'task_states':sorted(task_states)})
if not accepted:
    raise SystemExit(json.dumps({'error':'no exact under-replicated chunk REST response found','digest':digest,'inventory':inventory,'responses':responses}, indent=2))
print(json.dumps({'file':str(target),'created_this_attempt':created_this_attempt,'size':len(data),'sha256':digest,'inventory':inventory,'accepted':accepted,'responses':responses}))
"""


def probe_a(_: argparse.Namespace) -> None:
    result = json.loads(
        guest(
            "a",
            CREATE_AND_INSPECT.replace("__RUN__", repr(RUN["a"]))
            .replace("__TEST_FILE__", repr(TEST_FILE))
            .replace("__TEST_SIZE__", str(TEST_SIZE))
            .replace("__META_REST__", str(PORTS["meta_rest"])),
            timeout=120,
        )
    )
    dump("probe-a-create-underreplicated.json", result)
    dump("repair-chunk-id.json", {"chunk_id": result["accepted"][0]["chunk_id"], "file_sha256": result["sha256"]})


def start_b(_: argparse.Namespace) -> None:
    write_text("start-b.stdout", ctl("b", "start", "node", timeout=90))
    capture_identity("b", ("node",), "identity-b-after-start.json")


WAIT_REPAIR = r"""
import json, pathlib, time, urllib.parse, urllib.request
chunk_id=__CHUNK_ID__
deadline=time.monotonic()+60
polls=[]
def satisfied(payload):
    if payload.get('health') != 'Satisfied' or payload.get('available_copies') != 2:
        return False
    tasks=payload.get('tasks',[])
    return any(isinstance(task,dict) and task.get('chunk_id') == chunk_id and task.get('state') == 'Completed' for task in tasks)
while True:
    url='http://127.0.0.1:__META_REST__/v1/dfs/chunks/'+urllib.parse.quote(chunk_id, safe='')+'/replication'
    try:
        with urllib.request.urlopen(url, timeout=3) as resp:
            payload=json.loads(resp.read().decode())
            polls.append({'time_unix_ms':int(time.time()*1000),'status':resp.status,'json':payload})
            if satisfied(payload):
                print(json.dumps({'status':'satisfied','chunk_id':chunk_id,'polls':polls}))
                raise SystemExit(0)
    except Exception as exc:
        polls.append({'time_unix_ms':int(time.time()*1000),'error':repr(exc)})
    if time.monotonic() >= deadline:
        raise SystemExit(json.dumps({'status':'timeout','chunk_id':chunk_id,'polls':polls}, indent=2))
    time.sleep(1)
"""


def wait_repair(_: argparse.Namespace) -> None:
    info = json.loads((OUT / "repair-chunk-id.json").read_text())
    result = json.loads(
        guest(
            "a",
            WAIT_REPAIR.replace("__CHUNK_ID__", repr(info["chunk_id"])).replace("__META_REST__", str(PORTS["meta_rest"])),
            timeout=75,
        )
    )
    dump("wait-repair-satisfied.json", result)


VERIFY_B = r"""
import hashlib, json, pathlib, os
run=pathlib.Path(__RUN__)
expected=__SHA__
target=run/'mount-dfs'/__TEST_FILE__
fuse_sha=hashlib.sha256(target.read_bytes()).hexdigest()
inventory=[]
matched=[]
chunk_root=run/'state/node/dfs/chunks'
if not chunk_root.is_dir():
    raise SystemExit(json.dumps({'error':'B chunk catalog directory missing','path':str(chunk_root)}))
for path in sorted(chunk_root.iterdir()):
    if not path.is_file():
        continue
    size=path.stat().st_size
    if size == 0 or size > 128*1024*1024:
        continue
    sha=hashlib.sha256(path.read_bytes()).hexdigest()
    item={'path':str(path.relative_to(run)),'size':size,'sha256':sha}
    inventory.append(item)
    if sha == expected:
        matched.append(item)
if fuse_sha != expected or not matched:
    raise SystemExit(json.dumps({'error':'B verification failed','expected':expected,'fuse_path':str(target),'fuse_sha256':fuse_sha,'matched':matched,'inventory':inventory}, indent=2))
print(json.dumps({'remote_fuse_path':str(target),'remote_fuse_sha256':fuse_sha,'matched_chunk_files':matched,'inventory':inventory}))
"""


def verify(_: argparse.Namespace) -> None:
    info = json.loads((OUT / "repair-chunk-id.json").read_text())
    result = json.loads(
        guest(
            "b",
            VERIFY_B.replace("__RUN__", repr(RUN["b"]))
            .replace("__SHA__", repr(info["file_sha256"]))
            .replace("__TEST_FILE__", repr(TEST_FILE)),
            timeout=120,
        )
    )
    dump_unique("verify-b-bytes-and-fuse.json", result)
    capture_identity("a", ("meta", "node"), unique_name("identity-a-after-verify.json"))
    capture_identity("b", ("node",), unique_name("identity-b-after-verify.json"))
    assert_old_preserved("final")


def restart_b(_: argparse.Namespace) -> None:
    before = capture_identity("b", ("node",), unique_name("identity-b-before-restart.json"))
    write_text("restart-b-stop.stdout", ctl("b", "stop", "node", timeout=90))
    time.sleep(1)
    write_text("restart-b-start.stdout", ctl("b", "start", "node", timeout=90))
    after = capture_identity("b", ("node",), unique_name("identity-b-after-restart.json"))
    b0 = before["processes"]["node"]
    b1 = after["processes"]["node"]
    if b0["pid"] == b1["pid"] or b0["start_ticks"] == b1["start_ticks"]:
        raise SystemExit("B restart did not produce a distinct process identity")
    if b0["sha256"] != b1["sha256"] or before["config_sha256"]["node"] != after["config_sha256"]["node"]:
        raise SystemExit("B restart changed binary or config identity")
    content: dict[str, Any] | None = None
    chunk_file = OUT / "repair-chunk-id.json"
    if chunk_file.exists():
        info = json.loads(chunk_file.read_text())
        content = json.loads(
            guest(
                "b",
                VERIFY_B.replace("__RUN__", repr(RUN["b"]))
                .replace("__SHA__", repr(info["file_sha256"]))
                .replace("__TEST_FILE__", repr(TEST_FILE)),
                timeout=120,
            )
        )
        dump_unique("restart-b-content-verify.json", content)
    dump_unique("restart-b.identity-change.json", {"before": before, "after": after, "content": content})


def plan(_: argparse.Namespace) -> None:
    print(
        textwrap.dedent(
            f"""
            Isolated DFS asynchronous repair probe plan:
              1. prepare       stage {RUN['a']} and {RUN['b']} from v51 TOML templates, candidate artifacts, ports 18380..18385
              2. start-a       start A meta+node only and capture /proc identity plus mount evidence
              3. probe-a       create/fsync deterministic 1 MiB DFS file; find chunk_id from local storage; require UnderReplicated pending/blocked REST state
              4. start-b       start B node only and capture identity
              5. wait-repair   poll Meta REST for <=60s until available_copies>=2, health Satisfied, task Completed
              6. verify        prove B ext4 bytes and B FUSE reread checksum match A file
              7. restart-b     optional controlled processctl stop/start of isolated B only

            Defaults:
              candidate: v56
              artifact dir on build VM: {artifact_dir('v56')}
              evidence: {OUT}
            """
        ).strip()
    )


def parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(description=__doc__)
    env_gate = os.environ.get("AFS_REPAIR_GATE_REPORT")
    p.add_argument("--candidate", default=os.environ.get("AFS_REPAIR_CANDIDATE", "v56"))
    p.add_argument("--gate-report", type=pathlib.Path, default=pathlib.Path(env_gate) if env_gate else None)
    sub = p.add_subparsers(dest="cmd", required=True)
    for name, fn in (
        ("plan", plan),
        ("prepare", prepare),
        ("start-a", start_a),
        ("probe-a", probe_a),
        ("start-b", start_b),
        ("wait-repair", wait_repair),
        ("verify", verify),
        ("restart-b", restart_b),
    ):
        cmd = sub.add_parser(name)
        cmd.set_defaults(fn=fn)
    return p


def main() -> None:
    args = parser().parse_args()
    args.fn(args)


if __name__ == "__main__":
    main()
