#!/usr/bin/env python3
"""Prepare or run an isolated processctl product shutdown probe on Linux A.

Host-side orchestrator. It stages v49-qualified binaries from afs-build onto
afs-accept-a, writes fresh double-quoted configs under the processctl-v50 ext4
runtime, and can later copy the selected afs-processctl and run the real
start/stop lifecycle proof.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import pathlib
import shutil
import subprocess
import sys
import tempfile
import textwrap
import time
from typing import Iterable


ROOT = pathlib.Path(__file__).resolve().parents[2]
DEFAULT_EVIDENCE = ROOT / "evidence/afs-delivery/p2-processctl-v50/runtime-product"
GUEST_RUN = pathlib.PurePosixPath("/mnt/lima-afsadata/afs-delivery/processctl-v50")
BUILD_ARTIFACTS = pathlib.PurePosixPath("/home/lzc.guest/afs-build/artifacts/v49-qualified")
TLS_DIR = pathlib.PurePosixPath("/mnt/lima-afsadata/afs-delivery/p1b/tls")
NODE_SHA = "dbbf2ccd5eb47f06178bfc3140599865c1b7ad5f28851e244e728fbcdfc136af"
META_SHA = "917432057950380da208b057a4d94156841e1eab03e533f56675433d5184fd9c"
PORTS = {
    "meta_grpc": 18080,
    "meta_rest": 18081,
    "node_grpc": 18082,
    "node_rest": 18083,
}


def run(cmd: list[str], *, timeout: int = 60, check: bool = True, text: bool = True) -> subprocess.CompletedProcess[str]:
    proc = subprocess.run(cmd, capture_output=True, text=text, timeout=timeout)
    if check and proc.returncode != 0:
        raise RuntimeError(
            f"command failed rc={proc.returncode}: {' '.join(cmd)}\nSTDOUT:\n{proc.stdout}\nSTDERR:\n{proc.stderr}"
        )
    return proc


def lima(vm: str, script: str, *, timeout: int = 60, check: bool = True) -> subprocess.CompletedProcess[str]:
    return run(["limactl", "shell", vm, "--", "bash", "-lc", script], timeout=timeout, check=check)


def lima_copy(src: str, dst: str, *, timeout: int = 120) -> None:
    run(["limactl", "copy", src, dst], timeout=timeout)


def sha256_file(path: pathlib.Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for block in iter(lambda: f.read(1024 * 1024), b""):
            h.update(block)
    return h.hexdigest()


def write_text(path: pathlib.Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text)


def host_manifest(evidence: pathlib.Path, phase: str, controller: pathlib.Path | None = None) -> dict:
    manifest = {
        "phase": phase,
        "host_time_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "root": str(ROOT),
        "guest_run": str(GUEST_RUN),
        "build_artifacts": str(BUILD_ARTIFACTS),
        "ports": PORTS,
        "expected_sha256": {"afs-node": NODE_SHA, "afs-meta": META_SHA},
        "controller": None,
    }
    if controller is not None:
        manifest["controller"] = {
            "path": str(controller),
            "sha256": sha256_file(controller),
        }
    write_text(evidence / "manifest.json", json.dumps(manifest, indent=2) + "\n")
    return manifest


def pull_guest_files(evidence: pathlib.Path, paths: Iterable[str]) -> None:
    pulled = evidence / "guest"
    pulled.mkdir(parents=True, exist_ok=True)
    for rel in paths:
        target = pulled / rel.replace("/", "__")
        proc = run(["limactl", "copy", f"afs-accept-a:{GUEST_RUN}/{rel}", str(target)], check=False, timeout=120)
        if proc.returncode != 0:
            write_text(target.with_suffix(target.suffix + ".missing.txt"), proc.stderr + proc.stdout)


def prepare(evidence: pathlib.Path) -> None:
    evidence.mkdir(parents=True, exist_ok=True)
    host_manifest(evidence, "prepared")
    with tempfile.TemporaryDirectory(prefix="afs-v49-") as td:
        tmp = pathlib.Path(td)
        for name, expected in (("afs-node", NODE_SHA), ("afs-meta", META_SHA)):
            local = tmp / name
            lima_copy(f"afs-build:{BUILD_ARTIFACTS}/{name}", str(local))
            actual = sha256_file(local)
            if actual != expected:
                raise AssertionError(f"{name} sha mismatch: {actual} != {expected}")
        prep = f"""
set -euo pipefail
RUN="{GUEST_RUN}"
TLS="{TLS_DIR}"
for port in {PORTS["meta_grpc"]} {PORTS["meta_rest"]} {PORTS["node_grpc"]} {PORTS["node_rest"]}; do
  if ss -ltnH | awk '{{print $4}}' | grep -Eq ":${{port}}$"; then
    echo "port in use: $port" >&2
    exit 20
  fi
done
if [ ! -d "$TLS" ]; then echo "missing TLS dir $TLS" >&2; exit 21; fi
findmnt -rn -T /mnt/lima-afsadata -o FSTYPE,SOURCE,TARGET | tee /tmp/processctl-v50-findmnt.txt
if [ "$(findmnt -rn -T /mnt/lima-afsadata -o FSTYPE)" != ext4 ]; then
  echo "delivery volume is not ext4" >&2
  exit 22
fi
if [ -e "$RUN" ]; then
  echo "refusing to replace existing runtime: $RUN" >&2
  exit 26
fi
for mount in "$RUN/mount-dfs" "$RUN/mount-ownerfs"; do
  if findmnt -rn --mountpoint "$mount" >/dev/null 2>&1; then
    echo "refusing to replace mounted path: $mount" >&2
    exit 23
  fi
done
sudo install -d -o "$(id -u)" -g "$(id -g)" "$RUN/prefix/bin" "$RUN/etc" "$RUN/run" "$RUN/log" "$RUN/state/meta" "$RUN/state/node" "$RUN/mount-dfs" "$RUN/mount-ownerfs"
sudo chown "$(id -u):$(id -g)" "$RUN"
cat > "$RUN/etc/meta.toml" <<'EOF_META'
id = "processctl-v50-meta-a"
fs = "all"
meta_store = "memory"
data_dir = "{GUEST_RUN}/state/meta"
uds_path = "{GUEST_RUN}/run/meta.sock"
grpc_listen = "0.0.0.0:{PORTS["meta_grpc"]}"
rest_listen = "0.0.0.0:{PORTS["meta_rest"]}"
tls_ca_certificate = "{TLS_DIR}/ca.pem"
tls_identity_certificate = "{TLS_DIR}/node-a.pem"
tls_identity_private_key = "{TLS_DIR}/node-a-key.pem"
tls_server_name = "afs-cluster"
trusted_node_certs = {{ processctl-v50-node-a = "{TLS_DIR}/node-a.pem" }}
log_level = "info"
trace_enabled = false
EOF_META
cat > "$RUN/etc/node.toml" <<'EOF_NODE'
id = "processctl-v50-node-a"
fs = "all"
meta_endpoint = "https://127.0.0.1:{PORTS["meta_grpc"]}"
advertise_endpoint = "https://127.0.0.1:{PORTS["node_grpc"]}"
grpc_listen = "0.0.0.0:{PORTS["node_grpc"]}"
rest_listen = "0.0.0.0:{PORTS["node_rest"]}"
data_dir = "{GUEST_RUN}/state/node"
uds_path = "{GUEST_RUN}/run/node.sock"
ownerfs_mount = "{GUEST_RUN}/mount-ownerfs"
dfs_mount = "{GUEST_RUN}/mount-dfs"
data_mode = "grpc"
tls_ca_certificate = "{TLS_DIR}/ca.pem"
tls_identity_certificate = "{TLS_DIR}/node-a.pem"
tls_identity_private_key = "{TLS_DIR}/node-a-key.pem"
tls_server_name = "afs-cluster"
trusted_node_certs = {{ processctl-v50-node-a = "{TLS_DIR}/node-a.pem" }}
log_level = "info"
trace_enabled = false
EOF_NODE
sha256sum "$RUN/etc/meta.toml" "$RUN/etc/node.toml" > "$RUN/config.sha256"
openssl x509 -in "$TLS/node-a.pem" -noout -subject -issuer -ext subjectAltName > "$RUN/tls-node-a-public.txt"
cp /tmp/processctl-v50-findmnt.txt "$RUN/findmnt.txt"
"""
        lima("afs-accept-a", prep, timeout=60)
        for name in ("afs-node", "afs-meta"):
            lima_copy(str(tmp / name), f"afs-accept-a:{GUEST_RUN}/prefix/bin/{name}")
        verify = f"""
set -euo pipefail
RUN="{GUEST_RUN}"
chmod +x "$RUN/prefix/bin/afs-node" "$RUN/prefix/bin/afs-meta"
sha256sum "$RUN/prefix/bin/afs-node" "$RUN/prefix/bin/afs-meta" | tee "$RUN/staged-binaries.sha256"
grep -R "'.*'" "$RUN/etc" && {{ echo "single quotes found in configs" >&2; exit 24; }} || true
ss -ltnH | awk '{{print $4}}' | grep -E ":({PORTS["meta_grpc"]}|{PORTS["meta_rest"]}|{PORTS["node_grpc"]}|{PORTS["node_rest"]})$" && exit 25 || true
"""
        lima("afs-accept-a", verify, timeout=60)
    pull_guest_files(
        evidence,
        [
            "findmnt.txt",
            "staged-binaries.sha256",
            "config.sha256",
            "tls-node-a-public.txt",
            "etc/meta.toml",
            "etc/node.toml",
        ],
    )
    write_text(
        evidence / "README.md",
        textwrap.dedent(
            f"""\
            # processctl v50 runtime product probe

            Phase: prepared.

            Prepared fresh ext4 runtime `{GUEST_RUN}` on `afs-accept-a` with
            v49-qualified binaries copied from `afs-build:{BUILD_ARTIFACTS}`.
            Ports `{PORTS["meta_grpc"]}..{PORTS["node_rest"]}` were verified free before staging.

            The controller was intentionally not copied and no product process was
            started in this phase. Waiting for root to announce the stable
            `afs-processctl` hash before `run`.

            No TLS private key contents were copied into evidence; only public
            certificate metadata was recorded.
            """
        ),
    )


GUEST_RUNNER = r'''
#!/usr/bin/env python3
from __future__ import annotations
import errno, hashlib, json, os, pathlib, platform, shutil, signal, subprocess, time, urllib.request

RUN = pathlib.Path("__GUEST_RUN__")
PORTS = {"meta_rest": __META_REST__, "node_rest": __NODE_REST__}
NODE_SHA = "__NODE_SHA__"
META_SHA = "__META_SHA__"
CTL_SHA = "__CTL_SHA__"
CTL = RUN / "prefix/bin/afs-processctl"
RUN_ID = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime()) + f"-pid{os.getpid()}"

def sh(cmd, timeout=60, check=True):
    p = subprocess.run(cmd, shell=True, text=True, capture_output=True, timeout=timeout)
    if check and p.returncode != 0:
        raise AssertionError({"cmd": cmd, "rc": p.returncode, "stdout": p.stdout, "stderr": p.stderr})
    return p

def sha(path):
    return hashlib.sha256(pathlib.Path(path).read_bytes()).hexdigest()

def proc_identity(service, expected_sha):
    pid = int((RUN / "run" / f"{service}.pid").read_text().strip())
    proc = pathlib.Path("/proc") / str(pid)
    assert proc.exists(), pid
    exe = os.readlink(proc / "exe")
    actual_sha = hashlib.sha256((proc / "exe").read_bytes()).hexdigest()
    assert actual_sha == expected_sha, (service, actual_sha, expected_sha)
    stat = (proc / "stat").read_text().split(") ")[1].split()
    return {"pid": pid, "exe": exe, "sha256": actual_sha, "start_ticks": stat[19], "state": stat[0]}

def ctl(*args, timeout=60, check=False):
    cmd = [
        "sudo", str(CTL),
        "--prefix", str(RUN / "prefix"),
        "--config-dir", str(RUN / "etc"),
        "--run-dir", str(RUN / "run"),
        "--log-dir", str(RUN / "log"),
        "--timeout", "18",
        *args,
    ]
    started = time.monotonic()
    p = subprocess.run(cmd, text=True, capture_output=True, timeout=timeout)
    elapsed = time.monotonic() - started
    if check and p.returncode != 0:
        raise AssertionError({"cmd": cmd, "rc": p.returncode, "stdout": p.stdout, "stderr": p.stderr})
    return {"returncode": p.returncode, "stdout": p.stdout, "stderr": p.stderr, "elapsed_seconds": elapsed}

def ready(port):
    deadline = time.monotonic() + 20
    while time.monotonic() < deadline:
        try:
            body = urllib.request.urlopen(f"http://127.0.0.1:{port}/health", timeout=.5).read()
            if b'"status":"ready"' in body:
                return body.decode()
        except Exception:
            pass
        time.sleep(.05)
    raise AssertionError(f"not ready: {port}")

def close_fd(fd):
    try:
        os.close(fd)
        return {"errno": None}
    except OSError as e:
        return {"errno": e.errno, "strerror": e.strerror}

def status_json(service):
    out = ctl("--json", "status", service, timeout=20, check=True)["stdout"].strip().splitlines()
    assert len(out) == 1, out
    return json.loads(out[0])

def receipt(service):
    ident = {}
    for line in (RUN / "run" / f"{service}.identity").read_text().splitlines():
        if "=" in line:
            k, v = line.split("=", 1)
            ident[k] = v
    lifecycle = pathlib.Path(ident["lifecycle"])
    rec = {}
    for line in (lifecycle / "exit").read_text().splitlines():
        if "=" in line:
            k, v = line.split("=", 1)
            rec[k] = v
    return {"identity": ident, "receipt": rec}

def snapshot_lifecycle(label, service):
    root = RUN / "probe-artifacts" / RUN_ID / label
    root.mkdir(parents=True, exist_ok=True)
    ident_path = RUN / "run" / f"{service}.identity"
    shutil.copy2(ident_path, root / f"{service}.identity")
    ident = {}
    for line in ident_path.read_text().splitlines():
        if "=" in line:
            k, v = line.split("=", 1)
            ident[k] = v
    lifecycle = pathlib.Path(ident["lifecycle"])
    copied = {"identity": str(root / f"{service}.identity"), "lifecycle": str(lifecycle), "files": []}
    for name in ("child", "exit", "ready", "go"):
        src = lifecycle / name
        if src.exists():
            dst = root / name
            shutil.copy2(src, dst)
            copied["files"].append(str(dst))
    return copied

def exact_mounts():
    found = {}
    for kind in ("dfs", "ownerfs"):
        p = sh(f"findmnt -J -M {RUN / ('mount-' + kind)} -o TARGET,FSTYPE,SOURCE", timeout=10)
        fs = json.loads(p.stdout)["filesystems"][0]
        assert fs["target"] == str(RUN / ("mount-" + kind)), fs
        assert fs["source"] == f"afs-{kind}", fs
        assert fs["fstype"].startswith("fuse"), fs
        found[kind] = fs
    return found

def stop_all_cleanup(events):
    for service in ("node", "meta"):
        try:
            events.append({f"cleanup_stop_{service}": ctl("stop", service, timeout=25)})
        except Exception as e:
            events.append({f"cleanup_stop_{service}_error": repr(e)})
    try:
        for mount in (RUN / "mount-dfs", RUN / "mount-ownerfs"):
            sh(f"if findmnt -rn --mountpoint {mount} >/dev/null 2>&1; then sudo fusermount3 -u {mount} || sudo umount {mount}; fi", timeout=10, check=False)
    except Exception as e:
        events.append({"cleanup_unmount_error": repr(e)})

events = []
fds = []
meta_paused = False
try:
    assert platform.system() == "Linux"
    assert sha(CTL) == CTL_SHA, sha(CTL)
    assert sha(RUN / "prefix/bin/afs-node") == NODE_SHA
    assert sha(RUN / "prefix/bin/afs-meta") == META_SHA
    env = {
        "platform": platform.platform(),
        "run_id": RUN_ID,
        "boot_id": pathlib.Path("/proc/sys/kernel/random/boot_id").read_text().strip(),
        "filesystem": sh(f"findmnt -J -T {RUN} -o FSTYPE,SOURCE,TARGET", timeout=10).stdout,
        "controller_sha256": CTL_SHA,
        "binary_sha256": {"afs-node": NODE_SHA, "afs-meta": META_SHA},
        "config_sha256": {
            "node": sha(RUN / "etc/node.toml"),
            "meta": sha(RUN / "etc/meta.toml"),
        },
    }
    events.append({"environment": env})
    start = ctl("start", "all", timeout=60)
    events.append({"start_all": start})
    assert start["returncode"] == 0, start
    ready(PORTS["meta_rest"]); ready(PORTS["node_rest"])
    events.append({"identity_initial": {"meta": proc_identity("meta", META_SHA), "node": proc_identity("node", NODE_SHA), "mounts": exact_mounts()}})

    normal_path = RUN / f"mount-dfs/normal-drain-{RUN_ID}.bin"
    fd = os.open(normal_path, os.O_CREAT | os.O_EXCL | os.O_RDWR, 0o600); fds.append(fd)
    normal_write_started = time.monotonic()
    normal_write_count = os.write(fd, b"accepted-dirty")
    normal_write_elapsed = time.monotonic() - normal_write_started
    assert normal_write_count == len(b"accepted-dirty")
    events.append({"normal_dirty_write": {"path": str(normal_path), "bytes": normal_write_count, "elapsed_seconds": normal_write_elapsed}})
    normal_stop = ctl("stop", "node", timeout=25)
    normal_stop["old_fd_close"] = close_fd(fd); fds.remove(fd)
    normal_stop["status"] = status_json("node")
    normal_stop["receipt"] = receipt("node")
    normal_stop["raw_lifecycle_snapshot"] = snapshot_lifecycle("normal-stop-node", "node")
    events.append({"normal_stop": normal_stop})
    assert normal_stop["returncode"] == 0, normal_stop
    assert normal_stop["receipt"]["receipt"]["exit_code"] == "0", normal_stop
    assert normal_stop["status"]["state"] == "stopped" and normal_stop["status"]["exit_code"] == "0", normal_stop

    restart = ctl("start", "node", timeout=60)
    events.append({"normal_restart": restart})
    assert restart["returncode"] == 0, restart
    ready(PORTS["node_rest"]); exact_mounts()
    assert normal_path.read_bytes() == b"accepted-dirty"
    events.append({"normal_readback": normal_path.read_bytes().hex(), "identity_normal_restart": proc_identity("node", NODE_SHA)})

    forced_path = RUN / f"mount-dfs/forced-unknown-{RUN_ID}.bin"
    fd = os.open(forced_path, os.O_CREAT | os.O_EXCL | os.O_RDWR, 0o600)
    assert os.write(fd, b"seed") == 4
    os.fsync(fd)
    os.close(fd)
    assert forced_path.read_bytes() == b"seed"
    fd = os.open(forced_path, os.O_RDWR); fds.append(fd)
    forced_log = RUN / "log/node.log"
    meta_before = proc_identity("meta", META_SHA)
    node_before_fault = proc_identity("node", NODE_SHA)
    log_offset_before_failed_stop = forced_log.stat().st_size if forced_log.exists() else 0
    os.kill(meta_before["pid"], signal.SIGSTOP)
    meta_paused = True
    deadline = time.monotonic() + 2
    state = None
    while time.monotonic() < deadline:
        state = pathlib.Path(f"/proc/{meta_before['pid']}/stat").read_text().split(") ")[1].split()[0]
        if state == "T":
            break
        time.sleep(.02)
    assert state == "T", state
    write_started = time.monotonic()
    dirty_write_count = os.write(fd, b"next")
    dirty_write_elapsed = time.monotonic() - write_started
    assert dirty_write_count == 4, dirty_write_count
    events.append({
        "fault": {
            "signal": "SIGSTOP",
            "meta": meta_before,
            "state": state,
            "node_before_fault": node_before_fault,
            "node_after_t_dirty_write": proc_identity("node", NODE_SHA),
            "dirty_write_after_t": {
                "path": str(forced_path),
                "bytes": dirty_write_count,
                "elapsed_seconds": dirty_write_elapsed,
            },
        }
    })
    failed_stop = ctl("stop", "node", timeout=25)
    failed_stop["old_fd_close"] = close_fd(fd); fds.remove(fd)
    failed_stop["status"] = status_json("node")
    failed_stop["receipt"] = receipt("node")
    failed_stop["raw_lifecycle_snapshot"] = snapshot_lifecycle("failed-stop-node", "node")
    failed_log_after = ""
    if forced_log.exists():
        failed_log_after = forced_log.read_text(errors="replace")[log_offset_before_failed_stop:]
    failed_stop["log_after_stop"] = failed_log_after
    events.append({"failed_stop": failed_stop})
    assert failed_stop["returncode"] in (1, 124), failed_stop
    assert failed_stop["receipt"]["receipt"]["exit_code"] == str(failed_stop["returncode"]), failed_stop
    assert failed_stop["status"]["state"] == "failed", failed_stop
    assert failed_stop["status"]["exit_code"] == str(failed_stop["returncode"]), failed_stop
    if failed_stop["returncode"] == 1:
        assert "dfs.node_drain_incomplete" in failed_log_after, failed_stop
        assert "node.shutdown_failed" in failed_log_after, failed_stop
    if failed_stop["returncode"] == 124:
        assert failed_stop["elapsed_seconds"] >= 14, failed_stop
        assert "process.shutdown_forced" in failed_log_after, failed_stop

    os.kill(meta_before["pid"], signal.SIGCONT)
    meta_paused = False
    ready(PORTS["meta_rest"])
    meta_after = proc_identity("meta", META_SHA)
    assert meta_after["pid"] == meta_before["pid"] and meta_after["start_ticks"] == meta_before["start_ticks"], (meta_before, meta_after)
    events.append({"meta_resumed": {"before": meta_before, "after": meta_after}})
    restart2 = ctl("start", "node", timeout=60)
    events.append({"forced_restart": restart2})
    assert restart2["returncode"] == 0, restart2
    ready(PORTS["node_rest"]); exact_mounts()
    fresh = forced_path.read_bytes()
    assert fresh == b"seed", fresh
    assert normal_path.read_bytes() == b"accepted-dirty"
    events.append({"fresh_readback": {"forced_path": fresh.hex(), "watermark": normal_path.read_bytes().hex()}, "identity_forced_restart": proc_identity("node", NODE_SHA)})

    final_node = ctl("stop", "node", timeout=25)
    final_node["status"] = status_json("node")
    final_node["receipt"] = receipt("node")
    final_node["raw_lifecycle_snapshot"] = snapshot_lifecycle("final-stop-node", "node")
    events.append({"final_node_stop": final_node})
    assert final_node["returncode"] == 0 and final_node["receipt"]["receipt"]["exit_code"] == "0", final_node
    final_meta = ctl("stop", "meta", timeout=25)
    final_meta["status"] = status_json("meta")
    final_meta["receipt"] = receipt("meta")
    final_meta["raw_lifecycle_snapshot"] = snapshot_lifecycle("final-stop-meta", "meta")
    events.append({"final_meta_stop": final_meta})
    assert final_meta["returncode"] == 0 and final_meta["receipt"]["receipt"]["exit_code"] == "0", final_meta
    report = {"status": "PASS", "events": events}
except BaseException as e:
    report = {"status": "FAIL", "error": repr(e), "events": events}
    raise
finally:
    if meta_paused:
        try:
            pid = int((RUN / "run/meta.pid").read_text().strip())
            os.kill(pid, signal.SIGCONT)
        except Exception:
            pass
    for fd in list(fds):
        close_fd(fd)
    cleanup_events = report.setdefault("cleanup", [])
    stop_all_cleanup(cleanup_events)
    (RUN / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))
'''


def run_product(evidence: pathlib.Path, controller: pathlib.Path) -> None:
    if not controller.exists():
        raise FileNotFoundError(controller)
    evidence.mkdir(parents=True, exist_ok=True)
    ctl_hash = sha256_file(controller)
    host_manifest(evidence, "run", controller)
    lima_copy(str(controller), f"afs-accept-a:{GUEST_RUN}/prefix/bin/afs-processctl")
    chmod = f'chmod +x "{GUEST_RUN}/prefix/bin/afs-processctl"; sha256sum "{GUEST_RUN}/prefix/bin/afs-processctl" > "{GUEST_RUN}/controller.sha256"'
    lima("afs-accept-a", chmod, timeout=30)
    runner = (
        GUEST_RUNNER.replace("__GUEST_RUN__", str(GUEST_RUN))
        .replace("__META_REST__", str(PORTS["meta_rest"]))
        .replace("__NODE_REST__", str(PORTS["node_rest"]))
        .replace("__NODE_SHA__", NODE_SHA)
        .replace("__META_SHA__", META_SHA)
        .replace("__CTL_SHA__", ctl_hash)
    )
    with tempfile.TemporaryDirectory(prefix="afs-runtime-probe-") as td:
        local_runner = pathlib.Path(td) / "guest-runner.py"
        local_runner.write_text(runner)
        lima_copy(str(local_runner), f"afs-accept-a:{GUEST_RUN}/guest-runner.py")
    proc = lima("afs-accept-a", f'sudo python3 "{GUEST_RUN}/guest-runner.py"', timeout=240, check=False)
    write_text(evidence / "runtime.stdout", proc.stdout)
    write_text(evidence / "runtime.stderr", proc.stderr)
    pull_guest_files(
        evidence,
        [
            "report.json",
            "controller.sha256",
            "run/meta.identity",
            "run/node.identity",
            "config.sha256",
            "staged-binaries.sha256",
            "etc/meta.toml",
            "etc/node.toml",
            "log/meta.log",
            "log/node.log",
        ],
    )
    status = "PASS" if proc.returncode == 0 else "FAIL"
    report_path = evidence / "guest" / "report.json"
    if report_path.exists():
        try:
            status = json.loads(report_path.read_text()).get("status", status)
        except json.JSONDecodeError:
            pass
    run_id = None
    if report_path.exists():
        try:
            report = json.loads(report_path.read_text())
            for event in report.get("events", []):
                if "environment" in event:
                    run_id = event["environment"].get("run_id")
                    break
        except json.JSONDecodeError:
            pass
    if run_id:
        artifacts_target = evidence / "guest" / f"probe-artifacts-{run_id}"
        lima_copy(f"afs-accept-a:{GUEST_RUN}/probe-artifacts/{run_id}", str(artifacts_target))
    write_text(
        evidence / "README.md",
        textwrap.dedent(
            f"""\
            # processctl v50 runtime product probe

            Status: {status}.

            Controller SHA256: `{ctl_hash}`.
            Node SHA256: `{NODE_SHA}`.
            Meta SHA256: `{META_SHA}`.

            Runtime path: `{GUEST_RUN}` on `afs-accept-a`, ext4 delivery volume.
            Ports: `{PORTS["meta_grpc"]}..{PORTS["node_rest"]}`.

            This is a bounded memory Meta/R1/gRPC/TLS product lifecycle proof.
            It is not a full DEP/REL acceptance gate.
            """
        ),
    )
    if proc.returncode != 0 or status != "PASS":
        raise SystemExit(proc.returncode or 1)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--evidence", type=pathlib.Path, default=DEFAULT_EVIDENCE)
    parser.add_argument("--controller", type=pathlib.Path, default=ROOT / "source/scripts/deploy/afs-processctl")
    parser.add_argument("action", choices=("prepare", "run"))
    args = parser.parse_args()
    if args.action == "prepare":
        prepare(args.evidence)
    else:
        run_product(args.evidence, args.controller)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
