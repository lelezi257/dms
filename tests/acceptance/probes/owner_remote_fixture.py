#!/usr/bin/env python3
"""One fixed Linux ARM64 Owner remote diagnostic fixture, not acceptance.

AFS lifecycle belongs to the published afs-processctl. This tool prepares only
new fixture configs and owns wait(2) for three foreground stock Moose services.
It never signals a process discovered by name or touches historical fixtures.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import re
import shutil
import signal
import socket
import subprocess
import sys
import time
import tomllib
import uuid
from pathlib import Path


FIXTURE = "owner-remote-6d-bhome-20261007-r1"
WRITE_FIXTURE = "owner-remote-write-6d-bhome-20261007-r1"
DELETE_FIXTURE = "owner-remote-delete-6d-bhome-20261007-r1"
CURRENT_READ_FIXTURE = "owner-remote-read-f03-20261008-r1"
BACKING_HOT_READ_FIXTURE = "owner-remote-backing-hot-f03-20261008-r1"
F03_READ_FIXTURES = (CURRENT_READ_FIXTURE, BACKING_HOT_READ_FIXTURE)
FIXTURES = (FIXTURE, WRITE_FIXTURE, DELETE_FIXTURE, *F03_READ_FIXTURES)
MUTABLE_FIXTURES = (WRITE_FIXTURE, DELETE_FIXTURE, *F03_READ_FIXTURES)
VOLUMES = {"ctl": "/mnt/lima-afsctlstate", "a": "/mnt/lima-afsadata", "b": "/mnt/lima-afsbdata"}
IPS = {"ctl": "192.168.109.11", "a": "192.168.109.12", "b": "192.168.109.13"}
PORT_PLAN = {
    BACKING_HOT_READ_FIXTURE: {"ctl": {"meta_grpc": 25080, "meta_rest": 25081, "matoml": 25240, "matocs": 25241, "matocl": 25242},
                               "a": {"node_grpc": 25180, "node_rest": 25181},
                               "b": {"node_grpc": 25180, "node_rest": 25181, "chunk": 25243}},
    CURRENT_READ_FIXTURE: {"ctl": {"meta_grpc": 24780, "meta_rest": 24781, "matoml": 24940, "matocs": 24941, "matocl": 24942},
                           "a": {"node_grpc": 24880, "node_rest": 24881},
                           "b": {"node_grpc": 24880, "node_rest": 24881, "chunk": 24943}},
    FIXTURE: {"ctl": {"meta_grpc": 22800, "meta_rest": 22801, "matoml": 23040, "matocs": 23041, "matocl": 23042},
              "a": {"node_grpc": 22900, "node_rest": 22901},
              "b": {"node_grpc": 22900, "node_rest": 22901, "chunk": 23043}},
    WRITE_FIXTURE: {"ctl": {"meta_grpc": 22800, "meta_rest": 22801, "matoml": 23040, "matocs": 23041, "matocl": 23042},
                    "a": {"node_grpc": 22900, "node_rest": 22901},
                    "b": {"node_grpc": 22900, "node_rest": 22901, "chunk": 23043}},
    DELETE_FIXTURE: {"ctl": {"meta_grpc": 23400, "meta_rest": 23401, "matoml": 23640, "matocs": 23641, "matocl": 23642},
                     "a": {"node_grpc": 23500, "node_rest": 23501},
                     "b": {"node_grpc": 23500, "node_rest": 23501, "chunk": 23643}},
}
BUDGET = {"ctl": 256 * 1024**2, "a": 256 * 1024**2, "b": 1024**3}
FLOOR = {"ctl": 512 * 1024**2, "a": 1024**3, "b": 4 * 1024**3}
MOOSE = Path("/opt/afs-moose-round3-v85")
ELF = {"meta": "2c7b7d088b759e3b9375080002182aa484a424b4fa216da1fb79a1004e96168e",
       "node": "2cf1f538fe7af332a711f3c66a074ace140c00773826a182709a6445b8ae2645",
       "io": "70ac97c7634d406a177a74c783d446b62a2014ba198586132882a1d9228e55e8"}
CURRENT_READ_ELF = {"meta": "c7447bcfac7e3f8bf605446ade5e11be74f1333709a8f7bb5378b1d6ee7506fd",
                    "node": "3b1f1dce187a6285814c03b9024cdfc5f2dec990a73cba9cc13b3dc9ef402d36",
                    "io": "0d4346c99ad5ed3a7af0306c7db965eb1d6ea57595a89d639216a2be15c9fa8e"}
CURRENT_READ_BUDGET = {"ctl": 256 * 1024**2, "a": 512 * 1024**2, "b": 1024**3}
MOOSE_SHA = {
    "sbin/mfsmaster": "9febf4e7a9ae5285c4e792024f98d8a5531ae325f19268e103693981be509b70",
    "sbin/mfschunkserver": "422a36b7e31a6aa0bfbef08efe083d9d55003aca0a9a7bd0121b730c33f97303",
    "bin/mfsmount": "5752f91e2d174cc8a0a53df7b3713b3bed31fde5fe72a6f38cd86237ae3ebc37",
    "bin/mfscli": "247194b8e45c6cbd7bf0ae95f9e5b709ee0f08e669739e4ffef46c7659ec09df",
    "bin/mfscreatesclass": "a5aaa1875d9db7bcb36e2c0456285982965ff20ab79bedce701291b2a70993d0",
    "bin/mfslistsclass": "a5aaa1875d9db7bcb36e2c0456285982965ff20ab79bedce701291b2a70993d0",
    "bin/mfssetsclass": "7dc44d9383376bb456779516bb59ec65e4cc35e404c1a0ab82ffa781f703a7e3",
    "bin/mfsgetsclass": "7dc44d9383376bb456779516bb59ec65e4cc35e404c1a0ab82ffa781f703a7e3",
    "bin/mfsfileinfo": "d58823c1c07aa8a65c55931e6007b8c20b88ffbc4eb825ae057dfe5a452c93b6",
    "bin/mfscheckfile": "d58823c1c07aa8a65c55931e6007b8c20b88ffbc4eb825ae057dfe5a452c93b6",
    "var/mfs/metadata.mfs.empty": "e3b24c138492108e195cd0b310d5cb1bbebf02baed1ab4b4d6d07d9ad682a4ed",
}


def require(ok, message):
    if not ok:
        raise RuntimeError(message)


def digest(path):
    h = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for block in iter(lambda: stream.read(1024**2), b""):
            h.update(block)
    return h.hexdigest()


def write_json(path, value):
    # Same-directory atomic receipt: readers never accept an incomplete JSON.
    temporary = path.with_name(path.name + ".tmp-" + uuid.uuid4().hex)
    with temporary.open("x") as stream:
        json.dump(value, stream, indent=2)
        stream.write("\n")
        stream.flush()
        os.fsync(stream.fileno())
    os.replace(temporary, path)


def safe(path, boundary, exists=True):
    path, boundary = Path(path), Path(boundary)
    require(path.is_absolute() and path.is_relative_to(boundary), f"path outside boundary: {path}")
    require(".." not in path.parts, f"path traversal: {path}")
    for part in [path, *path.parents]:
        if part.exists() or part.is_symlink():
            require(not part.is_symlink(), f"symlink path component: {part}")
        if part == boundary:
            break
    if exists:
        require(path.exists(), f"missing path: {path}")
    return path


def tool(relative):
    # Stock tools contain aliases; allow only aliases resolving inside prefix.
    path = MOOSE / relative
    resolved = path.resolve(strict=True)
    require(resolved.is_relative_to(MOOSE.resolve(strict=True)) and resolved.is_file(), f"Moose alias escape: {path}")
    require(digest(resolved) == MOOSE_SHA[relative], f"fixed Moose SHA mismatch: {relative}")
    return path


class Fixture:
    def __init__(self, role, fixture=FIXTURE):
        require(fixture in FIXTURES, "unknown fixed fixture")
        self.fixture = fixture
        self.role = role
        self.volume = Path(VOLUMES[role])
        self.root = self.volume / "afs-delivery" / fixture
        self.name = "meta" if role == "ctl" else "node"
        self.binary = self.root / "prefix/bin" / ("afs-" + self.name)
        self.config = self.root / "etc" / (self.name + ".toml")
        self.service = {"ctl": "master", "a": "mount", "b": "chunk"}[role]
        self.commands = []

    def ports(self):
        return PORT_PLAN[self.fixture][self.role]

    def port(self, name):
        return self.ports()[name]

    def start_bind_ports(self):
        if self.role == "ctl":
            return [self.port("matoml"), self.port("matocs"), self.port("matocl")]
        if self.role == "b":
            return [self.port("chunk")]
        return []

    def guest(self):
        require(platform.system() == "Linux" and platform.machine() == "aarch64", "Linux ARM64 guest only")
        require(os.geteuid() == 0, "guest root required")
        require(platform.node() == "lima-afs-accept-" + self.role, "wrong guest role/hostname")
        safe(self.root, self.volume)
        require(self.root.stat().st_uid == 0, "fixture root must be root-owned")
        for relative in ("run", "logs", "evidence"):
            path = safe(self.root / relative, self.root, exists=False)
            path.mkdir(exist_ok=True)

    def run(self, argv, allowed=(0,), timeout=15):
        result = subprocess.run(list(map(str, argv)), text=True, capture_output=True, timeout=timeout)
        record = {"argv": result.args, "returncode": result.returncode, "stdout": result.stdout, "stderr": result.stderr}
        self.commands.append(record)
        require(result.returncode in allowed, f"command failed: {record}")
        return result.stdout

    def receipt(self, action, value):
        value = {"schema": "afs.owner_remote_fixture.v1", "action": action, "role": self.role,
                 "root": str(self.root), "time_unix_ns": time.time_ns(), "commands": self.commands, **value}
        path = self.root / "evidence" / (action + "-" + uuid.uuid4().hex + ".json")
        write_json(path, value)
        print(json.dumps({"receipt": str(path), **value}), flush=True)
        return value

    def exact_mount(self, path):
        raw = self.run(["findmnt", "-J", "--mountpoint", path, "-o", "TARGET,SOURCE,FSTYPE,OPTIONS,ID"], allowed=(0, 1))
        if not raw.strip():
            return None
        rows = json.loads(raw)["filesystems"]
        require(len(rows) == 1 and rows[0]["target"] == str(path), "ambiguous exact mount")
        return rows[0]

    def capacity(self, admission=False):
        rows = json.loads(self.run(["findmnt", "-J", "--mountpoint", self.volume]))["filesystems"]
        require(len(rows) == 1 and rows[0]["target"] == str(self.volume) and rows[0]["fstype"] == "ext4", "dedicated ext4 volume required")
        require(self.root.stat().st_dev == self.volume.stat().st_dev, "fixture escapes data volume")
        info = os.statvfs(self.volume)
        available = info.f_bavail * info.f_frsize
        used = 0
        # Do not descend FUSE mounts or hash/read dataset contents.
        for directory, directories, files in os.walk(self.root, followlinks=False):
            if Path(directory) == self.root:
                directories[:] = [d for d in directories if d != "mount"]
            for entry in [Path(directory), *(Path(directory) / f for f in files)]:
                st = entry.lstat()
                used += st.st_blocks * 512
            for name in list(directories):
                entry = Path(directory) / name
                require(not entry.is_symlink() and entry.stat().st_dev == self.volume.stat().st_dev, f"nested state escape: {entry}")
        budget = (CURRENT_READ_BUDGET if self.fixture in F03_READ_FIXTURES else BUDGET)[self.role]
        require(available >= FLOOR[self.role], "case free-space floor exceeded; preserve state")
        require(used <= budget, "case working budget exceeded; preserve state")
        if admission:
            require(available + used >= FLOOR[self.role] + budget, "insufficient new case working budget")
        return {"mount": rows[0], "available_bytes": available, "owned_allocated_bytes": used,
                "working_budget_bytes": budget, "remaining_free_floor_bytes": FLOOR[self.role]}

    def empty_ports(self):
        raw = self.run(["ss", "-ltnp"])
        ports = sorted(self.ports().values())
        for port in ports:
            require(not re.search(rf":{port}\s", raw), f"selected new port occupied: {port}")
        return ports

    def moose_configs(self):
        root = self.root
        common = "WORKING_USER = root\nWORKING_GROUP = root\nNICE_LEVEL = 0\nDISABLE_OOM_KILLER = 0\n"
        if self.role == "ctl":
            return {"mfsmaster.cfg": common + f"""DATA_PATH = {root}/state/moose/master
EXPORTS_FILENAME = {root}/config/mfsexports.cfg
MATOML_LISTEN_HOST = {IPS['ctl']}
MATOML_LISTEN_PORT = {self.port('matoml')}
MATOCS_LISTEN_HOST = {IPS['ctl']}
MATOCS_LISTEN_PORT = {self.port('matocs')}
MATOCL_LISTEN_HOST = {IPS['ctl']}
MATOCL_LISTEN_PORT = {self.port('matocl')}
CHANGELOG_SAVE_MODE = 2
""", "mfsexports.cfg": f"{IPS['a']} / rw,alldirs,admin,maproot=0:0\n"}
        if self.role == "b":
            return {"mfschunkserver.cfg": common + f"""DATA_PATH = {root}/state/moose/chunkstate
HDD_CONF_FILENAME = {root}/config/mfshdd.cfg
MASTER_HOST = {IPS['ctl']}
MASTER_PORT = {PORT_PLAN[self.fixture]['ctl']['matocs']}
BIND_HOST = {IPS['b']}
CSSERV_LISTEN_HOST = {IPS['b']}
CSSERV_LISTEN_PORT = {self.port('chunk')}
HDD_LEAVE_SPACE_DEFAULT = 4GiB
HDD_FSYNC_BEFORE_CLOSE = 1
""", "mfshdd.cfg": f"{root}/state/moose/chunks\n"}
        return {}

    def patch_toml(self, original):
        parsed = tomllib.loads(original)
        require(not re.search(r"^\s*\[", original, re.M), "expected generated flat TOML only")
        expected_id = "meta-ctl" if self.role == "ctl" else "remote-" + self.role + "-r1"
        require(parsed["id"] == expected_id and parsed["fs"] == "all", "not expected original generated config")
        require(set(parsed["trusted_node_certs"]) == {"remote-a-r1", "remote-b-r1"}, "unexpected trust identities")
        changed = {"fs": "ownerfs", "experimental_native_workspace": False,
                   "data_dir": str(self.root / "state" / self.name), "log_level": "info", "trace_enabled": False}
        if self.fixture in F03_READ_FIXTURES:
            changed["experimental_ownerfs_workspace_bind"] = False
        for key in ("tls_ca_certificate", "tls_identity_certificate", "tls_identity_private_key"):
            changed[key] = str(self.root / "etc/tls" / Path(parsed[key]).name)
        certs = {k: str(self.root / "etc/tls" / Path(v).name) for k, v in parsed["trusted_node_certs"].items()}
        if self.role == "ctl":
            require(parsed["meta_store"] == "local-file", "local-file Meta required")
            changed.update(grpc_listen=f"0.0.0.0:{self.port('meta_grpc')}", rest_listen=f"0.0.0.0:{self.port('meta_rest')}")
        else:
            changed.update(uds_path=str(self.root / "run/node.sock"), ownerfs_mount=str(self.root / "mount/ownerfs"),
                           meta_endpoint=f"https://{IPS['ctl']}:{PORT_PLAN[self.fixture]['ctl']['meta_grpc']}", data_mode="grpc", allow_volatile_meta=False,
                           advertise_endpoint=f"https://{IPS[self.role]}:{self.port('node_grpc')}", grpc_listen=f"0.0.0.0:{self.port('node_grpc')}", rest_listen=f"0.0.0.0:{self.port('node_rest')}")
        lines = []
        remaining = dict(changed)
        for line in original.splitlines():
            match = re.match(r"^\s*([a-z_]+)\s*=", line)
            key = match.group(1) if match else None
            if key == "dfs_mount":
                continue
            if key == "trusted_node_certs":
                line = "trusted_node_certs = { " + ", ".join(k + " = " + json.dumps(v) for k, v in certs.items()) + " }"
            elif key in remaining:
                line = key + " = " + json.dumps(remaining.pop(key))
            lines.append(line)
        lines.extend(k + " = " + json.dumps(v) for k, v in remaining.items())
        text = "\n".join(lines) + "\n"
        result = tomllib.loads(text)
        require("dfs_mount" not in result and result["fs"] == "ownerfs", "bad Owner-only adaptation")
        return text

    def prepare(self):
        require(self.fixture in MUTABLE_FIXTURES, "historical read fixture is closed; use a fresh mutable fixture")
        marker = self.root / "run/config-prepared.json"
        require(not marker.exists(), "prepare is exclusive; use preflight-config for existing prepared fixture")
        self.capacity(admission=True)
        self.empty_ports()
        require(self.exact_mount(self.root / "mount/ownerfs") is None and self.exact_mount(self.root / "mount/moose") is None, "new mount collision")
        safe(self.config, self.root)
        original = self.config.read_text()
        text = self.patch_toml(original)
        backup = self.config.with_name(self.config.stem + ".original.toml")
        with backup.open("x") as stream:
            stream.write(original)
        # All directories are solely within the new fixture.
        for relative in ("config", "state", "state/" + self.name, "mount", "mount/ownerfs", "mount/moose"):
            safe(self.root / relative, self.root, exists=False).mkdir(exist_ok=True)
        for name, contents in self.moose_configs().items():
            with safe(self.root / "config" / name, self.root, exists=False).open("x") as stream:
                stream.write(contents)
        if self.role == "ctl":
            master = self.root / "state/moose/master"
            safe(master, self.root, exists=False).mkdir(parents=True)
            with (master / "metadata.mfs").open("xb") as stream:
                stream.write(tool("var/mfs/metadata.mfs.empty").read_bytes())
        elif self.role == "b":
            for name in ("chunkstate", "chunks"):
                safe(self.root / "state/moose" / name, self.root, exists=False).mkdir(parents=True, exist_ok=True)
        self.config.write_text(text)
        value = {"role": self.role, "root": str(self.root), "config_sha256": self.config_hashes(), "original_sha256": digest(backup)}
        write_json(marker, value)
        return {"prepared": value, "note": "configuration only; preflight-config still required"}

    def config_hashes(self):
        files = [self.config, *(self.root / "config" / name for name in self.moose_configs())]
        return {str(safe(p, self.root)): digest(p) for p in files}

    def prepared(self):
        marker = json.loads(safe(self.root / "run/config-prepared.json", self.root).read_text())
        require(marker["role"] == self.role and marker["root"] == str(self.root), "prepared identity differs")
        require(marker["config_sha256"] == self.config_hashes(), "prepared config modified")
        return marker

    def check_elf(self, path, expected):
        safe(path, self.root)
        with path.open("rb") as stream:
            require(stream.read(4) == b"\x7fELF", f"not ELF: {path}")
        require(digest(path) == expected, f"fixed ELF SHA mismatch: {path}")
        raw = self.run(["ldd", path])
        require("not found" not in raw, f"missing ELF dependency: {path}")
        return {"path": str(path), "sha256": expected, "bytes": path.stat().st_size}

    def preflight(self, io_path):
        require(self.fixture in MUTABLE_FIXTURES, "historical read fixture admission is closed; use a fresh mutable fixture")
        self.prepared()
        dependency_names = ("findmnt", "ss", "ip", "ldd", "openssl")
        require(all(shutil.which(n) for n in dependency_names), "missing preflight dependency")
        require(hasattr(os, "pidfd_open") and hasattr(signal, "pidfd_send_signal"), "Python pidfd API required; stop without environment repair")
        addresses = json.loads(self.run(["ip", "-j", "-4", "addr"]))
        require(any(a.get("local") == IPS[self.role] for row in addresses for a in row.get("addr_info", [])), "role IP differs")
        self.empty_ports()
        for name in ("ownerfs", "moose"):
            require(self.exact_mount(self.root / "mount" / name) is None, "new mount already active")
        expected_elf = CURRENT_READ_ELF if self.fixture in F03_READ_FIXTURES else ELF
        identities = {self.name: self.check_elf(self.binary, expected_elf[self.name])}
        if self.role == "a":
            safe(io_path, self.root / "tools")
            identities["io"] = self.check_elf(io_path, expected_elf["io"])
            identities["fusermount3"] = self.inspect_unmount()
        selected = {"ctl": ["sbin/mfsmaster", "var/mfs/metadata.mfs.empty"],
                    "b": ["sbin/mfschunkserver"], "a": [p for p in MOOSE_SHA if p.startswith("bin/")]}[self.role]
        for relative in selected:
            path = tool(relative)
            identities[relative] = {"path": str(path), "resolved_path": str(path.resolve()), "sha256": MOOSE_SHA[relative]}
            if relative.startswith("sbin/") or relative == "bin/mfsmount":
                require("not found" not in self.run(["ldd", path]), "missing stock Moose dependency")
        cfg = tomllib.loads(self.config.read_text())
        output = json.loads(self.run([self.binary, "--config", self.config, "--print-config"]))
        for key in ("id", "data_dir", "grpc_listen", "rest_listen", "tls_ca_certificate", "tls_identity_certificate", "tls_identity_private_key", "trusted_node_certs"):
            require(output.get(key) == cfg[key], f"print-config differs: {key}")
        require(output.get("ownerfs") is True and output.get("dfs") is False and output.get("experimental_native_workspace") is False, "print-config backend/native differs")
        if self.fixture in F03_READ_FIXTURES:
            require(cfg.get("experimental_ownerfs_workspace_bind") is False and output.get("experimental_ownerfs_workspace_bind") is False, "workspace bind must be explicitly OFF")
        require(output.get("dfs_mount") is None, "unexpected DFS mount")
        if self.role != "ctl":
            for key in ("uds_path", "ownerfs_mount", "meta_endpoint", "advertise_endpoint", "data_mode"):
                require(output.get(key) == cfg[key], f"print-config differs: {key}")
        tls = self.root / "etc/tls"
        files = [tls / "meta.pem", tls / "remote-a-r1.pem", tls / "remote-b-r1.pem"]
        for certificate in files:
            safe(certificate, self.root)
            self.run(["openssl", "verify", "-CAfile", tls / "ca.pem", certificate])
        for role in ("a", "b"):
            self.run(["openssl", "verify", "-CAfile", tls / "ca.pem", "-verify_ip", IPS[role], tls / ("remote-" + role + "-r1.pem")])
        self.run(["openssl", "verify", "-CAfile", tls / "ca.pem", "-verify_hostname", "afs-meta", tls / "meta.pem"])
        self.run(["openssl", "x509", "-in", cfg["tls_identity_certificate"], "-noout", "-subject", "-ext", "subjectAltName"])
        public_cert = self.run(["openssl", "x509", "-in", cfg["tls_identity_certificate"], "-pubkey", "-noout"])
        public_key = self.run(["openssl", "pkey", "-in", cfg["tls_identity_private_key"], "-pubout"])
        require(public_cert == public_key, "TLS key does not match certificate")
        result = {"status": "PASS_CONFIG_ADMISSION_ONLY", "identities": identities, "print_config": output,
                  "config_sha256": self.config_hashes(),
                  "tls_sha256": {str(p): digest(safe(p, self.root)) for p in [tls / "ca.pem", *files]},
                  "capacity": self.capacity(admission=True), "remaining": "authenticated connectivity, live mount/Home/class/topology proofs belong to execution"}
        write_json(self.root / "run/config-admitted.json", result)
        return result

    def inspect_unmount(self):
        identity = self.unmount_identity()
        # Use the short options advertised by the installed stock helper. Its
        # help begins with '<program>: [options] mountpoint', not 'Usage:'.
        # Preserve the raw help returncode, including the observed exit 1.
        text = self.run([identity["path"], "-h"], allowed=(0, 1))
        text += self.commands[-1]["stderr"]
        patterns = (r"^(?:/usr/bin/)?fusermount3:\s*\[options\]\s+mountpoint\s*$",
                    r"^Options:\s*$", r"^\s*-h\s+print help\s*$",
                    r"^\s*-V\s+print version\s*$", r"^\s*-u\s+unmount\s*$")
        require(all(re.search(pattern, text, re.M) for pattern in patterns), "fusermount3 normal unmount capability missing")
        self.run([identity["path"], "-V"])
        require("not found" not in self.run(["ldd", identity["path"]]), "fusermount3 dependency missing")
        return identity

    def unmount_identity(self):
        path = shutil.which("fusermount3")
        require(path is not None, "fusermount3 missing; stop without installation or repair")
        resolved = Path(path).resolve(strict=True)
        require(resolved == Path("/usr/bin/fusermount3"), "unexpected system fusermount3 path")
        st = resolved.stat()
        require(st.st_uid == 0 and not st.st_mode & 0o022 and os.access(resolved, os.X_OK), "untrusted fusermount3 ownership/permission")
        return {"path": str(resolved), "sha256": digest(resolved), "device": st.st_dev, "inode": st.st_ino}

    def admitted_unmount(self):
        admitted = json.loads(safe(self.root / "run/config-admitted.json", self.root).read_text())
        require(admitted["config_sha256"] == self.config_hashes(), "configuration admission changed")
        expected = admitted.get("identities", {}).get("fusermount3")
        require(expected is not None and self.unmount_identity() == expected, "fusermount3 identity differs from admission")
        return expected["path"]

    def moose_argv(self):
        if self.role == "ctl":
            return [str(tool("sbin/mfsmaster")), "-f", "-c", str(self.root / "config/mfsmaster.cfg"), "start"]
        if self.role == "b":
            return [str(tool("sbin/mfschunkserver")), "-f", "-c", str(self.root / "config/mfschunkserver.cfg"), "start"]
        cache = "DIRECT" if self.fixture == CURRENT_READ_FIXTURE else "AUTO"
        return [str(tool("bin/mfsmount")), "-f", "-H", IPS["ctl"], "-P", str(PORT_PLAN[self.fixture]["ctl"]["matocl"]), "-o", f"allow_other,mfsnice=0,mfscachemode={cache},mfstimeout=30", str(self.root / "mount/moose")]

    def current_lifecycle(self):
        pointer = self.root / "run" / ("moose-" + self.service + ".json")
        value = json.loads(safe(pointer, self.root).read_text())
        lifecycle = safe(Path(value["lifecycle"]), self.root / "run")
        require(lifecycle.name.startswith("moose-" + self.service + "-"), "lifecycle role differs")
        return lifecycle

    def validate_child(self, identity):
        require(identity["role"] == self.role and identity["root"] == str(self.root), "child role/root differs")
        require(identity.get("fixture") == self.fixture and identity["script_sha256"] == digest(Path(__file__)), "child fixture/source differs")
        require(identity["boot_id"] == Path("/proc/sys/kernel/random/boot_id").read_text().strip(), "child boot differs")
        require(identity["argv"] == self.moose_argv() and identity["config_sha256"] == self.config_hashes(), "child launch config differs")
        before = fingerprint(identity["pid"])
        require(before["state"] != "Z", "child is exited; require supervisor wait receipt")
        for key in ("start_ticks", "exe_path", "exe_dev", "exe_inode"):
            require(before[key] == identity[key], f"child incarnation differs: {key}")
        require(before["exe_path"] == str(Path(identity["argv"][0]).resolve()), "child executable path differs")
        require(digest(Path("/proc") / str(identity["pid"]) / "exe") == identity["exe_sha256"], "live child SHA differs")
        require(same_incarnation(before, fingerprint(identity["pid"])), "child changed during identity validation")
        if self.role == "a":
            mount = self.exact_mount(self.root / "mount/moose")
            if mount is not None:
                require(mount["source"] == f"mfs#{IPS['ctl']}:{PORT_PLAN[self.fixture]['ctl']['matocl']}" and mount["fstype"] in ("fuse", "fuse.mfs"), "Moose mount source differs")
                if identity.get("mount"):
                    require(mount == identity["mount"], "Moose mount incarnation differs")
            cmdline = (Path("/proc") / str(identity["pid"]) / "cmdline").read_bytes().split(b"\0")
            argv = [a.decode() for a in cmdline if a]
            title = [f"mfsmount (mounted on: {self.root / 'mount/moose'})"]
            require(argv == identity["argv"] or (mount is not None and argv == title), "mount child argv/title differs")
        else:
            argv = (Path("/proc") / str(identity["pid"]) / "cmdline").read_bytes().split(b"\0")
            require([a.decode() for a in argv if a] == identity["argv"], "server child argv differs")
        return before

    def start(self, timeout):
        self.prepared()
        admitted = json.loads(safe(self.root / "run/config-admitted.json", self.root).read_text())
        require(admitted["config_sha256"] == self.config_hashes(), "configuration admission changed")
        self.capacity()
        require(self.exact_mount(self.root / "mount/moose") is None, "Moose mount collision")
        pointer = self.root / "run" / ("moose-" + self.service + ".json")
        if pointer.exists():
            previous = self.current_lifecycle()
            require((previous / "exit.json").exists(), "previous owned launch has no wait receipt; inspect before retry")
        for port in self.start_bind_ports():
            with socket.socket() as listener:
                listener.bind((IPS[self.role], port))
        lifecycle = self.root / "run" / ("moose-" + self.service + "-" + uuid.uuid4().hex)
        lifecycle.mkdir()
        launch = {"role": self.role, "fixture": self.fixture, "root": str(self.root), "argv": self.moose_argv(), "config_sha256": self.config_hashes(),
                  "script_sha256": digest(Path(__file__)), "boot_id": Path("/proc/sys/kernel/random/boot_id").read_text().strip()}
        write_json(lifecycle / "launch.json", launch)
        with (lifecycle / "supervisor.log").open("xb") as log:
            supervisor = subprocess.Popen(self.supervisor_argv(lifecycle),
                                          stdin=subprocess.DEVNULL, stdout=log, stderr=log, start_new_session=True, close_fds=True)
        write_json(pointer, {"lifecycle": str(lifecycle), "supervisor_pid": supervisor.pid})
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            require(not (lifecycle / "exit.json").exists(), "Moose exited during startup; preserve lifecycle receipt")
            child = lifecycle / "child.json"
            if child.exists():
                identity = json.loads(child.read_text())
                self.validate_child(identity)
                ready = False
                if self.role == "a":
                    mount = self.exact_mount(self.root / "mount/moose")
                    if mount:
                        identity["mount"] = mount
                        write_json(lifecycle / "child.json", identity)
                        ready = True
                else:
                    raw = self.run(["ss", "-ltnp"])
                    needed = [self.port("matoml"), self.port("matocs"), self.port("matocl")] if self.role == "ctl" else [self.port("chunk")]
                    ready = all(any(re.search(rf":{p}\s", line) and f"pid={identity['pid']}," in line for line in raw.splitlines()) for p in needed)
                if ready:
                    return {"status": "STARTED_IDENTITY_ONLY", "lifecycle": str(lifecycle), "child": identity}
            time.sleep(0.1)
        raise RuntimeError(f"Moose start readiness timeout; owned state retained: {lifecycle}")

    def supervisor_argv(self, lifecycle):
        return [sys.executable, str(Path(__file__).resolve()), "__supervise", self.role,
                "--fixture", self.fixture, "--lifecycle", str(lifecycle)]

    def supervise(self, lifecycle):
        lifecycle = safe(lifecycle, self.root / "run")
        launch = json.loads((lifecycle / "launch.json").read_text())
        require(launch["role"] == self.role and launch["root"] == str(self.root), "supervisor role/root differs")
        require(launch.get("fixture") == self.fixture and launch["boot_id"] == Path("/proc/sys/kernel/random/boot_id").read_text().strip(), "supervisor fixture/boot differs")
        require(launch["script_sha256"] == digest(Path(__file__)), "supervisor source changed after launch")
        require(launch["argv"] == self.moose_argv() and launch["config_sha256"] == self.config_hashes(), "supervisor launch modified")
        with (lifecycle / "service.log").open("xb") as log:
            child = subprocess.Popen(launch["argv"], stdin=subprocess.DEVNULL, stdout=log, stderr=log, close_fds=True)
            identity = None
            identity_error = None
            try:
                observed = fingerprint(child.pid)
                require(observed["exe_path"] == str(Path(launch["argv"][0]).resolve()), "unexpected spawned child exe")
                identity = {**launch, **observed, "pid": child.pid, "supervisor_pid": os.getpid(),
                            "exe_sha256": digest(Path("/proc") / str(child.pid) / "exe")}
                require(same_incarnation(observed, fingerprint(child.pid)), "spawned child changed during hashing")
                write_json(lifecycle / "child.json", identity)
            except Exception as exc:
                identity_error = str(exc)
                write_json(lifecycle / "identity-error.json", {"pid": child.pid, "error": identity_error})
            # No timeout/kill: this exact parent owns real wait status, including
            # startup failures and external TERM. Keep ownership on identity error.
            code = child.wait()
            write_json(lifecycle / "exit.json", {"role": self.role, "fixture": self.fixture, "root": str(self.root), "pid": child.pid,
                       "supervisor_pid": os.getpid(), "exit_code": code, "identity": identity,
                       "identity_error": identity_error, "wait_completed": True, "time_unix_ns": time.time_ns()})

    def stop(self, timeout):
        lifecycle = self.current_lifecycle()
        exit_path = lifecycle / "exit.json"
        identity = json.loads(safe(lifecycle / "child.json", self.root).read_text())
        if not exit_path.exists():
            pidfd = None
            try:
                pidfd = os.pidfd_open(identity["pid"])
                self.validate_child(identity)
                if self.role == "a":
                    helper = self.admitted_unmount()
                    # Startup may observe no mount, but stop never unmounts an
                    # unrecorded or replaced mount. Check immediately before -u.
                    self.validate_child(identity)
                    mount = self.exact_mount(self.root / "mount/moose")
                    require(identity.get("mount") is not None and mount == identity["mount"], "normal unmount requires original owned mount incarnation")
                    self.run([helper, "-u", self.root / "mount/moose"], timeout=timeout)
                else:
                    signal.pidfd_send_signal(pidfd, signal.SIGTERM)
            except (ProcessLookupError, FileNotFoundError):
                # The child can exit between the receipt check and pidfd/hash.
                # Accept only its actual supervisor wait receipt below.
                pass
            finally:
                if pidfd is not None:
                    os.close(pidfd)
        deadline = time.monotonic() + timeout
        while not exit_path.exists() and time.monotonic() < deadline:
            time.sleep(0.1)
        require(exit_path.exists(), "normal stop wait receipt timeout; preserve process/state, no KILL")
        receipt = json.loads(exit_path.read_text())
        require(receipt["role"] == self.role and receipt["root"] == str(self.root) and receipt.get("fixture") == self.fixture, "exit receipt role/root/fixture differs")
        require(receipt["wait_completed"] is True and receipt["pid"] == identity["pid"] and receipt["identity"] is not None, "exit receipt lacks owned wait/identity")
        for key in ("role", "root", "fixture", "boot_id", "argv", "config_sha256", "script_sha256", "start_ticks", "exe_path", "exe_dev", "exe_inode", "exe_sha256"):
            require(receipt["identity"].get(key) == identity.get(key), f"exit receipt identity differs: {key}")
        require(identity.get("fixture") == self.fixture and identity["root"] == str(self.root)
                and identity["role"] == self.role and identity["script_sha256"] == digest(Path(__file__)), "exit child fixture/source differs")
        require(identity["boot_id"] == Path("/proc/sys/kernel/random/boot_id").read_text().strip()
                and identity["argv"] == self.moose_argv() and identity["config_sha256"] == self.config_hashes(), "exit child boot/argv/config differs")
        if self.role == "a":
            require(self.exact_mount(self.root / "mount/moose") is None, "Moose mount remains after normal stop; no lazy unmount")
        require(receipt["exit_code"] == 0, f"Moose nonzero real wait exit: {receipt['exit_code']}")
        return {"status": "STOPPED", "lifecycle": str(lifecycle), "exit": receipt}

    def postcheck(self):
        self.prepared()
        self.empty_ports()
        for name in ("ownerfs", "moose"):
            require(self.exact_mount(self.root / "mount" / name) is None, "owned mount remains")
        lifecycle = self.current_lifecycle()
        receipt = json.loads(safe(lifecycle / "exit.json", self.root).read_text())
        require(receipt["wait_completed"] is True and receipt["exit_code"] == 0, "no successful Moose wait receipt")
        return {"status": "PASS_OWNED_CLEANUP_ONLY", "moose_exit": receipt, "capacity": self.capacity(), "old_services": "not touched; parent retains independent old-cohort inventory"}


def fingerprint(pid):
    proc = Path("/proc") / str(pid)
    raw = (proc / "stat").read_text()
    fields = raw[raw.rindex(")") + 2:].split()
    exe = proc / "exe"
    st = exe.stat()
    return {"state": fields[0], "start_ticks": int(fields[19]), "exe_path": os.readlink(exe), "exe_dev": st.st_dev, "exe_inode": st.st_ino}


def same_incarnation(before, after):
    # R/S are ordinary scheduling states, not process identity.
    return all(before[key] == after[key] for key in ("start_ticks", "exe_path", "exe_dev", "exe_inode"))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("prepare-config", "preflight-config", "moose-start", "moose-stop", "postcheck", "__supervise"))
    parser.add_argument("role", choices=("ctl", "a", "b"))
    parser.add_argument("--fixture", choices=FIXTURES, default=FIXTURE)
    parser.add_argument("--timeout", type=float, default=30)
    parser.add_argument("--io-path", type=Path)
    parser.add_argument("--lifecycle", type=Path)
    args = parser.parse_args()
    fixture = Fixture(args.role, args.fixture)
    try:
        fixture.guest()
        require(0 < args.timeout <= 60, "bounded timeout must be0<seconds<=60")
        if args.command == "__supervise":
            require(args.lifecycle is not None, "internal supervisor lifecycle required")
            fixture.supervise(args.lifecycle)
            return
        if args.command == "prepare-config":
            result = fixture.prepare()
        elif args.command == "preflight-config":
            result = fixture.preflight(args.io_path or fixture.root / "tools/io")
        elif args.command == "moose-start":
            result = fixture.start(args.timeout)
        elif args.command == "moose-stop":
            result = fixture.stop(args.timeout)
        else:
            result = fixture.postcheck()
        fixture.receipt(args.command, result)
    except Exception as exc:
        value = {"status": "BLOCKED", "error": str(exc), "scope": "affected fixture action only; state retained, no environment repair"}
        if platform.system() == "Linux" and (fixture.root / "evidence").is_dir():
            fixture.receipt(args.command, value)
        else:
            print(json.dumps(value), flush=True)
        raise SystemExit(1)


if __name__ == "__main__":
    main()
