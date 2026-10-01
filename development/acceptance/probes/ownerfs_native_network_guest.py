#!/usr/bin/env python3
"""Trusted VM-side architecture probe. Not production lifecycle/READY wiring.

Every action is confined to a fresh run directory. Process signals require
matching executable digest, start tick and boot id. Actors retain real FUSE or
native directory/file references; control files never represent Agent input.
"""
import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import signal
import socket
import subprocess
import sys
import time
import urllib.request


def digest(path):
    with open(path, "rb") as handle:
        return hashlib.file_digest(handle, "sha256").hexdigest()


def save(path, value):
    temporary = path.with_name("." + path.name + ".tmp")
    temporary.write_text(json.dumps(value, indent=2) + "\n")
    temporary.replace(path)


def identity(pid):
    proc = Path(f"/proc/{pid}")
    stat = (proc / "stat").read_text()
    # comm may contain spaces/parentheses; starttime is field 22.
    fields = stat[stat.rfind(")") + 2:].split()
    return dict(pid=pid, start_tick=fields[19], boot_id=Path(
        "/proc/sys/kernel/random/boot_id").read_text().strip(),
        exe=os.readlink(proc / "exe"), sha256=digest(proc / "exe"),
        namespace=os.readlink(proc / "ns/mnt"),
        cmdline=(proc / "cmdline").read_bytes().replace(b"\0", b" ").decode())


def verify(record):
    current = identity(record["pid"])
    for field in ("pid", "start_tick", "boot_id", "exe", "sha256", "namespace"):
        if current[field] != record[field]:
            raise RuntimeError(f"process identity changed: {field}")
    return current


def wait_file(path, timeout=30):
    until = time.monotonic() + timeout
    while time.monotonic() < until:
        if path.is_file():
            return json.loads(path.read_text())
        time.sleep(.1)
    raise TimeoutError(str(path))


def role_config(base):
    return json.loads((base / "role.json").read_text())


def supervise(base, command, log, terminal):
    with open(base / log, "wb", buffering=0) as output:
        child = subprocess.Popen(command, stdin=subprocess.DEVNULL,
                                 stdout=output, stderr=subprocess.STDOUT)
        expected = os.path.realpath(shutil.which(command[0]) or command[0])
        until = time.monotonic() + 3
        while time.monotonic() < until:
            current = identity(child.pid)
            actor_exec = terminal.startswith("actor-") and current["exe"] == os.path.realpath(sys.executable) \
                and str(base / "guest.py") in current["cmdline"] and " actor " in current["cmdline"]
            if current["exe"] == expected or actor_exec:
                save(base / (terminal + "-child.json"), current)
                break
            time.sleep(.01)
        else:
            raise RuntimeError("child did not exec expected command")
        code = child.wait()
    save(base / (terminal + "-exit.json"), {"exit": code})


def launch(base):
    cfg = role_config(base)
    for name, expected in cfg["inputs"].items():
        assert digest(base / name) == expected, f"input digest mismatch: {name}"
    assert subprocess.check_output(["findmnt", "-T", str(base), "-n", "-o", "FSTYPE"],
                                   text=True).strip() == "ext4"
    for port in cfg["ports"]:
        with socket.socket() as check:
            check.bind(("0.0.0.0", port))
    assert not (base / "supervisor.json").exists(), "fresh run only"
    (base / "parent-mountinfo-before.txt").write_text(Path("/proc/self/mountinfo").read_text())
    (base / "control").mkdir(mode=0o700)
    (base / "mount").mkdir()
    (base / "data").mkdir()
    for cert in ("ca", cfg["cert"], "node-a", "node-b"):
        path = Path(cfg["tls"]) / (cert + ".pem")
        result = subprocess.run(["openssl", "x509", "-in", str(path), "-noout",
                                 "-fingerprint", "-sha256", "-dates"],
                                check=True, capture_output=True, text=True)
        (base / (cert + "-public-certificate.txt")).write_text(result.stdout)
    command = [sys.executable, str(base / "guest.py"), str(base), "supervise"]
    supervisor = subprocess.Popen(command, stdin=subprocess.DEVNULL,
                                  stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                                  start_new_session=True)
    save(base / "supervisor.json", identity(supervisor.pid))
    return {"launched": cfg["role"], "supervisor": supervisor.pid}


def node_ready(base):
    cfg = role_config(base)
    health = urllib.request.urlopen(f"http://127.0.0.1:{cfg['ports'][1]}/health", timeout=3).read()
    if cfg["role"] == "ctl":
        actual = wait_file(base / "node-child.json", 1)
    else:
        driver = wait_file(base / "control/driver.json", 1)
        actual = identity(driver["pid"])
        assert actual["namespace"] == driver["namespace"]
        assert actual["namespace"] != os.readlink("/proc/self/ns/mnt"), "private namespace required"
        mounts = Path(f"/proc/{actual['pid']}/mountinfo").read_text()
        assert any(str(base / "mount") == line.split()[4] and " - fuse" in line
                   for line in mounts.splitlines()), "OwnerFs not mounted"
        (base / "ready-mountinfo.txt").write_text(mounts)
    verify(actual)
    expected = cfg["inputs"]["afs-meta" if cfg["role"] == "ctl" else "node-tests"]
    assert actual["sha256"] == expected
    save(base / "node-identity.json", actual)
    (base / "health.json").write_bytes(health)
    return actual


def node(base):
    record = json.loads((base / "node-identity.json").read_text())
    verify(record)
    return record


def in_namespace(base, arguments):
    current = node(base)
    return subprocess.run(["nsenter", "--target", str(current["pid"]), "--mount", "--"]
                          + arguments, check=True, capture_output=True, text=True)


def driver_command(base, command):
    node(base)
    save(base / "control/request.json", command)
    result = wait_file(base / f"control/reply-{command['id']}.json")
    assert result["id"] == command["id"]
    return result


def actor_start(base, actor_name):
    assert re.fullmatch(r"[a-z]+", actor_name)
    directory = base / ("actor-" + actor_name)
    directory.mkdir(mode=0o700)
    current = node(base)
    supervisor = subprocess.Popen([sys.executable, str(base / "guest.py"), str(base),
                                   "actor-supervise", actor_name],
                                  stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                                  stderr=subprocess.DEVNULL, start_new_session=True)
    save(directory / "supervisor.json", identity(supervisor.pid))
    # Actor-supervise constructs the same command after rechecking Node identity.
    result = wait_file(directory / "ready.json")
    verify(result["process"])
    assert result["process"]["namespace"] == current["namespace"]
    return result


def actor_command(base, command, wait=True):
    directory = base / ("actor-" + command.pop("actor"))
    ready = json.loads((directory / "ready.json").read_text())
    verify(ready["process"])
    save(directory / "request.json", command)
    if not wait:
        return {"id": command["id"], "submitted": True}
    result = wait_file(directory / f"reply-{command['id']}.json")
    assert result["id"] == command["id"]
    return result


def actor(base, actor_name):
    directory = base / ("actor-" + actor_name)
    root = os.open(base / "mount/agent1", os.O_RDONLY | os.O_DIRECTORY)
    handles = {}
    initial = os.fstat(root)
    save(directory / "ready.json", {"process": identity(os.getpid()),
         "root": {"device": initial.st_dev, "inode": initial.st_ino}})
    deadline = time.monotonic() + 300
    try:
        while time.monotonic() < deadline:
            request = directory / "request.json"
            if not request.exists():
                time.sleep(.02)
                continue
            command = json.loads(request.read_text())
            request.unlink()
            operation = command["operation"]
            name = command.get("name")
            if name is not None:
                assert re.fullmatch(r"[a-zA-Z0-9_.-]+", name) and name not in (".", "..")
            try:
                value = None
                key = command.get("handle")
                if operation == "open":
                    requested = command.get("flags", "O_RDONLY")
                    requested = requested if isinstance(requested, list) else [requested]
                    flags = 0
                    for flag in requested:
                        assert flag in ("O_RDONLY", "O_RDWR", "O_WRONLY", "O_APPEND")
                        flags |= getattr(os, flag)
                    if command.get("create"):
                        flags |= os.O_CREAT | os.O_EXCL
                    handles[key] = os.open(name, flags, 0o600, dir_fd=root)
                    stat = os.fstat(handles[key])
                    value = {"device": stat.st_dev, "inode": stat.st_ino}
                elif operation == "read":
                    value = os.pread(handles[key], 65536, 0).decode()
                elif operation == "flock":
                    mode = command["mode"]
                    assert mode in ("EX", "SH", "UN")
                    # Bounded probes only: no actor/controller can hang on a lock.
                    fcntl.flock(handles[key], getattr(fcntl, "LOCK_" + mode) | fcntl.LOCK_NB)
                elif operation == "append-series":
                    count, repeat = command["count"], command["repeat"]
                    delay = command.get("delay", 0)
                    assert 1 <= count <= 1000 and 1 <= repeat <= 2 * 1024 * 1024
                    assert 0 <= delay <= .01 and command["byte"] in ("N", "R")
                    fd = handles[key]
                    assert fcntl.fcntl(fd, fcntl.F_GETFL) & os.O_APPEND
                    os.lseek(fd, 0, os.SEEK_SET)
                    data = command["byte"].encode() * repeat
                    positions = []
                    started = time.monotonic_ns()
                    for index in range(count):
                        assert os.write(fd, data) == len(data), "short write is a distinct outcome"
                        positions.append(os.lseek(fd, 0, os.SEEK_CUR))
                        if index == 0:
                            save(directory / f"progress-{command['id']}.json", {"id": command["id"], "first_write": True})
                        if delay:
                            time.sleep(delay)
                    value = {"writes": count, "bytes_each": len(data), "positions": positions,
                             "started_ns": started, "finished_ns": time.monotonic_ns()}
                elif operation == "inspect-append":
                    fd = os.open(name, os.O_RDONLY, dir_fd=root)
                    try:
                        size = os.fstat(fd).st_size
                        assert size <= 4 * 1024 * 1024
                        data = os.pread(fd, size, 0)
                        assert len(data) == size
                    finally:
                        os.close(fd)
                    (directory / f"content-{name}.bin").write_bytes(data)
                    segments = []
                    for offset, byte in enumerate(data):
                        assert byte in (78, 82)
                        if not segments or segments[-1][0] != chr(byte):
                            segments.append([chr(byte), offset, 1])
                        else:
                            segments[-1][2] += 1
                    value = {"size": size, "sha256": hashlib.sha256(data).hexdigest(), "segments": segments}
                elif operation == "write":
                    fd = handles[key] if key is not None else os.open(
                        name, os.O_WRONLY | os.O_CREAT, 0o600, dir_fd=root)
                    try:
                        data = command["data"].encode()
                        assert os.pwrite(fd, data, 0) == len(data)
                        if command.get("truncate", True):
                            os.ftruncate(fd, len(data))
                        if command.get("fsync"):
                            os.fsync(fd)
                        value = len(data)
                    finally:
                        if key is None:
                            os.close(fd)
                elif operation == "close":
                    os.close(handles.pop(key))
                elif operation == "rename":
                    target = command["target"]
                    assert re.fullmatch(r"[a-zA-Z0-9_.-]+", target)
                    os.rename(name, target, src_dir_fd=root, dst_dir_fd=root)
                elif operation == "unlink":
                    os.unlink(name, dir_fd=root)
                elif operation == "stat":
                    stat = os.stat(name, dir_fd=root)
                    value = {"size": stat.st_size, "mode": stat.st_mode & 0o777,
                             "device": stat.st_dev, "inode": stat.st_ino}
                elif operation == "quit":
                    assert not handles, "close retained handles explicitly"
                else:
                    raise ValueError("unknown actor operation")
                result = {"id": command["id"], "ok": True, "result": value}
            except Exception as error:
                result = {"id": command["id"], "ok": False, "error": repr(error),
                          "errno": getattr(error, "errno", None)}
            save(directory / f"reply-{command['id']}.json", result)
            if operation == "quit" and result["ok"]:
                return
        raise TimeoutError("actor lifetime exceeded")
    finally:
        for fd in handles.values():
            os.close(fd)
        os.close(root)


def stop(base):
    record_path = base / "node-identity.json"
    if record_path.exists():
        record = json.loads(record_path.read_text())
    elif (base / "control/driver.json").exists():
        record = identity(json.loads((base / "control/driver.json").read_text())["pid"])
        assert record["sha256"] == role_config(base)["inputs"]["node-tests"]
        save(base / "cleanup-node-identity.json", record)
    else:
        record = wait_file(base / "node-child.json", 1)
        assert role_config(base)["role"] == "ctl", "Node not identified; preserve for inspection"
    if not (base / "node-exit.json").exists():
        verify(record)
        os.kill(record["pid"], signal.SIGTERM)
    result = wait_file(base / "node-exit.json", 25)
    (base / "parent-mountinfo-after.txt").write_text(Path("/proc/self/mountinfo").read_text())
    assert (base / "parent-mountinfo-before.txt").read_text() == (
        base / "parent-mountinfo-after.txt").read_text(), "parent mounts changed"
    return result


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("base", type=Path)
    parser.add_argument("operation")
    parser.add_argument("extra", nargs="?")
    args = parser.parse_args()
    base = args.base
    assert os.getuid() == 0
    assert re.fullmatch(r"network-probe-\d{8}T\d{6}-[a-f0-9]{8}", base.name)
    assert str(base.parent) in ("/mnt/afsdata/ownerfs-native-network",
                              "/mnt/afsstate/ownerfs-native-network")
    op = args.operation
    if op == "launch":
        result = launch(base)
    elif op == "supervise":
        cfg = role_config(base)
        if cfg["role"] == "ctl":
            command = [str(base / "afs-meta"), "--config", str(base / "config.toml")]
        else:
            command = ["unshare", "--mount", "--propagation", "private", "--fork", "env",
                       "AFS_NATIVE_PRIVATE_NAMESPACE=1",
                       f"AFS_NATIVE_VALIDATION_CONFIG={base}/config.toml",
                       f"AFS_NATIVE_VALIDATION_CONTROL={base}/control",
                       str(base / "node-tests"), "--ignored", "--exact",
                       "node::native_validation::privileged_native_validation_node",
                       "--nocapture", "--test-threads=1"]
        supervise(base, command, "node.log", "node")
        return
    elif op == "ready":
        result = node_ready(base)
    elif op == "driver":
        result = driver_command(base, json.load(sys.stdin))
    elif op == "seed":
        result = in_namespace(base, [sys.executable, "-c",
            "from pathlib import Path; p=Path(__import__('sys').argv[1]); "
            "p.mkdir(); (p/'data').write_text('original-A-data'); "
            "(p/'identity').write_text('original-object')", str(base / "mount/agent1")]).stdout
    elif op == "actor-start":
        result = actor_start(base, args.extra)
    elif op == "actor-command":
        result = actor_command(base, json.load(sys.stdin))
    elif op == "actor-submit":
        result = actor_command(base, json.load(sys.stdin), wait=False)
    elif op in ("actor-progress", "actor-result"):
        command = json.load(sys.stdin)
        assert re.fullmatch(r"c[0-9]+", command["id"])
        assert re.fullmatch(r"[a-zA-Z0-9_-]+", command["actor"])
        prefix = "progress" if op == "actor-progress" else "reply"
        result = wait_file(base / ("actor-" + command["actor"]) / f"{prefix}-{command['id']}.json")
    elif op in ("actor-wait", "actor-stop"):
        directory = base / ("actor-" + args.extra)
        if op == "actor-stop" and not (directory / "actor-exit.json").exists():
            record = json.loads((directory / "ready.json").read_text())["process"]
            verify(record)
            os.kill(record["pid"], signal.SIGTERM)
        result = wait_file(directory / "actor-exit.json", 10)
    elif op == "actor-supervise":
        current = node(base)
        command = ["nsenter", "--target", str(current["pid"]), "--mount", "--",
                   sys.executable, str(base / "guest.py"), str(base), "actor", args.extra]
        supervise(base, command, f"actor-{args.extra}/actor.log", f"actor-{args.extra}/actor")
        return
    elif op == "actor":
        actor(base, args.extra)
        return
    elif op == "capture":
        current = node(base)
        (base / "final-mountinfo.txt").write_text(Path(f"/proc/{current['pid']}/mountinfo").read_text())
        cfg = role_config(base)
        data = urllib.request.urlopen(f"http://127.0.0.1:{cfg['ports'][1]}/metrics", timeout=3).read()
        (base / "metrics.txt").write_bytes(data)
        sockets = []
        for fd in Path(f"/proc/{current['pid']}/fd").iterdir():
            try:
                target = os.readlink(fd)
                if target.startswith("socket:["):
                    sockets.append(target[8:-1])
            except FileNotFoundError:
                pass
        connections = {}
        for protocol in ("tcp", "tcp6"):
            lines = Path(f"/proc/{current['pid']}/net/{protocol}").read_text().splitlines()
            connections[protocol] = [line for line in lines[1:] if line.split()[9] in sockets]
        save(base / "owned-tcp.json", connections)
        result = {"metrics_sha256": digest(base / "metrics.txt")}
    elif op == "stop":
        result = stop(base)
    elif op == "archive":
        assert (base / "node-exit.json").exists(), "stop before freezing evidence"
        paths = sorted(path for path in base.rglob("*") if path.is_file()
                       and "data" not in path.relative_to(base).parts
                       and path.name not in ("node-tests", "afs-meta", "raw.tar.gz", "evidence-files.sha256"))
        (base / "evidence-files.sha256").write_text("".join(
            digest(path) + "  " + str(path.relative_to(base)) + "\n" for path in paths))
        archive = base.with_suffix(".raw.tar.gz")
        assert not archive.exists(), "do not replace prior evidence"
        subprocess.run(["tar", "-czf", str(archive), "--exclude=raw.tar.gz",
                        "--exclude=node-tests", "--exclude=afs-meta", "--exclude=data",
                        "-C", str(base), "."], check=True)
        import pwd
        user = pwd.getpwnam("lzc")
        os.chown(archive, user.pw_uid, user.pw_gid)
        result = {"path": str(archive), "sha256": digest(archive)}
    else:
        raise ValueError(op)
    print(json.dumps(result))


if __name__ == "__main__":
    main()
