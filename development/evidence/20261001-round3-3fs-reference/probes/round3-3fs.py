#!/usr/bin/env python3
"""Round 3 isolated 3FS qualifier driver for Linux aarch64 guests.

This helper prepares and drives one role at a time.  It does not allocate VMs,
start global services, or touch any process whose executable identity does not
match this run's owned prefix.
"""
from __future__ import annotations

import argparse
import errno
import hashlib
import json
import os
import pathlib
import platform
import resource
import signal
import subprocess
import sys
import time

TOKEN = "AADHHSOs8QA92iRe2wB1fmuL"
CLUSTER_ID = "afs_3fs_round3_v84"
FDB_KEY = "afs3fsround3v84"
ROOT_NAME = "3fs-round3-v84"
TEMPLATE = pathlib.Path("/home/lzc.guest/3fs-round3-templates")
CTL_IP = "192.168.109.11"
PORTS = {"fdb": 19000, "mgmtd": 19001, "meta": 19002, "storage": 19003}
VOLUMES = {
    "ctl": pathlib.Path("/mnt/lima-afsctlstate"),
    "a": pathlib.Path("/mnt/lima-afsadata"),
    "b": pathlib.Path("/mnt/lima-afsbdata"),
    "c": pathlib.Path("/mnt/lima-afscdata"),
}
ROLE_IP = {"ctl": CTL_IP, "a": "192.168.109.12", "b": "192.168.109.13", "c": "192.168.109.14"}
STORAGE_NODE = {"a": 10000, "b": 10001, "c": 10002}
PREFIX_BIN = {
    "ctl": pathlib.Path("/opt/afs-3fs-round3-v84/bin"),
    "a": pathlib.Path("/opt/afs-3fs-round3-v84/bin"),
    "b": pathlib.Path("/opt/afs-3fs-round3-v84/bin"),
    "c": pathlib.Path("/opt/afs-3fs-round3-v84/bin"),
}
PREFIX_ROOT = pathlib.Path("/opt/afs-3fs-round3-v84")
PREFIX_LIB = PREFIX_ROOT / "lib"
MANIFEST = PREFIX_ROOT / "manifest.json"
SERVICES = {
    "fdb": "fdbserver",
    "mgmtd": "mgmtd_main",
    "meta": "meta_main",
    "storage": "storage_main",
    "fuse": "hf3fs_fuse_main",
}
JEMALLOC = PREFIX_LIB / "libjemalloc.so.2"
FORMAL = {"formal_acceptance": "NOT_RUN", "environment": "PREPARING"}
ATTEMPT = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime()) + f"-pid{os.getpid()}"


class DriverError(RuntimeError):
    pass


def root(role: str) -> pathlib.Path:
    return VOLUMES[role] / ROOT_NAME


def now() -> str:
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())


def sha256_file(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def require_guest() -> None:
    if platform.system() != "Linux" or platform.machine() != "aarch64":
        raise DriverError(f"Linux aarch64 required, got {platform.system()} {platform.machine()}")
    if os.geteuid() != 0:
        raise DriverError("root required")
    source = pathlib.Path(__file__).resolve()
    if str(source).startswith(("/Users/", "/mnt/")):
        raise DriverError(f"run the immutable guest copy, not a VirtioFS host path: {source}")
    resource.setrlimit(resource.RLIMIT_NOFILE, (1048576, 1048576))


def ensure_dirs(run: pathlib.Path) -> None:
    for name in ("config", "data", "evidence", "log", "mount", "run"):
        (run / name).mkdir(parents=True, exist_ok=True)


def guard_volume(role: str) -> dict:
    stat = os.statvfs(VOLUMES[role])
    available = stat.f_bavail * stat.f_frsize
    record = {"volume": str(VOLUMES[role]), "available_bytes": available, "required_bytes": 4 * 1024**3}
    if available < record["required_bytes"]:
        raise DriverError(f"volume reserve below 4GiB: {record}")
    return record


def save_json(run: pathlib.Path, label: str, value: dict) -> pathlib.Path:
    value = dict(value)
    value.update(FORMAL, utc=now())
    path = run / "evidence" / f"{label}-{ATTEMPT}.json"
    if path.exists():
        raise DriverError(f"exclusive evidence exists: {path}")
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return path


def cmd_record(run: pathlib.Path, label: str, argv: list[str], timeout: int = 60, check: bool = True) -> dict:
    stdout = run / "log" / f"{label}-{ATTEMPT}.stdout"
    stderr = run / "log" / f"{label}-{ATTEMPT}.stderr"
    receipt = run / "evidence" / f"{label}-{ATTEMPT}.command.json"
    for path in (stdout, stderr, receipt):
        if path.exists():
            raise DriverError(f"exclusive command artifact exists: {path}")
    started = time.monotonic()
    env = os.environ.copy()
    env["LD_LIBRARY_PATH"] = str(PREFIX_LIB)
    with stdout.open("wb") as out, stderr.open("wb") as err:
        proc = subprocess.run(argv, stdout=out, stderr=err, timeout=timeout, check=False, env=env)
    record = {
        "argv": argv,
        "exit": proc.returncode,
        "elapsed_seconds": time.monotonic() - started,
        "stdout": str(stdout),
        "stderr": str(stderr),
    }
    receipt.write_text(json.dumps(record, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    if check and proc.returncode != 0:
        raise DriverError(f"{label} failed exit={proc.returncode}")
    return record


def alive(pid: int) -> bool:
    return (pathlib.Path("/proc") / str(pid)).exists()


def proc_state(pid: int) -> str | None:
    try:
        text = (pathlib.Path("/proc") / str(pid) / "stat").read_text(encoding="utf-8")
    except FileNotFoundError:
        return None
    return text[text.rindex(")") + 2 :].split()[0]


def terminated(pid: int) -> bool:
    state = proc_state(pid)
    return state is None or state == "Z"


def proc_start_ticks(pid: int) -> str:
    text = (pathlib.Path("/proc") / str(pid) / "stat").read_text(encoding="utf-8")
    return text[text.rindex(")") + 2 :].split()[19]


def identity(pid: int, expected_exe: pathlib.Path | None = None) -> dict:
    proc = pathlib.Path("/proc") / str(pid)
    exe = pathlib.Path(os.readlink(proc / "exe")).resolve()
    if expected_exe is not None and exe != expected_exe.resolve():
        raise DriverError(f"pid {pid} exe mismatch: {exe} != {expected_exe.resolve()}")
    return {"pid": pid, "start_ticks": proc_start_ticks(pid), "exe": str(exe), "sha256": sha256_file(proc / "exe")}


def saved_identity(run: pathlib.Path, service: str) -> dict:
    path = run / "run" / f"{service}.identity.json"
    if not path.exists():
        raise DriverError(f"missing saved identity for owned stop: {path}")
    return json.loads(path.read_text(encoding="utf-8"))


def verify_saved_identity(run: pathlib.Path, service: str, pid: int) -> dict:
    saved = saved_identity(run, service)
    live = identity(pid, service_exe(run_role(run), service))
    for key in ("pid", "start_ticks", "sha256", "exe"):
        if str(live.get(key)) != str(saved.get(key)):
            raise DriverError(f"{service} saved identity mismatch at {key}: {live.get(key)} != {saved.get(key)}")
    return live


def run_role(run: pathlib.Path) -> str:
    for role in VOLUMES:
        if run == root(role):
            return role
    raise DriverError(f"unknown run root: {run}")


def service_exe(role: str, service: str) -> pathlib.Path:
    return PREFIX_BIN[role] / SERVICES[service]


def flatten_manifest(value: object) -> dict[str, str]:
    if isinstance(value, dict) and isinstance(value.get("files"), dict):
        value = value["files"]
    if isinstance(value, dict):
        out = {}
        for key, item in value.items():
            if isinstance(item, str):
                out[str(key)] = item
            elif isinstance(item, dict) and isinstance(item.get("sha256"), str):
                out[str(key)] = item["sha256"]
        return out
    if isinstance(value, list):
        out = {}
        for item in value:
            if isinstance(item, dict) and isinstance(item.get("path"), str) and isinstance(item.get("sha256"), str):
                out[item["path"]] = item["sha256"]
        return out
    return {}


def verify_manifest() -> dict:
    if not MANIFEST.exists():
        raise DriverError(f"missing prefix manifest: {MANIFEST}")
    raw = json.loads(MANIFEST.read_text(encoding="utf-8"))
    expected = flatten_manifest(raw)
    if not expected:
        raise DriverError(f"manifest has no recognizable file hashes: {MANIFEST}")
    checked = {}
    for rel, want in sorted(expected.items()):
        rel_path = pathlib.Path(rel)
        path = rel_path if rel_path.is_absolute() else PREFIX_ROOT / rel_path
        if not path.exists():
            raise DriverError(f"manifest file missing: {path}")
        got = sha256_file(path)
        if got != want:
            raise DriverError(f"manifest hash mismatch: {path} {got} != {want}")
        checked[str(path)] = got
    return {"manifest": str(MANIFEST), "checked": checked}


def preflight(role: str) -> dict:
    run = root(role)
    bins = sorted({"admin_cli", "hf3fs_fuse_main", "storage_main"} if role != "ctl" else {"admin_cli", "fdbcli", "fdbserver", "meta_main", "mgmtd_main"})
    if not TEMPLATE.is_dir():
        raise DriverError(f"missing templates: {TEMPLATE}")
    hashes = {}
    for binary in bins:
        path = PREFIX_BIN[role] / binary
        if not path.exists():
            raise DriverError(f"missing required binary: {path}")
        hashes[binary] = sha256_file(path)
    templates = {p.name: sha256_file(p) for p in sorted(TEMPLATE.glob("*.toml"))}
    needed = {"admin_cli.toml", "hf3fs_fuse_main.toml", "hf3fs_fuse_main_launcher.toml", "storage_main.toml", "storage_main_app.toml", "storage_main_launcher.toml"}
    if role == "ctl":
        needed |= {"meta_main.toml", "meta_main_app.toml", "meta_main_launcher.toml", "mgmtd_main.toml", "mgmtd_main_app.toml", "mgmtd_main_launcher.toml"}
    missing = sorted(needed - set(templates))
    if missing:
        raise DriverError(f"missing template files: {missing}")
    if not JEMALLOC.exists():
        raise DriverError(f"missing mandatory owned jemalloc: {JEMALLOC}")
    return {"role": role, "root": str(run), "bin_root": str(PREFIX_BIN[role]), "lib_root": str(PREFIX_LIB), "volume_guard": guard_volume(role), "manifest": verify_manifest(), "binary_sha256": hashes, "template_sha256": templates, "source_sha256": sha256_file(pathlib.Path(__file__).resolve())}


def rewrite_template(text: str, role: str, name: str) -> str:
    run = root(role)
    data = run / "data"
    log = run / "log"
    config = run / "config"
    replacements = {
        "/mnt/lima-afsadata/3fs-baseline-arm64-patched/runs/a-only-qualify32-20260930T113654Z": str(run),
        "afs_3fs_patched_qualify32": CLUSTER_ID,
        "192.168.109.12:19001": f"{CTL_IP}:{PORTS['mgmtd']}",
        "127.0.0.1:19000": f"{CTL_IP}:{PORTS['fdb']}",
    }
    for old, new in replacements.items():
        text = text.replace(old, new)
    text = text.replace("listen_port = 19001", f"listen_port = {PORTS['mgmtd']}")
    text = text.replace("listen_port = 19002", f"listen_port = {PORTS['meta']}")
    text = text.replace("listen_port = 19003", f"listen_port = {PORTS['storage']}")
    text = text.replace("fsync_length_hint = true", "fsync_length_hint = false")
    if name == "hf3fs_fuse_main.toml" and "fdatasync_update_length" not in text:
        text = text.replace("fsync_length_hint = false", "fsync_length_hint = false\nfdatasync_update_length = true")
    if "clusterFile =" in text:
        text = replace_line(text, "clusterFile =", f"clusterFile = '{data / 'foundationdb/fdb.cluster'}'")
    if "token_file =" in text:
        text = replace_line(text, "token_file =", f"token_file = '{config / 'token'}'")
    if name == "admin_cli.toml":
        text = replace_line(text, "token =", f"token = '{TOKEN}'")
    log_map = {"mgmtd_main.toml": "mgmtd.log", "meta_main.toml": "meta.log", "storage_main.toml": "storage.log", "hf3fs_fuse_main.toml": "fuse.log"}
    if name in log_map and "file_path =" in text:
        text = replace_line(text, "file_path =", f"file_path = '{log / log_map[name]}'")
    if name == "storage_main.toml":
        target = data / "storage/data1"
        text = replace_line(text, "target_paths =", f"target_paths = [ '{target}' ]")
    return text if text.endswith("\n") else text + "\n"


def replace_line(text: str, prefix: str, value: str) -> str:
    lines = [value if line.strip().startswith(prefix) else line for line in text.splitlines()]
    return "\n".join(lines) + "\n"


def write_config(role: str) -> dict:
    run = root(role)
    config = run / "config"
    for src in TEMPLATE.glob("*.toml"):
        (config / src.name).write_text(rewrite_template(src.read_text(encoding="utf-8"), role, src.name), encoding="utf-8")
    app_nodes = {"mgmtd_main_app.toml": 1, "meta_main_app.toml": 50}
    if role in STORAGE_NODE:
        app_nodes["storage_main_app.toml"] = STORAGE_NODE[role]
    for filename, node_id in app_nodes.items():
        (config / filename).write_text(f"allow_empty_node_id = false\nnode_id = {node_id}\n", encoding="utf-8")
    (config / "token").write_text(TOKEN + "\n", encoding="utf-8")
    (run / "data" / "foundationdb").mkdir(parents=True, exist_ok=True)
    (run / "data" / "storage" / "data1").mkdir(parents=True, exist_ok=True)
    (run / "mount").mkdir(parents=True, exist_ok=True)
    fuse = config / "hf3fs_fuse_main.toml"
    if fuse.exists():
        text = fuse.read_text(encoding="utf-8")
        if "fsync_length_hint = false" not in text or "fdatasync_update_length = true" not in text:
            raise DriverError("fuse config must set fsync_length_hint=false and fdatasync_update_length=true")
    return {p.name: sha256_file(p) for p in sorted(config.glob("*")) if p.is_file()}


def prepare(role: str) -> dict:
    run = root(role)
    ensure_dirs(run)
    cluster = f"round3:{FDB_KEY}@{CTL_IP}:{PORTS['fdb']}\n"
    cfg_hashes = write_config(role)
    (run / "data/foundationdb/fdb.cluster").write_text(cluster, encoding="utf-8")
    cfg_hashes["fdb.cluster"] = sha256_file(run / "data/foundationdb/fdb.cluster")
    topo = {"role": role, "role_ip": ROLE_IP[role], "ports": PORTS, "cluster_file": cluster.strip(), "node_id": STORAGE_NODE.get(role)}
    return {"status": "PASS", "preflight": preflight(role), "config_sha256": cfg_hashes, "topology": topo}


def fdb_cli(run: pathlib.Path, label: str, command: str, timeout: int = 60) -> dict:
    return cmd_record(run, label, [str(PREFIX_BIN["ctl"] / "fdbcli"), "-C", str(run / "data/foundationdb/fdb.cluster"), "--exec", command], timeout=timeout)


def start_service(role: str, service: str, argv: list[str], env: dict[str, str] | None = None) -> dict:
    run = root(role)
    pidfile = run / "run" / f"{service}.pid"
    exe = service_exe(role, service)
    if pidfile.exists():
        pid = int(pidfile.read_text(encoding="utf-8").strip())
        if not terminated(pid):
            return {"service": service, "already_running": True, "identity": identity(pid, exe)}
    stdout_path = run / "log" / f"{service}-{ATTEMPT}.stdout"
    stderr_path = run / "log" / f"{service}-{ATTEMPT}.stderr"
    if stdout_path.exists() or stderr_path.exists():
        raise DriverError(f"exclusive service log exists for {service}")
    out = stdout_path.open("wb")
    err = stderr_path.open("wb")
    child_env = os.environ.copy()
    child_env["LD_LIBRARY_PATH"] = str(PREFIX_LIB)
    if env:
        child_env.update(env)
    proc = subprocess.Popen(argv, stdin=subprocess.DEVNULL, stdout=out, stderr=err, start_new_session=True, env=child_env)
    out.close()
    err.close()
    pidfile.write_text(str(proc.pid) + "\n", encoding="utf-8")
    time.sleep(2)
    if proc.poll() is not None:
        raise DriverError(f"{service} exited early with {proc.returncode}")
    ident = identity(proc.pid, exe)
    (run / "run" / f"{service}.identity.json").write_text(json.dumps(ident, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return {"service": service, "pid": proc.pid, "identity": ident, "argv": argv, "stdout": str(stdout_path), "stderr": str(stderr_path)}


def start_control() -> dict:
    role = "ctl"
    run = root(role)
    (run / "data/foundationdb/fdb.cluster").write_text(f"round3:{FDB_KEY}@{CTL_IP}:{PORTS['fdb']}\n", encoding="utf-8")
    fdb = start_service(role, "fdb", [str(PREFIX_BIN[role] / "fdbserver"), "-p", f"{CTL_IP}:{PORTS['fdb']}", "-m", "1GiB", "--cache-memory", "128MiB", "--storage-memory", "128MiB", "--data-filesystem", str(VOLUMES["ctl"]), "-d", str(run / "data/foundationdb"), "-L", str(run / "log"), "-C", str(run / "data/foundationdb/fdb.cluster")])
    time.sleep(5)
    fdb_configured = run / "run" / "fdb-configured.json"
    initialized = run / "run" / "control-initialized.json"
    if fdb_configured.exists() or initialized.exists():
        configure = {"skipped": "fdb_configured_marker_exists"}
    else:
        configure = fdb_cli(run, "fdb-configure-ssd-single", "configure new ssd single", timeout=120)
        fdb_configured.write_text(json.dumps({"utc": now(), "command": configure}, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    status = fdb_cli(run, "fdb-status-minimal", "status minimal", timeout=60)
    bootstrap = {"skipped": "initialized_marker_exists"}
    if not initialized.exists():
        bootstrap_records = [
            admin(run, "bootstrap-user-add-root", ["user-add", "--root", "--admin", "--token", TOKEN, "0", "root"]),
            admin(run, "bootstrap-user-set-token", ["user-set-token", "--new", "0"]),
            admin(run, "bootstrap-init-cluster", ["init-cluster", "--mgmtd", str(run / "config/mgmtd_main.toml"), "--meta", str(run / "config/meta_main.toml"), "--storage", str(run / "config/storage_main.toml"), "--fuse", str(run / "config/hf3fs_fuse_main.toml"), "--skip-config-check", "1", "524288", "1"], timeout=120),
        ]
        bootstrap = {"commands": bootstrap_records}
        initialized.write_text(json.dumps({"utc": now(), "commands": bootstrap_records}, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    env = {"LD_PRELOAD": str(JEMALLOC)}
    cfg = run / "config"
    mgmtd = start_service(role, "mgmtd", [str(PREFIX_BIN[role] / "mgmtd_main"), "--app_cfg", str(cfg / "mgmtd_main_app.toml"), "--launcher_cfg", str(cfg / "mgmtd_main_launcher.toml"), "--cfg", str(cfg / "mgmtd_main.toml")], env)
    meta = start_service(role, "meta", [str(PREFIX_BIN[role] / "meta_main"), "--app_cfg", str(cfg / "meta_main_app.toml"), "--launcher_cfg", str(cfg / "meta_main_launcher.toml"), "--cfg", str(cfg / "meta_main.toml")], env)
    return {"status": "PASS", "fdb_resource_budget": {"memory": "1GiB", "cache_memory": "128MiB", "storage_memory": "128MiB", "data_filesystem": str(VOLUMES["ctl"]), "fair_performance_default": False}, "fdb": fdb, "configure": configure, "fdb_status": status, "bootstrap": bootstrap, "mgmtd": mgmtd, "meta": meta}


def start_storage(role: str) -> dict:
    if role not in STORAGE_NODE:
        raise DriverError("start-storage is only valid for a/b/c")
    run = root(role)
    cfg = run / "config"
    result = start_service(role, "storage", [str(PREFIX_BIN[role] / "storage_main"), "--app_cfg", str(cfg / "storage_main_app.toml"), "--launcher_cfg", str(cfg / "storage_main_launcher.toml"), "--cfg", str(cfg / "storage_main.toml")])
    return {"status": "PASS", "storage": result}


def admin(run: pathlib.Path, label: str, args: list[str], timeout: int = 60) -> dict:
    return cmd_record(run, label, [str(PREFIX_BIN["ctl"] / "admin_cli"), "--cfg", str(run / "config/admin_cli.toml"), "--", *args], timeout=timeout)


def init_chain() -> dict:
    run = root("ctl")
    records = []
    initialized = run / "run" / "control-initialized.json"
    if not initialized.exists():
        raise DriverError("run start-control bootstrap before init-chain")
    chains = run / "config/chains.csv"
    table = run / "config/chain-table.csv"
    chain_id = "1"
    table.write_text("ChainId\n1\n", encoding="utf-8")
    targets = {}
    chain_targets = []
    for peer, node in STORAGE_NODE.items():
        target = f"{node}01001"
        targets[peer] = {"node": node, "target": target}
        chain_targets.append(target)
        records.append(admin(run, f"admin-create-target-{peer}", ["create-target", "--node-id", str(node), "--disk-index", "0", "--target-id", target, "--chain-id", chain_id]))
    chains.write_text("ChainId,TargetId,TargetId,TargetId\n" + ",".join([chain_id, *chain_targets]) + "\n", encoding="utf-8")
    records.append(admin(run, "admin-upload-chains", ["upload-chains", str(chains)]))
    records.append(admin(run, "admin-upload-chain-table", ["upload-chain-table", "1", str(table), "--desc", "round3-abc-replica-3"]))
    records.append(admin(run, "admin-list-nodes", ["list-nodes"]))
    records.append(admin(run, "admin-list-chains", ["list-chains"]))
    records.append(admin(run, "admin-list-chain-tables", ["list-chain-tables"]))
    records.append(admin(run, "admin-mkdir-test", ["mkdir", "--perm", "0755", "test"]))
    records.append(admin(run, "admin-set-perm-test", ["set-perm", "--uid", str(os.getuid()), "--gid", str(os.getgid()), "test"]))
    return {"status": "PASS", "chain_table_id": 1, "chunk_size": 524288, "stripe": 1, "replicas": 3, "targets": targets, "commands": records}


def mount(role: str) -> dict:
    if role not in ("a", "b"):
        raise DriverError("mount is only valid for a/b")
    run = root(role)
    mountpoint = run / "mount"
    result = start_service(role, "fuse", [str(PREFIX_BIN[role] / "hf3fs_fuse_main"), "--launcher_cfg", str(run / "config/hf3fs_fuse_main_launcher.toml"), "--cfg", str(run / "config/hf3fs_fuse_main.toml"), f"--launcher_config.mountpoint={mountpoint}"], {"LD_PRELOAD": str(JEMALLOC)})
    deadline = time.monotonic() + 45
    while time.monotonic() < deadline:
        if (mountpoint / "test").is_dir():
            return {"status": "PASS", "fuse": result, "test_dir": str(mountpoint / "test")}
        time.sleep(1)
    raise DriverError("mount did not expose /test within 45s")


def stop(role: str) -> dict:
    run = root(role)
    stopped = []
    if role in ("a", "b"):
        mounted = subprocess.run(["findmnt", "-M", str(run / "mount")], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False).returncode == 0
        if mounted:
            stopped.append({"mount": str(run / "mount"), "umount": cmd_record(run, "umount-fuse", ["fusermount3", "-u", str(run / "mount")], timeout=15, check=True)})
        else:
            stopped.append({"mount": str(run / "mount"), "umount": "not_mounted"})
    for service in ("fuse", "storage", "meta", "mgmtd", "fdb"):
        pidfile = run / "run" / f"{service}.pid"
        if not pidfile.exists():
            continue
        pid = int(pidfile.read_text(encoding="utf-8").strip())
        if terminated(pid):
            stopped.append({"service": service, "pid": pid, "already_terminated": True, "state": proc_state(pid)})
            continue
        ident = verify_saved_identity(run, service, pid)
        signals_sent = ["SIGTERM"]
        try:
            os.kill(pid, signal.SIGTERM)
        except ProcessLookupError:
            if not terminated(pid):
                raise
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline and not terminated(pid):
            time.sleep(0.2)
        if not terminated(pid):
            ident = verify_saved_identity(run, service, pid)
            signals_sent.append("SIGKILL")
            try:
                os.kill(pid, signal.SIGKILL)
            except ProcessLookupError:
                if not terminated(pid):
                    raise
            deadline = time.monotonic() + 5
            while time.monotonic() < deadline and not terminated(pid):
                time.sleep(0.2)
        final_state = proc_state(pid)
        final_terminated = terminated(pid)
        stopped.append({"service": service, "pid": pid, "identity": ident, "terminated": final_terminated, "final_state": final_state, "signals_sent": signals_sent, "forced_kill": "SIGKILL" in signals_sent})
        if not final_terminated:
            raise DriverError(f"{service} still running after SIGKILL: pid={pid} state={final_state}")
    return {"status": "PASS", "stopped": stopped}


def payload() -> bytes:
    return b"".join(hashlib.sha256(f"round3-3fs-v84-{i}".encode()).digest() * 32768 for i in range(32))


def qualifier_path(role: str, name: str) -> pathlib.Path:
    return root(role) / "mount" / "test" / name


def require_owned_mount(role: str) -> dict:
    run = root(role)
    pid = int((run / "run/fuse.pid").read_text(encoding="utf-8").strip())
    ident = verify_saved_identity(run, "fuse", pid)
    point = run / "mount"
    if not os.path.ismount(point):
        raise DriverError(f"owned FUSE mount absent: {point}")
    found = subprocess.run(["findmnt", "-M", str(point), "-J"], capture_output=True, text=True, check=True)
    mounts = json.loads(found.stdout)["filesystems"]
    if len(mounts) != 1 or not mounts[0].get("fstype", "").startswith("fuse"):
        raise DriverError(f"unexpected mounted filesystem: {mounts}")
    return {"identity": ident, "findmnt": mounts[0]}


def call_io(label: str, func) -> dict:
    started = time.monotonic()
    try:
        value = func()
        return {"label": label, "status": "OK", "value": value, "elapsed_seconds": time.monotonic() - started}
    except OSError as error:
        return {"label": label, "status": "ERROR", "errno": error.errno, "errno_name": errno.errorcode.get(error.errno), "error": str(error), "elapsed_seconds": time.monotonic() - started}


def write_qualifier(role: str, name: str) -> dict:
    if role not in ("a", "b"):
        raise DriverError("write is only valid for mounted a/b roles")
    require_owned_mount(role)
    data = payload()
    path = qualifier_path(role, name)
    fd = os.open(path, os.O_CREAT | os.O_EXCL | os.O_RDWR, 0o600)
    writes = []
    write_error = None
    fdatasync = {"label": "fdatasync", "status": "NOT_RUN"}
    fsync_file = {"label": "fsync_file", "status": "NOT_RUN"}
    try:
        for offset in range(0, len(data), 1024 * 1024):
            block = data[offset : offset + 1024 * 1024]
            outcome = call_io(f"write_{offset}", lambda block=block: os.write(fd, block))
            outcome["offset"] = offset
            outcome["expected_bytes"] = len(block)
            writes.append(outcome)
            if outcome["status"] != "OK" or outcome.get("value") != len(block):
                write_error = outcome
                break
        fdatasync = call_io("fdatasync", lambda: os.fdatasync(fd))
        fsync_file = call_io("fsync_file", lambda: os.fsync(fd))
    finally:
        os.close(fd)
    dirfd = os.open(str(path.parent), os.O_RDONLY | os.O_DIRECTORY)
    try:
        dirsync = call_io("fsync_directory", lambda: os.fsync(dirfd))
    finally:
        os.close(dirfd)
    observed = path.read_bytes() if path.exists() else b""
    content_ok = observed == data
    ok = write_error is None and content_ok and fdatasync["status"] == "OK" and fsync_file["status"] == "OK"
    gap = dirsync.get("errno") == errno.ENOSYS
    dir_ok = dirsync["status"] == "OK" or gap
    return {"status": "PASS" if ok and dir_ok else "FAIL", "file": str(path), "bytes": len(data), "sha256": hashlib.sha256(data).hexdigest(), "observed_bytes": len(observed), "observed_sha256": hashlib.sha256(observed).hexdigest(), "writes_1mib": writes, "write_error": write_error, "fdatasync": fdatasync, "fsync_file": fsync_file, "directory_fsync": dirsync, "directory_fsync_qualification_gap": bool(gap), "file_content_qualified": bool(ok)}


def read_qualifier(role: str, name: str) -> dict:
    if role not in ("a", "b"):
        raise DriverError("read is only valid for mounted a/b roles")
    require_owned_mount(role)
    data = payload()
    path = qualifier_path(role, name)
    fd = os.open(path, os.O_RDONLY)
    ranges = []
    try:
        for i in range(64):
            size = 4096 if i % 2 == 0 else 65536
            offset = ((i * 104729) % (len(data) - size + 1))
            block = os.pread(fd, size, offset)
            if block != data[offset : offset + size]:
                raise DriverError(f"range mismatch {i} offset={offset} size={size}")
            ranges.append({"index": i, "offset": offset, "size": size, "sha256": hashlib.sha256(block).hexdigest()})
        chunks = []
        while True:
            chunk = os.read(fd, 1024 * 1024)
            if not chunk:
                break
            chunks.append(chunk)
        full = b"".join(chunks)
    finally:
        os.close(fd)
    if full != data:
        raise DriverError("full read mismatch")
    return {"status": "PASS", "file": str(path), "bytes": len(data), "sha256": hashlib.sha256(data).hexdigest(), "fixed_ranges_exact": ranges}


def main() -> dict:
    parser = argparse.ArgumentParser()
    parser.add_argument("role", choices=("ctl", "a", "b", "c"))
    parser.add_argument("action", choices=("prepare", "start-control", "start-storage", "init-chain", "mount", "stop", "write", "read", "preflight"))
    parser.add_argument("--file", default="qualifier32m.bin")
    args = parser.parse_args()
    require_guest()
    if args.action != "stop":
        guard_volume(args.role)
    run = root(args.role)
    if args.action == "prepare":
        result = prepare(args.role)
    elif args.action == "preflight":
        result = {"status": "PASS", "preflight": preflight(args.role)}
    else:
        if not run.exists():
            raise DriverError(f"prepare first: {run}")
        prerequisites = None if args.action == "stop" else preflight(args.role)
        if args.action == "start-control":
            if args.role != "ctl":
                raise DriverError("start-control is ctl-only")
            result = start_control()
        elif args.action == "start-storage":
            result = start_storage(args.role)
        elif args.action == "init-chain":
            if args.role != "ctl":
                raise DriverError("init-chain is ctl-only")
            result = init_chain()
        elif args.action == "mount":
            result = mount(args.role)
        elif args.action == "stop":
            result = stop(args.role)
        elif args.action == "write":
            result = write_qualifier(args.role, args.file)
        elif args.action == "read":
            result = read_qualifier(args.role, args.file)
        else:
            raise DriverError(args.action)
        if prerequisites is not None:
            result.setdefault("prerequisites", prerequisites)
    label = args.action if args.action not in ("write", "read") else f"{args.action}-{args.file}"
    result.update(role=args.role, action=args.action, root=str(run))
    save_json(run, label, result)
    if result.get("status") != "PASS":
        raise SystemExit(1)
    return result


if __name__ == "__main__":
    try:
        print(json.dumps(main(), indent=2, sort_keys=True), flush=True)
    except Exception as error:
        failure_role = sys.argv[1] if len(sys.argv) > 1 and sys.argv[1] in VOLUMES else None
        if failure_role is not None:
            run_root = root(failure_role)
            try:
                ensure_dirs(run_root)
                save_json(run_root, f"FAIL-{int(time.time())}", {"status": "FAIL", "error": repr(error), "argv": sys.argv})
            except Exception:
                pass
        raise
