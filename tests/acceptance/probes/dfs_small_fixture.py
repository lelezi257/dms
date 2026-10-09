#!/usr/bin/env python3
"""Fixed 64MiB R2 diagnostic config/admission; no service lifecycle or benchmark.

prepare changes only the new fixture. preflight emits evidence on stdout and
does not write files. The published afs-processctl owns start/wait/stop.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import re
import shutil
import subprocess
import sys
import time
import tomllib
from pathlib import Path

FIXTURE = "dfs-small-6d-20261007-r1"
SYNC_FIXTURE = "dfs-sync-6d-20261007-r1"
DELETE_FIXTURE = "dfs-delete-6d-20261007-r1"
FIXTURE_PORTS = {FIXTURE: {"ctl": (23100, 23101), "node": (23200, 23201)},
                 SYNC_FIXTURE: {"ctl": (23700, 23701), "node": (23800, 23801)},
                 DELETE_FIXTURE: {"ctl": (23900, 23901), "node": (24000, 24001)}}
VOLUMES = {"ctl": "/mnt/lima-afsctlstate", "a": "/mnt/lima-afsadata",
           "b": "/mnt/lima-afsbdata", "c": "/mnt/lima-afscdata"}
IPS = {"ctl": "192.168.109.11", "a": "192.168.109.12",
       "b": "192.168.109.13", "c": "192.168.109.14"}
NODES = {r: "dfs-" + r + "-r1" for r in ("a", "b", "c")}
SHA = {"meta": "2c7b7d088b759e3b9375080002182aa484a424b4fa216da1fb79a1004e96168e",
       "node": "2cf1f538fe7af332a711f3c66a074ace140c00773826a182709a6445b8ae2645",
       "io": "70ac97c7634d406a177a74c783d446b62a2014ba198586132882a1d9228e55e8",
       "afs-processctl": "1ea10552180c917da2c598bee9dd8fecee7be26ae9d70dcdcba2e4829150d896",
       "afs-trial-config": "c47783f9525826d4a0e137499bb1a779f94b3474c7355dd53aec0e615b98889b"}
R2 = {"dfs_desired_copies": 2, "dfs_sync_required_copies": 2,
      "dfs_min_distinct_nodes": 2, "dfs_min_distinct_failure_domains": 2,
      "dfs_local_copy": "required"}


def require(ok, message):
    if not ok:
        raise RuntimeError(message)


def digest(path):
    h = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for block in iter(lambda: stream.read(1024**2), b""):
            h.update(block)
    return h.hexdigest()


def safe(path, boundary, exists=True):
    path, boundary = Path(path), Path(boundary)
    require(path.is_absolute() and path.is_relative_to(boundary), f"path escape: {path}")
    require(".." not in path.parts, f"path traversal: {path}")
    # Check all ancestors, including the volume and /mnt, before writing.
    for part in (path, *path.parents):
        require(not part.is_symlink(), f"symlink component: {part}")
    if exists:
        require(path.exists(), f"missing path: {path}")
    return path


class Fixture:
    def __init__(self, role, fixture=FIXTURE):
        require(fixture in FIXTURE_PORTS, "unknown named fixture")
        self.fixture = fixture
        self.role = role
        self.volume = Path(VOLUMES[role])
        self.root = self.volume / "afs-delivery" / self.fixture
        self.name = "meta" if role == "ctl" else "node"
        self.config = self.root / "etc" / (self.name + ".toml")
        self.binary = self.root / "prefix/bin" / ("afs-" + self.name)
        self.commands = []

    def guest(self):
        require(platform.system() == "Linux" and platform.machine() == "aarch64", "Linux ARM64 guest only")
        require(os.geteuid() == 0, "guest root required")
        require(platform.node() == "lima-afs-accept-" + self.role, "wrong guest role")
        safe(self.root, self.volume)
        require(self.root.stat().st_uid == 0, "fixture root must be root-owned")

    def run(self, argv, allowed=(0,)):
        result = subprocess.run(list(map(str, argv)), capture_output=True, text=True, timeout=20)
        record = {"argv": result.args, "returncode": result.returncode,
                  "stdout": result.stdout, "stderr": result.stderr}
        self.commands.append(record)
        require(result.returncode in allowed, f"command failed: {record}")
        return result.stdout

    def ports(self):
        return FIXTURE_PORTS[self.fixture]["ctl" if self.role == "ctl" else "node"]

    def expected(self):
        tls = self.root / "etc/tls"
        identity = "meta" if self.role == "ctl" else NODES[self.role]
        values = {"id": "meta-ctl" if self.role == "ctl" else identity,
                  "fs": "dfs", "experimental_native_workspace": False,
                  "data_dir": str(self.root / "state" / self.name),
                  "grpc_listen": "0.0.0.0:" + str(self.ports()[0]),
                  "rest_listen": "0.0.0.0:" + str(self.ports()[1]),
                  "log_level": "info", "trace_enabled": False,
                  "tls_ca_certificate": str(tls / "ca.pem"),
                  "tls_identity_certificate": str(tls / (identity + ".pem")),
                  "tls_identity_private_key": str(tls / (identity + "-key.pem")),
                  "tls_server_name": "afs-meta",
                  "trusted_node_certs": {node: str(tls / (node + ".pem")) for node in NODES.values()}, **R2}
        if self.role == "ctl":
            values["meta_store"] = "local-file"
        else:
            values.update(meta_endpoint=f"https://{IPS['ctl']}:{FIXTURE_PORTS[self.fixture]['ctl'][0]}", data_mode="grpc",
                          allow_volatile_meta=False, advertise_endpoint=f"https://{IPS[self.role]}:{self.ports()[0]}",
                          uds_path=str(self.root / "run/node.sock"), dfs_mount=str(self.root / "mount/dfs"))
        return values

    def patch(self, original):
        require(not re.search(r"^\s*\[", original, re.M), "expected flat trial-config TOML")
        cfg = tomllib.loads(original)
        expected = self.expected()
        require(cfg.get("id") == expected["id"] and cfg.get("fs") == "all", "not fresh generated role config")
        for key in ("grpc_listen", "rest_listen", *(() if self.role == "ctl" else ("meta_endpoint", "advertise_endpoint"))):
            require(cfg.get(key) == expected[key], "generated fixture endpoint differs: " + key)
        require(all(cfg.get(k) == v for k, v in R2.items()), "generated R2 policy differs")
        require(set(cfg.get("trusted_node_certs", {})) == set(NODES.values()), "generated trust set differs")
        for node, path in cfg["trusted_node_certs"].items():
            require(Path(path).name == node + ".pem", "generated trust identity mapping differs")
        identity = "meta" if self.role == "ctl" else NODES[self.role]
        for key, basename in (("tls_ca_certificate", "ca.pem"),
                              ("tls_identity_certificate", identity + ".pem"),
                              ("tls_identity_private_key", identity + "-key.pem")):
            require(Path(cfg.get(key, "")).name == basename, "generated TLS identity differs")
        if self.role == "ctl":
            require(cfg.get("meta_store") == "local-file", "local-file Meta required")
        # Fail closed on unfamiliar fields rather than preserve an external path
        # or a new native setting without reviewing the fixed flat generator.
        allowed = set(expected) | {"ownerfs_mount", "dfs_mount"}
        require(not (set(cfg) - allowed), f"unexpected generated keys: {set(cfg) - allowed}")
        return "\n".join(k + " = " + self.toml(v) for k, v in expected.items()) + "\n"

    @staticmethod
    def toml(value):
        if isinstance(value, dict):
            return "{ " + ", ".join(k + " = " + json.dumps(v) for k, v in value.items()) + " }"
        return json.dumps(value)

    def validate_config(self):
        cfg = tomllib.loads(safe(self.config, self.root).read_text())
        require(cfg == self.expected(), "prepared config differs from frozen DFS-only contract")
        for key in ("data_dir", "dfs_mount", "uds_path", "tls_ca_certificate",
                    "tls_identity_certificate", "tls_identity_private_key"):
            if key in cfg:
                safe(cfg[key], self.root, exists=False)
        for path in cfg["trusted_node_certs"].values():
            safe(path, self.root)
        return cfg

    def capacity(self):
        rows = json.loads(self.run(["findmnt", "-J", "--mountpoint", self.volume]))["filesystems"]
        require(len(rows) == 1 and rows[0]["target"] == str(self.volume) and rows[0]["fstype"] == "ext4", "dedicated ext4 volume required")
        require(self.root.stat().st_dev == self.volume.stat().st_dev, "fixture escapes data volume")
        used = 0
        for directory, dirs, files in os.walk(self.root, followlinks=False):
            for path in [Path(directory), *(Path(directory) / n for n in files + dirs)]:
                safe(path, self.root)
                require(path.stat().st_dev == self.volume.stat().st_dev, "nested mount/device escape")
            used += Path(directory).stat().st_blocks * 512
            used += sum((Path(directory) / n).stat().st_blocks * 512 for n in files)
        info = os.statvfs(self.volume)
        available = info.f_bavail * info.f_frsize
        budget = (256 if self.role == "ctl" else 512) * 1024**2
        floor = (512 if self.role == "ctl" else 1024) * 1024**2
        require(used <= budget and available >= floor and available + used >= floor + budget,
                "case working budget/free floor not satisfied; preserve state")
        return {"volume": rows[0], "available_bytes": available, "allocated_bytes": used,
                "budget_bytes": budget, "free_floor_bytes": floor,
                "scope": "single 64MiB file, R2 local storage/dirty view, binaries and bounded logs"}

    def idle(self):
        mounts = json.loads(self.run(["findmnt", "-J", "-o", "TARGET,SOURCE,FSTYPE,OPTIONS,ID"]))["filesystems"]
        def walk(rows):
            for row in rows:
                yield row
                yield from walk(row.get("children", []))
        for row in walk(mounts):
            require(not Path(row["target"]).is_relative_to(self.root), "new fixture mount already active")
        ports = list(self.ports())
        sockets = self.run(["ss", "-ltnp"])
        for port in ports:
            require(not re.search(rf":{port}\s", sockets), f"new port occupied: {port}")
        boot = Path("/proc/sys/kernel/random/boot_id").read_text().strip()
        old = []
        for path in Path("/proc").iterdir():
            if not path.name.isdigit():
                continue
            selected = False
            try:
                argv = (path / "cmdline").read_bytes().split(b"\0")
                executable = os.readlink(path / "exe")
                if Path(executable).name not in ("afs-meta", "afs-node", "redis-server", "mfsmaster", "mfschunkserver", "mfsmount"):
                    continue
                selected = True
                require(not any(str(self.root).encode() in arg for arg in argv), "new fixture process already active")
                before = (path / "stat").read_text().rsplit(") ", 1)[1].split()[19]
                identity = {"pid": int(path.name), "boot_id": boot, "start_ticks": before,
                            "exe": executable, "exe_sha256": digest(path / "exe"),
                            "argv": [a.decode(errors="replace") for a in argv if a]}
                after = (path / "stat").read_text().rsplit(") ", 1)[1].split()[19]
                require(before == after and executable == os.readlink(path / "exe"), "old process changed during inventory")
                old.append(identity)
            except (FileNotFoundError, ProcessLookupError):
                if selected:
                    raise RuntimeError("selected process inventory changed; preserve evidence")
        return {"boot_id": boot, "old_processes": old, "mount_inventory": mounts, "selected_ports": ports}

    def prepare(self):
        marker = safe(self.root / "run/config-prepared.json", self.root, exists=False)
        backup = self.config.with_name(self.name + ".original.toml")
        require(not marker.exists() and not backup.exists(), "prepare is exclusive; use preflight")
        original = safe(self.config, self.root).read_text()
        text = self.patch(original)
        inventory = self.idle()
        capacity = self.capacity()
        for relative in ("run", "logs", "results", "state", "state/" + self.name, "mount", "mount/dfs"):
            safe(self.root / relative, self.root, exists=False).mkdir(exist_ok=True)
        with safe(backup, self.root, exists=False).open("x") as stream:
            stream.write(original)
        # Existing new config is the only overwritten file; original stays intact.
        self.config.write_text(text)
        result = {"role": self.role, "fixture": self.fixture, "root": str(self.root), "config_sha256": digest(self.config),
                  "original_sha256": digest(backup), "capacity": capacity, "inventory": inventory}
        with marker.open("x") as stream:
            json.dump(result, stream, indent=2)
            stream.write("\n")
        return {"status": "PREPARED_NOT_ADMITTED", **result}

    def preflight(self):
        dependencies = ("findmnt", "ss", "ip", "ldd", "openssl", "bash", "timeout", "curl",
                        "readlink", "sed", "awk", "head", "tr", "grep", "stat", "mktemp", "nohup")
        require(all(shutil.which(n) for n in dependencies), "missing dependency; stop without repair")
        inventory = self.idle()
        cfg = self.validate_config()
        marker = json.loads(safe(self.root / "run/config-prepared.json", self.root).read_text())
        require(marker.get("fixture", FIXTURE) == self.fixture and marker["role"] == self.role and marker["root"] == str(self.root) and marker["config_sha256"] == digest(self.config), "prepared marker identity differs")
        require(marker["original_sha256"] == digest(safe(self.config.with_name(self.name + ".original.toml"), self.root)), "original config modified")
        addresses = json.loads(self.run(["ip", "-j", "-4", "addr"]))
        require(any(a.get("local") == IPS[self.role] for row in addresses for a in row.get("addr_info", [])), "role IP differs")
        identities = {}
        paths = {self.name: self.binary, "afs-processctl": self.root / "prefix/bin/afs-processctl"}
        if self.role == "ctl":
            paths["afs-trial-config"] = self.root / "prefix/bin/afs-trial-config"
        else:
            paths["io"] = self.root / "tools/io"
            require(Path("/dev/fuse").exists(), "missing /dev/fuse")
            require(shutil.which("fusermount3") or shutil.which("fusermount"), "missing FUSE unmount helper")
        for key, path in paths.items():
            safe(path, self.root)
            require(os.access(path, os.X_OK) and digest(path) == SHA[key], f"fixed executable SHA/permission mismatch: {key}")
            if key in (self.name, "io"):
                with path.open("rb") as stream:
                    header = stream.read(20)
                require(header[:6] == b"\x7fELF\x02\x01" and int.from_bytes(header[18:20], "little") == 183, "not aarch64 ELF")
                require("not found" not in self.run(["ldd", path]), "missing ELF dependency")
            identities[key] = {"path": str(path), "sha256": SHA[key], "bytes": path.stat().st_size}
        output = json.loads(self.run([self.binary, "--config", self.config, "--print-config"]))
        for key, value in cfg.items():
            if key != "fs":
                require(output.get(key) == value, f"print-config differs: {key}")
        require(output.get("dfs") is True and output.get("ownerfs") is False and output.get("ownerfs_mount") is None, "not DFS-only")
        tls = self.root / "etc/tls"
        certs = {"meta": "afs-meta", **{node: node for node in NODES.values()}}
        tls_hashes = {str(safe(tls / "ca.pem", self.root)): digest(tls / "ca.pem")}
        for certificate, hostname in certs.items():
            path = safe(tls / (certificate + ".pem"), self.root)
            ip = IPS["ctl"] if certificate == "meta" else IPS[next(r for r, n in NODES.items() if n == certificate)]
            for flag, value in (("-verify_hostname", hostname), ("-verify_ip", ip)):
                self.run(["openssl", "verify", "-CAfile", tls / "ca.pem", flag, value, path])
            # Trial certificates contain common SANs, so verify the subject CN too.
            subject = self.run(["openssl", "x509", "-in", path, "-noout", "-subject", "-nameopt", "RFC2253"]).strip()
            require(subject == "subject=CN=" + hostname, "TLS subject identity mapping differs")
            tls_hashes[str(path)] = digest(path)
        certificate_key = self.run(["openssl", "x509", "-in", cfg["tls_identity_certificate"], "-pubkey", "-noout"])
        private_key = self.run(["openssl", "pkey", "-in", safe(cfg["tls_identity_private_key"], self.root), "-pubout"])
        require(certificate_key == private_key, "TLS private key mismatch")
        return {"status": "PASS_CONFIG_ADMISSION_ONLY", "identities": identities,
                "config_sha256": digest(self.config), "tls_sha256": tls_hashes, "print_config": output,
                "capacity": self.capacity(), "inventory": inventory,
                "remaining": "live authenticated connectivity/mount, writer commit, two reader content and timing, normal processctl cleanup; not three-sync/3FS qualification"}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("prepare", "preflight"))
    parser.add_argument("role", choices=VOLUMES)
    parser.add_argument("--fixture", choices=FIXTURE_PORTS, default=FIXTURE)
    args = parser.parse_args()
    fixture = Fixture(args.role, args.fixture)
    try:
        fixture.guest()
        result = getattr(fixture, args.action)()
        print(json.dumps({"schema": "afs.dfs_small_fixture.v1", "action": args.action,
                          "role": args.role, "fixture": fixture.fixture, "root": str(fixture.root), "time_unix_ns": time.time_ns(),
                          **result, "commands": fixture.commands}, indent=2))
        return 0
    except (RuntimeError, OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        print(json.dumps({"status": "BLOCKED", "action": args.action, "role": args.role, "fixture": fixture.fixture,
                          "error": str(error), "commands": fixture.commands}, indent=2))
        return 1


if __name__ == "__main__":
    sys.exit(main())
