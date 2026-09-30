#!/usr/bin/env python3
"""Host orchestrator for the isolated v51 shutdown-signal runtime probe.

This wrapper intentionally imports the processctl runtime probe helpers instead
of copying its host orchestration utilities. It owns only the v51 runtime path
and evidence directory.
"""
from __future__ import annotations

import argparse
import importlib.util
import json
import pathlib
import tempfile
import textwrap
import time


ROOT = pathlib.Path(__file__).resolve().parents[2]
BASE_HELPER = ROOT / "experiments/afs-acceptance/processctl-runtime-probe.py"
DEFAULT_EVIDENCE = ROOT / "evidence/afs-delivery/p2-shutdown-signal-v51/runtime-product"
GUEST_RUN = pathlib.PurePosixPath("/mnt/lima-afsadata/afs-delivery/shutdown-signal-v51")
BUILD_ARTIFACTS = pathlib.PurePosixPath("/home/lzc.guest/afs-build/artifacts/v51-qualified")
TLS_DIR = pathlib.PurePosixPath("/mnt/lima-afsadata/afs-delivery/p1b/tls")
PORTS = {
    "meta_grpc": 18180,
    "meta_rest": 18181,
    "node_grpc": 18182,
    "node_rest": 18183,
}


def load_base():
    spec = importlib.util.spec_from_file_location("afs_processctl_runtime_probe", BASE_HELPER)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot import helper: {BASE_HELPER}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def configure_base(base, node_sha: str | None = None, meta_sha: str | None = None):
    base.GUEST_RUN = GUEST_RUN
    base.BUILD_ARTIFACTS = BUILD_ARTIFACTS
    base.PORTS = PORTS
    if node_sha is not None:
        base.NODE_SHA = node_sha
    if meta_sha is not None:
        base.META_SHA = meta_sha
    return base


def write_json(path: pathlib.Path, data: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(data, indent=2) + "\n")


def write_text(path: pathlib.Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text)


def preflight(evidence: pathlib.Path) -> None:
    base = configure_base(load_base())
    evidence.mkdir(parents=True, exist_ok=True)
    script = f"""
set -euo pipefail
RUN="{GUEST_RUN}"
printf 'filesystem='
findmnt -rn -T /mnt/lima-afsadata -o FSTYPE,SOURCE,TARGET
[ "$(findmnt -rn -T /mnt/lima-afsadata -o FSTYPE)" = ext4 ]
if [ -e "$RUN" ]; then
  echo "runtime_exists=$RUN" >&2
  exit 30
fi
for port in {PORTS["meta_grpc"]} {PORTS["meta_rest"]} {PORTS["node_grpc"]} {PORTS["node_rest"]}; do
  if ss -ltnH | awk '{{print $4}}' | grep -Eq ":${{port}}$"; then
    echo "port_in_use=$port" >&2
    exit 31
  fi
done
for mount in "$RUN/mount-dfs" "$RUN/mount-ownerfs"; do
  if findmnt -rn --mountpoint "$mount" >/dev/null 2>&1; then
    echo "mount_present=$mount" >&2
    exit 32
  fi
done
if [ -e "{BUILD_ARTIFACTS}" ]; then
  find "{BUILD_ARTIFACTS}" -maxdepth 1 -type f -printf 'artifact=%p %s\\n' | sort
else
  echo "artifacts_missing={BUILD_ARTIFACTS}"
fi
"""
    proc = base.lima("afs-accept-a", script, timeout=30)
    manifest = {
        "phase": "preflight",
        "time_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "guest_run": str(GUEST_RUN),
        "build_artifacts": str(BUILD_ARTIFACTS),
        "ports": PORTS,
        "preflight_stdout": proc.stdout,
        "preflight_stderr": proc.stderr,
    }
    write_json(evidence / "manifest.json", manifest)
    write_text(
        evidence / "README.md",
        textwrap.dedent(
            f"""\
            # shutdown signal v51 runtime product probe

            Status: PREPARED / BLOCKED on v51 artifacts and exact Node/Meta SHAs.

            Preflight on `afs-accept-a` verified:

            - `{GUEST_RUN}` does not exist.
            - `/mnt/lima-afsadata` is ext4.
            - Ports `18180..18183` are clear.
            - No v51 FUSE mounts are present.

            No runtime was created, no controller was copied, and no product process
            was launched. The future run will use v51 artifacts from
            `{BUILD_ARTIFACTS}` after root provides exact hashes.
            """
        ),
    )


def prepare(evidence: pathlib.Path, node_sha: str, meta_sha: str) -> None:
    base = configure_base(load_base(), node_sha, meta_sha)
    evidence.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="afs-v51-") as td:
        tmp = pathlib.Path(td)
        for name, expected in (("afs-node", node_sha), ("afs-meta", meta_sha)):
            local = tmp / name
            base.lima_copy(f"afs-build:{BUILD_ARTIFACTS}/{name}", str(local))
            actual = base.sha256_file(local)
            if actual != expected:
                raise AssertionError(f"{name} sha mismatch: {actual} != {expected}")
        prep = f"""
set -euo pipefail
RUN="{GUEST_RUN}"
TLS="{TLS_DIR}"
if [ -e "$RUN" ]; then echo "refusing existing runtime: $RUN" >&2; exit 40; fi
for port in {PORTS["meta_grpc"]} {PORTS["meta_rest"]} {PORTS["node_grpc"]} {PORTS["node_rest"]}; do
  if ss -ltnH | awk '{{print $4}}' | grep -Eq ":${{port}}$"; then echo "port in use: $port" >&2; exit 41; fi
done
[ -d "$TLS" ] || {{ echo "missing TLS dir: $TLS" >&2; exit 42; }}
[ "$(findmnt -rn -T /mnt/lima-afsadata -o FSTYPE)" = ext4 ] || {{ echo "delivery volume not ext4" >&2; exit 43; }}
sudo install -d -o "$(id -u)" -g "$(id -g)" "$RUN/prefix/bin" "$RUN/etc" "$RUN/run" "$RUN/log" "$RUN/state/meta" "$RUN/state/node" "$RUN/mount-dfs" "$RUN/mount-ownerfs"
sudo chown "$(id -u):$(id -g)" "$RUN"
cat > "$RUN/etc/meta.toml" <<'EOF_META'
id = "shutdown-signal-v51-meta-a"
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
trusted_node_certs = {{ shutdown-signal-v51-node-a = "{TLS_DIR}/node-a.pem" }}
log_level = "info"
trace_enabled = false
EOF_META
cat > "$RUN/etc/node.toml" <<'EOF_NODE'
id = "shutdown-signal-v51-node-a"
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
trusted_node_certs = {{ shutdown-signal-v51-node-a = "{TLS_DIR}/node-a.pem" }}
log_level = "info"
trace_enabled = false
EOF_NODE
sha256sum "$RUN/etc/meta.toml" "$RUN/etc/node.toml" > "$RUN/config.sha256"
openssl x509 -in "$TLS/node-a.pem" -noout -subject -issuer -ext subjectAltName > "$RUN/tls-node-a-public.txt"
findmnt -rn -T "$RUN" -o FSTYPE,SOURCE,TARGET > "$RUN/findmnt.txt"
"""
        base.lima("afs-accept-a", prep, timeout=60)
        for name in ("afs-node", "afs-meta"):
            base.lima_copy(str(tmp / name), f"afs-accept-a:{GUEST_RUN}/prefix/bin/{name}")
        verify = f"""
set -euo pipefail
RUN="{GUEST_RUN}"
chmod +x "$RUN/prefix/bin/afs-node" "$RUN/prefix/bin/afs-meta"
sha256sum "$RUN/prefix/bin/afs-node" "$RUN/prefix/bin/afs-meta" > "$RUN/staged-binaries.sha256"
grep -R "'.*'" "$RUN/etc" && {{ echo "single quotes found in configs" >&2; exit 44; }} || true
"""
        base.lima("afs-accept-a", verify, timeout=60)
    base.pull_guest_files(
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
    write_json(
        evidence / "manifest.json",
        {
            "phase": "prepared",
            "time_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
            "guest_run": str(GUEST_RUN),
            "build_artifacts": str(BUILD_ARTIFACTS),
            "ports": PORTS,
            "expected_sha256": {"afs-node": node_sha, "afs-meta": meta_sha},
        },
    )


GUEST_RUNNER = r'''
#!/usr/bin/env python3
from __future__ import annotations
import hashlib, json, os, pathlib, platform, shutil, signal, subprocess, time, urllib.request

RUN = pathlib.Path("__GUEST_RUN__")
PORTS = {"meta_rest": __META_REST__, "node_rest": __NODE_REST__}
NODE_SHA = "__NODE_SHA__"
META_SHA = "__META_SHA__"
CTL_SHA = "__CTL_SHA__"
CTL = RUN / "prefix/bin/afs-processctl"
RUN_ID = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime()) + f"-pid{os.getpid()}"

def sha(path):
    return hashlib.sha256(pathlib.Path(path).read_bytes()).hexdigest()

def sh(cmd, timeout=60, check=True):
    p = subprocess.run(cmd, shell=True, text=True, capture_output=True, timeout=timeout)
    if check and p.returncode != 0:
        raise AssertionError({"cmd": cmd, "rc": p.returncode, "stdout": p.stdout, "stderr": p.stderr})
    return p

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

def proc_identity(service, expected_sha):
    pid = int((RUN / "run" / f"{service}.pid").read_text().strip())
    proc = pathlib.Path("/proc") / str(pid)
    exe = os.readlink(proc / "exe")
    actual_sha = hashlib.sha256((proc / "exe").read_bytes()).hexdigest()
    assert actual_sha == expected_sha, (service, actual_sha, expected_sha)
    stat = (proc / "stat").read_text().split(") ")[1].split()
    return {"pid": pid, "exe": exe, "sha256": actual_sha, "start_ticks": stat[19], "state": stat[0]}

def is_same_node(identity):
    current = proc_identity("node", NODE_SHA)
    assert current["pid"] == identity["pid"], (current, identity)
    assert current["start_ticks"] == identity["start_ticks"], (current, identity)
    assert current["sha256"] == identity["sha256"], (current, identity)
    return current

def status_json(service):
    lines = ctl("--json", "status", service, timeout=20, check=True)["stdout"].strip().splitlines()
    assert len(lines) == 1, lines
    return json.loads(lines[0])

def receipt(service):
    ident = {}
    ident_path = RUN / "run" / f"{service}.identity"
    for line in ident_path.read_text().splitlines():
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

def close_fd(fd):
    try:
        os.close(fd)
        return {"errno": None}
    except OSError as e:
        return {"errno": e.errno, "strerror": e.strerror}

def exact_mounts():
    mounts = {}
    for kind in ("dfs", "ownerfs"):
        p = sh(f"findmnt -J -M {RUN / ('mount-' + kind)} -o TARGET,FSTYPE,SOURCE", timeout=10)
        fs = json.loads(p.stdout)["filesystems"][0]
        assert fs["target"] == str(RUN / ("mount-" + kind)), fs
        assert fs["source"] == f"afs-{kind}", fs
        assert fs["fstype"].startswith("fuse"), fs
        mounts[kind] = fs
    return mounts

def stop_all_cleanup(events):
    for service in ("node", "meta"):
        try:
            events.append({f"cleanup_stop_{service}": ctl("stop", service, timeout=25)})
        except Exception as e:
            events.append({f"cleanup_stop_{service}_error": repr(e)})
    for mount in (RUN / "mount-dfs", RUN / "mount-ownerfs"):
        sh(f"if findmnt -rn --mountpoint {mount} >/dev/null 2>&1; then sudo fusermount3 -u {mount} || sudo umount {mount}; fi", timeout=10, check=False)

events = []
fds = []
meta_paused = False
try:
    assert platform.system() == "Linux"
    assert sha(CTL) == CTL_SHA
    assert sha(RUN / "prefix/bin/afs-node") == NODE_SHA
    assert sha(RUN / "prefix/bin/afs-meta") == META_SHA
    events.append({"environment": {
        "run_id": RUN_ID,
        "platform": platform.platform(),
        "boot_id": pathlib.Path("/proc/sys/kernel/random/boot_id").read_text().strip(),
        "filesystem": sh(f"findmnt -J -T {RUN} -o FSTYPE,SOURCE,TARGET", timeout=10).stdout,
        "controller_sha256": CTL_SHA,
        "binary_sha256": {"afs-node": NODE_SHA, "afs-meta": META_SHA},
        "config_sha256": {"node": sha(RUN / "etc/node.toml"), "meta": sha(RUN / "etc/meta.toml")},
    }})
    start = ctl("start", "all", timeout=60)
    events.append({"start_all": start})
    assert start["returncode"] == 0, start
    ready(PORTS["meta_rest"]); ready(PORTS["node_rest"])
    events.append({"identity_initial": {"meta": proc_identity("meta", META_SHA), "node": proc_identity("node", NODE_SHA), "mounts": exact_mounts()}})

    normal_path = RUN / f"mount-dfs/normal-drain-{RUN_ID}.bin"
    fd = os.open(normal_path, os.O_CREAT | os.O_EXCL | os.O_RDWR, 0o600); fds.append(fd)
    assert os.write(fd, b"accepted-dirty") == len(b"accepted-dirty")
    normal_stop = ctl("stop", "node", timeout=25)
    normal_stop["old_fd_close"] = close_fd(fd); fds.remove(fd)
    normal_stop["status"] = status_json("node")
    normal_stop["receipt"] = receipt("node")
    normal_stop["raw_lifecycle_snapshot"] = snapshot_lifecycle("normal-stop-node", "node")
    events.append({"normal_stop": normal_stop})
    assert normal_stop["returncode"] == 0, normal_stop
    assert normal_stop["status"]["exit_code"] == "0"
    assert normal_stop["receipt"]["receipt"]["exit_code"] == "0"

    restart = ctl("start", "node", timeout=60)
    events.append({"normal_restart": restart})
    assert restart["returncode"] == 0, restart
    ready(PORTS["node_rest"]); exact_mounts()
    assert normal_path.read_bytes() == b"accepted-dirty"
    events.append({"normal_readback": normal_path.read_bytes().hex(), "identity_normal_restart": proc_identity("node", NODE_SHA)})

    forced_path = RUN / f"mount-dfs/forced-signal-{RUN_ID}.bin"
    fd = os.open(forced_path, os.O_CREAT | os.O_EXCL | os.O_RDWR, 0o600)
    assert os.write(fd, b"seed") == 4
    os.fsync(fd); os.close(fd)
    assert forced_path.read_bytes() == b"seed"
    fd = os.open(forced_path, os.O_RDWR); fds.append(fd)
    node_before_signal = proc_identity("node", NODE_SHA)
    meta_before = proc_identity("meta", META_SHA)
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
    dirty_count = os.write(fd, b"next")
    dirty_elapsed = time.monotonic() - write_started
    assert dirty_count == 4
    verified_before_signal = is_same_node(node_before_signal)
    log_path = RUN / "log/node.log"
    log_offset = log_path.stat().st_size if log_path.exists() else 0
    signal_started = time.monotonic()
    os.kill(node_before_signal["pid"], signal.SIGTERM)
    receipt_data = None
    receipt_elapsed = None
    deadline = time.monotonic() + 18
    while time.monotonic() < deadline:
        try:
            receipt_data = receipt("node")
            receipt_elapsed = time.monotonic() - signal_started
            break
        except Exception:
            time.sleep(.05)
    assert receipt_data is not None, "node receipt not published within 18s after direct SIGTERM"
    observed = ctl("stop", "node", timeout=25)
    observed["old_fd_close"] = close_fd(fd); fds.remove(fd)
    observed["status"] = status_json("node")
    observed["receipt"] = receipt_data
    observed["direct_signal"] = {
        "signal": "SIGTERM",
        "node_before_signal": node_before_signal,
        "node_verified_before_signal": verified_before_signal,
        "signal_to_receipt_seconds": receipt_elapsed,
    }
    observed["raw_lifecycle_snapshot"] = snapshot_lifecycle("direct-signal-stop-node", "node")
    log_after = log_path.read_text(errors="replace")[log_offset:] if log_path.exists() else ""
    observed["log_after_signal"] = log_after
    events.append({"fault": {
        "signal": "SIGSTOP",
        "meta": meta_before,
        "state": state,
        "dirty_write_after_t": {"path": str(forced_path), "bytes": dirty_count, "elapsed_seconds": dirty_elapsed},
    }})
    events.append({"direct_signal_stop": observed})
    code = int(receipt_data["receipt"]["exit_code"])
    assert observed["returncode"] == code, observed
    assert code in (1, 124), observed
    assert observed["status"]["state"] == "failed", observed
    assert observed["status"]["exit_code"] == str(code), observed
    assert receipt_elapsed is not None and receipt_elapsed < 18, observed
    if code == 1:
        assert "dfs.node_drain_incomplete" in log_after, observed
        assert "node.shutdown_failed" in log_after, observed
    if code == 124:
        assert "process.shutdown_forced" in log_after, observed

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
    assert forced_path.read_bytes() == b"seed"
    assert normal_path.read_bytes() == b"accepted-dirty"
    events.append({"fresh_readback": {"forced_path": forced_path.read_bytes().hex(), "watermark": normal_path.read_bytes().hex()}, "identity_forced_restart": proc_identity("node", NODE_SHA)})

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
            os.kill(int((RUN / "run/meta.pid").read_text().strip()), signal.SIGCONT)
        except Exception:
            pass
    for fd in list(fds):
        close_fd(fd)
    cleanup_events = report.setdefault("cleanup", [])
    stop_all_cleanup(cleanup_events)
    (RUN / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))
'''


def run_product(evidence: pathlib.Path, controller: pathlib.Path, node_sha: str, meta_sha: str) -> None:
    base = configure_base(load_base(), node_sha, meta_sha)
    if not controller.exists():
        raise FileNotFoundError(controller)
    evidence.mkdir(parents=True, exist_ok=True)
    ctl_sha = base.sha256_file(controller)
    base.lima_copy(str(controller), f"afs-accept-a:{GUEST_RUN}/prefix/bin/afs-processctl")
    base.lima("afs-accept-a", f'chmod +x "{GUEST_RUN}/prefix/bin/afs-processctl"; sha256sum "{GUEST_RUN}/prefix/bin/afs-processctl" > "{GUEST_RUN}/controller.sha256"', timeout=30)
    runner = (
        GUEST_RUNNER.replace("__GUEST_RUN__", str(GUEST_RUN))
        .replace("__META_REST__", str(PORTS["meta_rest"]))
        .replace("__NODE_REST__", str(PORTS["node_rest"]))
        .replace("__NODE_SHA__", node_sha)
        .replace("__META_SHA__", meta_sha)
        .replace("__CTL_SHA__", ctl_sha)
    )
    with tempfile.TemporaryDirectory(prefix="afs-v51-signal-runner-") as td:
        local_runner = pathlib.Path(td) / "guest-runner.py"
        local_runner.write_text(runner)
        base.lima_copy(str(local_runner), f"afs-accept-a:{GUEST_RUN}/guest-runner.py")
    proc = base.lima("afs-accept-a", f'sudo python3 "{GUEST_RUN}/guest-runner.py"', timeout=240, check=False)
    write_text(evidence / "runtime.stdout", proc.stdout)
    write_text(evidence / "runtime.stderr", proc.stderr)
    base.pull_guest_files(
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
    report_path = evidence / "guest/report.json"
    status = "PASS" if proc.returncode == 0 else "FAIL"
    run_id = None
    if report_path.exists():
        report = json.loads(report_path.read_text())
        status = report.get("status", status)
        for event in report.get("events", []):
            if "environment" in event:
                run_id = event["environment"].get("run_id")
                break
    if run_id:
        base.lima_copy(f"afs-accept-a:{GUEST_RUN}/probe-artifacts/{run_id}", str(evidence / "guest" / f"probe-artifacts-{run_id}"))
    write_json(
        evidence / "manifest.json",
        {
            "phase": "run",
            "time_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
            "status": status,
            "guest_run": str(GUEST_RUN),
            "ports": PORTS,
            "controller": {"path": str(controller), "sha256": ctl_sha},
            "expected_sha256": {"afs-node": node_sha, "afs-meta": meta_sha},
            "run_id": run_id,
        },
    )
    write_text(
        evidence / "README.md",
        textwrap.dedent(
            f"""\
            # shutdown signal v51 runtime product probe

            Status: {status}.

            Runtime path: `{GUEST_RUN}` on `afs-accept-a`, ext4 delivery volume.
            Ports: `18180..18183`.

            Controller SHA256: `{ctl_sha}`.
            Node SHA256: `{node_sha}`.
            Meta SHA256: `{meta_sha}`.
            Run id: `{run_id}`.

            This is a bounded memory Meta/R1/gRPC/TLS direct-SIGTERM shutdown
            proof. It is not a full DEP/REL acceptance gate.
            """
        ),
    )
    if proc.returncode != 0 or status != "PASS":
        raise SystemExit(proc.returncode or 1)


def collect_existing(evidence: pathlib.Path, node_sha: str, meta_sha: str) -> None:
    base = configure_base(load_base(), node_sha, meta_sha)
    evidence.mkdir(parents=True, exist_ok=True)
    base.pull_guest_files(
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
    report_path = evidence / "guest/report.json"
    report = json.loads(report_path.read_text())
    status = report.get("status", "UNKNOWN")
    run_id = None
    for event in report.get("events", []):
        if "environment" in event:
            run_id = event["environment"].get("run_id")
            break
    if run_id:
        base.lima_copy(f"afs-accept-a:{GUEST_RUN}/probe-artifacts/{run_id}", str(evidence / "guest" / f"probe-artifacts-{run_id}"))
    controller_sha = ""
    controller_file = evidence / "guest/controller.sha256"
    if controller_file.exists():
        controller_sha = controller_file.read_text().split()[0]
    write_json(
        evidence / "manifest.json",
        {
            "phase": "collected",
            "time_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
            "status": status,
            "guest_run": str(GUEST_RUN),
            "ports": PORTS,
            "controller_sha256": controller_sha,
            "expected_sha256": {"afs-node": node_sha, "afs-meta": meta_sha},
            "run_id": run_id,
        },
    )
    write_text(
        evidence / "README.md",
        textwrap.dedent(
            f"""\
            # shutdown signal v51 runtime product probe

            Status: {status}.

            Runtime path: `{GUEST_RUN}` on `afs-accept-a`, ext4 delivery volume.
            Ports: `18180..18183`.

            Controller SHA256: `{controller_sha}`.
            Node SHA256: `{node_sha}`.
            Meta SHA256: `{meta_sha}`.
            Run id: `{run_id}`.

            This is a bounded memory Meta/R1/gRPC/TLS direct-SIGTERM shutdown
            proof. It is not a full DEP/REL acceptance gate.

            Raw report: `guest/report.json`.
            Raw lifecycle snapshots: `guest/probe-artifacts-{run_id}/`.
            """
        ),
    )


def require_shas(args: argparse.Namespace) -> tuple[str, str]:
    if not args.node_sha or not args.meta_sha:
        raise SystemExit("--node-sha and --meta-sha are required for prepare/run")
    return args.node_sha, args.meta_sha


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--evidence", type=pathlib.Path, default=DEFAULT_EVIDENCE)
    parser.add_argument("--controller", type=pathlib.Path, default=ROOT / "source/scripts/deploy/afs-processctl")
    parser.add_argument("--node-sha")
    parser.add_argument("--meta-sha")
    parser.add_argument("action", choices=("preflight", "prepare", "run", "collect"))
    args = parser.parse_args()
    if args.action == "preflight":
        preflight(args.evidence)
    elif args.action == "prepare":
        node_sha, meta_sha = require_shas(args)
        prepare(args.evidence, node_sha, meta_sha)
    else:
        node_sha, meta_sha = require_shas(args)
        if args.action == "run":
            run_product(args.evidence, args.controller, node_sha, meta_sha)
        else:
            collect_existing(args.evidence, node_sha, meta_sha)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
