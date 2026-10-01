#!/usr/bin/env python3
"""Linux-only current-package healthy-flow probe; not formal acceptance."""
import argparse
import hashlib
import json
import os
import pathlib
import platform
import subprocess
import tarfile
import tomllib
import urllib.request

assert platform.system() == "Linux", "Linux execution required"
VOLUMES = {"ctl": "/mnt/lima-afsctlstate", "a": "/mnt/lima-afsadata", "b": "/mnt/lima-afsbdata", "c": "/mnt/lima-afscdata"}
IPS = {"ctl": "192.168.109.11", "a": "192.168.109.12", "b": "192.168.109.13", "c": "192.168.109.14"}
PORTS = {"ctl": 19980, "a": 19982, "b": 19984, "c": 19986}
SHA = {"afs-node": "d28d17faa8bb71eb8c0a28241e8556396e440b204cf0242d2af8133700f6a494", "afs-meta": "64d85fc3933637ccd7fdf4eafded5e5c8cb2354e73b4c8d1a27b2b8d43f4c7e7"}

def command(args):
    return subprocess.check_output(args, text=True, timeout=90)

def digest(path):
    return hashlib.sha256(pathlib.Path(path).read_bytes()).hexdigest()

def runpath(which):
    suffix = "" if args.cohort == "r1" else "-" + args.cohort
    return pathlib.Path(VOLUMES[which]) / ("afs-delivery/round1-mainline-v77" + suffix)

def web(path):
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    with opener.open("http://192.168.109.11:19981" + path, timeout=15) as response:
        return json.load(response)

def prepare(which):
    run = runpath(which)
    assert not run.exists(), "Never overwrite a runtime"
    assert platform.machine() == "aarch64"
    volume = json.loads(command(["findmnt", "-J", "-T", VOLUMES[which], "-o", "TARGET,SOURCE,FSTYPE"]))["filesystems"][0]
    assert volume["fstype"] == "ext4"
    disk = os.statvfs(VOLUMES[which])
    available = disk.f_bavail * disk.f_frsize
    assert available >= 4 * 1024**3, available
    assert pathlib.Path("/dev/fuse").exists()
    assert "link rxe0/1 state ACTIVE" in command(["rdma", "link", "show"])
    listeners = command(["ss", "-ltnH"])
    for row in listeners.splitlines():
        assert not any(row.split()[3].endswith(":" + str(p)) for p in (PORTS[which], PORTS[which] + 1)), row
    run.mkdir(parents=True)
    uid, gid = int(os.environ["SUDO_UID"]), int(os.environ["SUDO_GID"])
    os.chown(run, uid, gid)
    (run / "evidence").mkdir()
    os.chown(run / "evidence", uid, gid)
    before = {"boot_id": pathlib.Path("/proc/sys/kernel/random/boot_id").read_text().strip(), "volume": volume, "available_bytes": available, "listeners": listeners, "mountinfo": pathlib.Path("/proc/self/mountinfo").read_text(), "rdma": command(["rdma", "resource", "show"]), "kernel": platform.release(), "cpu": os.cpu_count()}
    (run / "evidence/preflight.json").write_text(json.dumps(before, indent=2))
    corrected = args.cohort.startswith("archive-")
    package = pathlib.Path("/home/lzc.guest/afs-round1-v77-corrected-package.tar.gz" if corrected else "/home/lzc.guest/afs-round1-v77-package.tar.gz")
    expected = "e13bc12e2d51ea70287a935c4f2a92b6b5313979ac94a2b080e0ec89fbaac8b5" if corrected else "ccd21dcbe1cd0a153a13f06e84e730fc1f44811e973069fe2caffd759a50dad3"
    assert digest(package) == expected
    with tarfile.open(package) as archive:
        archive.extractall(run / "package", filter="data")
    extracted = run / "package/afs-0.1.0-linux-aarch64"
    manifest = json.loads((extracted / "manifest.json").read_text())
    assert {k: v["sha256"] for k, v in manifest["binaries"].items()} == SHA
    if corrected:
        assert manifest["source_commit"] == "370cba91a15999a7edd7edb644a852c2b2dfd89b"
        assert digest(extracted / "bin/afs-processctl") == "01f38d32f34994f89bea3c75f57eecc506db6157309e74f3fe5425f64775601e"
    install_command = ["bash", str(extracted / "install.sh")]
    for option, directory in (("prefix", "prefix"), ("config-dir", "etc"), ("state-dir", "state"), ("run-dir", "run"), ("log-dir", "log"), ("mount-root", "mount")):
        install_command.extend(["--" + option, str(run / directory)])
    output = command(install_command)
    (run / "evidence/install.log").write_text(output)
    for name, value in SHA.items():
        assert digest(run / "prefix/bin" / name) == value
    if corrected:
        assert digest(run / "prefix/bin/afs-processctl") == digest(extracted / "bin/afs-processctl")
    result = {"level": "mainline preparation", "installed": which, "package_sha256": digest(package), "manifest": manifest, "available_bytes_before": available, "command": install_command, "formal_acceptance": "NOT_RUN"}
    (run / "evidence/install.json").write_text(json.dumps(result, indent=2))
    return result

def configure(which):
    run = runpath(which)
    tls = run / "etc/tls"
    trusted = "{ " + ", ".join(f'"round1-{n}" = "{tls}/round1-{n}.pem"' for n in "abc") + " }"
    name = "meta" if which == "ctl" else "round1-" + which
    copies = 1 if args.cohort in ("r1", "archive-r1") else 2
    required = 1 if args.cohort == "archive-async" else copies
    common = f'''fs = "all"
grpc_listen = "0.0.0.0:{PORTS[which]}"
rest_listen = "{IPS[which]}:{PORTS[which] + 1}"
log_level = "info"
tls_ca_certificate = "{tls}/ca.pem"
tls_identity_certificate = "{tls}/{name}.pem"
tls_identity_private_key = "{tls}/{name}-key.pem"
tls_server_name = "afs-cluster"
trusted_node_certs = {trusted}
dfs_desired_copies = {copies}
dfs_sync_required_copies = {required}
dfs_min_distinct_nodes = {required}
dfs_min_distinct_failure_domains = 1
dfs_local_copy = "required"
'''
    for filename in ("ca.pem", "meta.pem", "round1-a.pem", "round1-b.pem", "round1-c.pem", name + "-key.pem"):
        assert (tls / filename).is_file(), filename
    if which == "ctl":
        text = common + f'id = "round1-meta"\nmeta_store = "memory"\ndata_dir = "{run}/state/meta"\n'
        role = "meta"
    else:
        text = common + f'''id = "round1-{which}"
meta_endpoint = "https://192.168.109.11:19980"
advertise_endpoint = "https://{IPS[which]}:{PORTS[which]}"
data_dir = "{run}/state/node"
uds_path = "{run}/run/node.sock"
ownerfs_mount = "{run}/mount/ownerfs"
dfs_mount = "{run}/mount/dfs"
data_mode = "auto"
rdma_device = "rxe0"
timeout_ms = 10000
'''
        role = "node"
    (run / "etc" / (role + ".toml")).write_text(text)
    assert tomllib.loads(text)["dfs_sync_required_copies"] == required
    return {"configured": which, "role": role, "config_sha256": digest(run / "etc" / (role + ".toml"))}

def replicas():
    assert args.cohort in ("rn", "archive-async")
    chunks = ("blake3-4e94e6f582581a0f3855f3ce504b153e951e65036fe9e2f010b7e25473c54f98-4194304", "blake3-63c31766464b0c4931ff8b7406a2c1d8140d08b94328ccd7cf3b431d94cc690f-17")
    results = [web("/v1/dfs/chunks/" + chunk + "/replication") for chunk in chunks]
    for result in results:
        assert result["placement"]["desired_copies"] == 2, result
        assert result["available_copies"] >= 2 and result["health"] == "Satisfied", result
        ready = [c["record"] for c in result["copies"] if c["available"]]
        assert len({c["location"]["Node"]["node_id"] for c in ready}) >= 2
        assert all(c["role"] == "DurableReplica" and c["state"] == "Ready" for c in ready)
    return {"level": "healthy replica qualification", "chunks": results, "formal_acceptance": "NOT_RUN"}

def files(which, action, kind, name):
    assert which != "ctl"
    run = runpath(which)
    mount = run / "mount" / kind
    entry = json.loads(command(["findmnt", "-J", "-M", str(mount), "-o", "TARGET,SOURCE,FSTYPE"]))["filesystems"][0]
    assert entry["source"] == "afs-" + kind and entry["fstype"].startswith("fuse")
    workspace = "workspace-round1-v77"
    if kind == "ownerfs":
        if action == "create":
            assert which == "a"
            os.mkdir(mount / workspace)
            root = web("/v1/roots/root-" + workspace.encode().hex())
            assert root["home_node_id"] == "round1-a" and root["home_serving"], root
            return {"created": workspace, "root": root}
        path = mount / workspace / name
    else:
        path = mount / name
    size = 4 * 1024 * 1024 + 17
    payload = (bytes(range(251)) * (size // 251 + 1))[:size]
    if action == "write":
        fd = os.open(path, os.O_CREAT | os.O_EXCL | os.O_RDWR, 0o600)
        try:
            position = 0
            while position < size:
                count = os.write(fd, payload[position:position + 1024 * 1024])
                assert count > 0
                position += count
            os.fdatasync(fd)
            os.fsync(fd)
        finally:
            os.close(fd)
    elif action == "read":
        with path.open("rb") as stream:
            value = stream.read(size + 1)
            assert value == payload and stream.read(1) == b"", (len(value), digest(path))
    else:
        raise ValueError(action)
    assert path.stat().st_size == size
    return {"action": action, "which": which, "kind": kind, "name": name, "bytes": size, "sha256": hashlib.sha256(payload).hexdigest(), "actual_sha256": digest(path), "inode": path.stat().st_ino, "level": "healthy mainline flow", "formal_acceptance": "NOT_RUN"}

parser = argparse.ArgumentParser()
parser.add_argument("which", choices=VOLUMES)
parser.add_argument("action", choices=("prepare", "configure", "create", "write", "read", "replicas"))
parser.add_argument("--kind", choices=("ownerfs", "dfs"), default="ownerfs")
parser.add_argument("--name", choices=("data.bin", "remote.bin"), default="data.bin")
parser.add_argument("--cohort", choices=("r1", "rn", "archive-r1", "archive-async"), default="r1")
args = parser.parse_args()
result = prepare(args.which) if args.action == "prepare" else configure(args.which) if args.action == "configure" else replicas() if args.action == "replicas" else files(args.which, args.action, args.kind, args.name)
print(json.dumps(result, indent=2))
