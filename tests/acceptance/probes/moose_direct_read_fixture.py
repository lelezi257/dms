#!/usr/bin/env python3
"""Bounded stock Moose DIRECT read fixture; no AFS or historical lifecycle ownership.

Reuse the maintained foreground/pidfd/true-wait lifecycle. The inherited client
role is ``a`` internally; it always denotes the actual C guest here. Parent owns
comparison timing, content/network-read proofs, and protected-service inventory.
"""
from __future__ import annotations

import argparse
import importlib.util
import json
import os
import platform
import shutil
import signal
import sys
from pathlib import Path

# Use an isolated module instance so importing this adapter cannot change the
# historical fixture defaults used by other acceptance helpers or test suites.
_base_spec = importlib.util.spec_from_file_location("_moose_direct_lifecycle", Path(__file__).resolve().with_name("owner_remote_fixture.py"))
base = importlib.util.module_from_spec(_base_spec)
_base_spec.loader.exec_module(base)

RUN = "owner-remote-direct-read-20261009-r1"
CLIENT_IP = "192.168.109.14"
WRAPPER = Path(__file__).resolve()
BASE = Path(base.__file__).resolve()
OLD = base.CURRENT_READ_FIXTURE
VOLUMES = {"ctl": "/mnt/lima-afsctlstate", "b": "/mnt/lima-afsbdata", "a": "/mnt/lima-afscdata"}
LIMITS = {"ctl": 256 * 1024**2, "b": 512 * 1024**2, "a": 512 * 1024**2}
FLOORS = {"ctl": 512 * 1024**2, "b": 4 * 1024**3, "a": 4 * 1024**3}
GROWTH = 64 * 1024**2
LOG_LIMIT = 64 * 1024**2

# Register only this fixed adapter profile in this imported module instance.
base.FIXTURES = (*base.FIXTURES, RUN)
base.VOLUMES = VOLUMES
base.IPS = {**base.IPS, "a": CLIENT_IP}
base.PORT_PLAN[RUN] = {"ctl": {"matoml": 24940, "matocs": 24941, "matocl": 24942}, "b": {"chunk": 24943}, "a": {}}


class Fixture(base.Fixture):
    def __init__(self, actual_role, expectations):
        base.require(actual_role in ("ctl", "b", "c"), "unknown actual guest role")
        self.actual_role = actual_role
        super().__init__("a" if actual_role == "c" else actual_role, RUN)
        self.expectations_path = Path(expectations)
        base.require(self.expectations_path.is_absolute(), "absolute expectations path required")
        base.safe(self.expectations_path, Path("/"))
        self.expected = json.loads(self.expectations_path.read_text())
        base.require(self.expected.get("schema") == "afs.moose_direct_read_expectations.v1", "expectations schema differs")
        for name, path in (("wrapper", WRAPPER), ("base", BASE)):
            base.require(base.digest(path) == self.expected[name + "_sha256"], f"fixed {name} source differs")
        self.contract = Path(self.expected["contract_path"])
        base.safe(self.contract, Path("/"))
        base.require(base.digest(self.contract) == self.expected["contract_sha256"], "frozen comparison contract differs")
        base.require(self.expected["mfsmount_sha256"] == base.MOOSE_SHA["bin/mfsmount"], "non-stock mount expectation")
        self.old_root = self.volume / "afs-delivery" / OLD

    def guest(self):
        base.require(platform.system() == "Linux" and platform.machine() == "aarch64", "Linux ARM64 guest only")
        base.require(os.geteuid() == 0, "guest root required")
        base.require(platform.node() == "lima-afs-accept-" + self.actual_role, "wrong guest hostname")
        dependencies = ("findmnt", "ss", "ip", "ldd")
        base.require(all(shutil.which(n) for n in dependencies), "missing dependency; no environment repair")
        base.require(hasattr(os, "pidfd_open") and hasattr(signal, "pidfd_send_signal"), "pidfd APIs required")
        addresses = json.loads(self.run(["ip", "-j", "-4", "addr"]))
        base.require(any(a.get("local") == base.IPS[self.role] for row in addresses for a in row.get("addr_info", [])), "actual guest IP differs")
        base.safe(self.root, self.volume, exists=False)
        if self.root.exists():
            base.require(self.root.stat().st_uid == 0, "new fixture root must be root-owned")

    def state_paths(self):
        if self.role == "ctl":
            return [self.old_root / "state/moose/master"]
        if self.role == "b":
            return [self.old_root / "state/moose/chunkstate", self.old_root / "state/moose/chunks"]
        return []

    def old_state_idle(self):
        paths = self.state_paths()
        for path in paths:
            base.safe(path, self.old_root)
            base.require(path.is_dir() and path.stat().st_dev == self.volume.stat().st_dev, "reused state not on expected volume")
        references = []
        for proc in Path("/proc").iterdir():
            if not proc.name.isdigit() or int(proc.name) == os.getpid():
                continue
            try:
                argv = [a.decode(errors="replace") for a in (proc / "cmdline").read_bytes().split(b"\0") if a]
                if not argv:
                    continue
                executable = Path(os.readlink(proc / "exe")).name
                # Old stock config paths and either old/new configurations
                # referring to reused state are forbidden before this launch.
                hit = any(str(p) in arg for p in paths for arg in argv)
                for i, arg in enumerate(argv[:-1]):
                    if arg == "-c" and executable in ("mfsmaster", "mfschunkserver"):
                        cfg = Path(argv[i + 1])
                        if cfg.is_file():
                            text = cfg.read_text(errors="replace")
                            hit |= any(str(p) in text for p in paths)
                for link in [proc / "cwd", *(proc / "fd").iterdir()]:
                    try:
                        target = os.readlink(link)
                        hit |= any(target == str(p) or target.startswith(str(p) + "/") for p in paths)
                    except FileNotFoundError:
                        pass
                if hit:
                    references.append({"pid": int(proc.name), "argv": argv})
            except FileNotFoundError:
                continue  # exited during inventory, never signal/restart it
        base.require(not references, f"reused historical state still referenced: {references}")
        return {"reused_state_paths": list(map(str, paths)), "active_references": references}

    def capacity(self, admission=False):
        rows = json.loads(self.run(["findmnt", "-J", "--mountpoint", self.volume]))["filesystems"]
        base.require(len(rows) == 1 and rows[0]["target"] == str(self.volume) and rows[0]["fstype"] == "ext4", "dedicated ext4 volume required")
        used = 0
        seen = set()
        for root in [self.root, *self.state_paths()]:
            if not root.exists():
                continue
            base.safe(root, self.volume)
            for directory, directories, files in os.walk(root, followlinks=False):
                if Path(directory) == self.root:
                    directories[:] = [d for d in directories if d != "mount"]
                for entry in [Path(directory), *(Path(directory) / f for f in files)]:
                    st = entry.lstat()
                    base.require(not entry.is_symlink() and st.st_dev == self.volume.stat().st_dev, f"state escape: {entry}")
                    if (st.st_dev, st.st_ino) not in seen:
                        seen.add((st.st_dev, st.st_ino))
                        used += st.st_blocks * 512
                for name in directories:
                    entry = Path(directory) / name
                    base.require(not entry.is_symlink() and entry.stat().st_dev == self.volume.stat().st_dev, f"nested state escape: {entry}")
        info = os.statvfs(self.volume)
        free = info.f_bavail * info.f_frsize
        growth = GROWTH if admission else 0
        base.require(used + growth <= LIMITS[self.role], "new root plus reused state exceeds bounded budget")
        base.require(free - growth >= FLOORS[self.role], "projected free-space floor exceeded")
        base.require(free + used >= FLOORS[self.role] + LIMITS[self.role], "insufficient bounded fixture capacity")
        logs = sum(p.stat().st_size for relative in ("logs", "run") for p in (self.root / relative).rglob("*.log") if p.is_file())
        base.require(logs <= LOG_LIMIT, "fixture log limit exceeded; preserve and stop affected item")
        return {"mount": rows[0], "available_bytes": free, "owned_and_reused_allocated_bytes": used,
                "additional_growth_bytes": growth, "working_budget_bytes": LIMITS[self.role],
                "remaining_free_floor_bytes": FLOORS[self.role], "log_bytes": logs, "log_limit_bytes": LOG_LIMIT}

    def moose_configs(self):
        common = "WORKING_USER = root\nWORKING_GROUP = root\nNICE_LEVEL = 0\nDISABLE_OOM_KILLER = 0\n"
        if self.role == "ctl":
            return {"mfsmaster.cfg": common + f"DATA_PATH = {self.state_paths()[0]}\nEXPORTS_FILENAME = {self.root}/config/mfsexports.cfg\nMATOML_LISTEN_HOST = {base.IPS['ctl']}\nMATOML_LISTEN_PORT = 24940\nMATOCS_LISTEN_HOST = {base.IPS['ctl']}\nMATOCS_LISTEN_PORT = 24941\nMATOCL_LISTEN_HOST = {base.IPS['ctl']}\nMATOCL_LISTEN_PORT = 24942\nCHANGELOG_SAVE_MODE = 2\n",
                    "mfsexports.cfg": f"{CLIENT_IP} / rw,alldirs,admin,maproot=0:0\n"}
        if self.role == "b":
            return {"mfschunkserver.cfg": common + f"DATA_PATH = {self.state_paths()[0]}\nHDD_CONF_FILENAME = {self.root}/config/mfshdd.cfg\nMASTER_HOST = {base.IPS['ctl']}\nMASTER_PORT = 24941\nBIND_HOST = {base.IPS['b']}\nCSSERV_LISTEN_HOST = {base.IPS['b']}\nCSSERV_LISTEN_PORT = 24943\nHDD_LEAVE_SPACE_DEFAULT = 4GiB\nHDD_FSYNC_BEFORE_CLOSE = 1\n",
                    "mfshdd.cfg": str(self.state_paths()[1]) + "\n"}
        return {}

    def config_hashes(self):
        files = [WRAPPER, BASE, self.expectations_path, self.contract,
                 *(self.root / "config" / name for name in self.moose_configs())]
        return {str(p): base.digest(base.safe(p, Path("/"))) for p in files}

    def prepare(self):
        base.require(not (self.root / "run/config-prepared.json").exists(), "prepare is exclusive; preserve existing run")
        self.empty_ports()
        self.old_state_idle()
        for mount in (self.root / "mount/moose", self.old_root / "mount/moose"):
            base.require(self.exact_mount(mount) is None, "new or historical Moose mount active")
        capacity = self.capacity(admission=True)
        # Parent may have staged only fixed tools/expectations before prepare.
        if self.root.exists():
            base.require(set(p.name for p in self.root.iterdir()) <= {"tools"}, "unprepared root contains unexpected state")
        for relative in ("", "config", "run", "logs", "evidence", "mount", "mount/moose"):
            base.safe(self.root / relative, self.volume, exists=False).mkdir(exist_ok=True)
        for name, text in self.moose_configs().items():
            with (self.root / "config" / name).open("x") as out:
                out.write(text)
        value = {"role": self.role, "actual_guest_role": self.actual_role, "root": str(self.root), "config_sha256": self.config_hashes()}
        base.write_json(self.root / "run/config-prepared.json", value)
        return {"prepared": value, "capacity": capacity, "reused_state": list(map(str, self.state_paths()))}

    def admit(self):
        self.prepared()
        self.empty_ports()
        self.old_state_idle()
        base.require(self.exact_mount(self.root / "mount/moose") is None, "new mount already active")
        identities = {self.stock_name(): self.staged_stock()}
        if self.role == "a":
            helper = self.inspect_unmount()
            base.require(helper["sha256"] == self.expected["fusermount3_sha256"], "fixed normal unmount helper differs")
            identities["fusermount3"] = helper
        result = {"status": "PASS_CONFIG_ADMISSION_ONLY", "actual_guest_role": self.actual_role,
                  "identities": identities, "config_sha256": self.config_hashes(), "capacity": self.capacity(admission=True)}
        base.write_json(self.root / "run/config-admitted.json", result)
        return result

    def stock_name(self):
        return {"ctl": "mfsmaster", "b": "mfschunkserver", "a": "mfsmount"}[self.role]

    def staged_stock(self):
        path = base.safe(self.root / "tools" / self.stock_name(), self.root)
        # Trust the isolated executable and its complete mutable path. Never
        # modify or launch the historical uid501 installation.
        for component in (path, path.parent, self.root):
            st = component.stat()
            base.require(st.st_uid == 0 and not st.st_mode & 0o022, "untrusted staged stock permissions")
        base.require(path.is_file() and os.access(path, os.X_OK), "staged stock executable required")
        relative = ("bin/" if self.role == "a" else "sbin/") + self.stock_name()
        return self.check_elf(path, base.MOOSE_SHA[relative])

    def moose_argv(self):
        executable = str(self.root / "tools" / self.stock_name())
        if self.role != "a":
            return [executable, "-f", "-c", str(self.root / "config" / (self.stock_name() + ".cfg")), "start"]
        return [executable, "-f", "-H", base.IPS["ctl"], "-P", "24942", "-o",
                "allow_other,mfsnice=0,mfscachemode=DIRECT,mfstimeout=30", str(self.root / "mount/moose")]

    def validate_child(self, identity):
        observed = super().validate_child(identity)
        expected = (self.expected["mfsmount_sha256"] if self.role == "a" else
                    base.MOOSE_SHA["sbin/mfsmaster" if self.role == "ctl" else "sbin/mfschunkserver"])
        base.require(identity["exe_sha256"] == expected, "live stock child differs from fixed executable")
        return observed

    def start(self, timeout):
        self.old_state_idle()
        self.capacity(admission=True)
        # Recheck executable/helper dependencies at launch, not only admission.
        self.admit()
        return super().start(timeout)

    def supervisor_argv(self, lifecycle):
        return [sys.executable, str(WRAPPER), "__supervise", self.actual_role, "--expectations", str(self.expectations_path), "--lifecycle", str(lifecycle)]

    def receipt(self, action, value):
        return super().receipt(action, {"actual_guest_role": self.actual_role, "internal_lifecycle_role": self.role, **value})


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("prepare", "admit", "start", "stop", "postcheck", "__supervise"))
    parser.add_argument("role", choices=("ctl", "b", "c"))
    parser.add_argument("--expectations", required=True, type=Path)
    parser.add_argument("--timeout", type=float, default=30)
    parser.add_argument("--lifecycle", type=Path)
    args = parser.parse_args()
    fixture = None
    try:
        fixture = Fixture(args.role, args.expectations)
        fixture.guest()
        base.require(0 < args.timeout <= 60, "bounded timeout must be 0<seconds<=60")
        if args.command == "__supervise":
            base.require(args.lifecycle is not None, "internal lifecycle required")
            fixture.supervise(args.lifecycle)
            return
        result = {"prepare": fixture.prepare, "admit": fixture.admit,
                  "start": lambda: fixture.start(args.timeout), "stop": lambda: fixture.stop(args.timeout),
                  "postcheck": fixture.postcheck}[args.command]()
        fixture.receipt(args.command, result)
    except Exception as exc:
        value = {"status": "BLOCKED", "error": str(exc), "scope": "affected fixture only; preserve state, no repair"}
        if fixture is not None and (fixture.root / "evidence").is_dir():
            fixture.receipt(args.command, value)
        else:
            print(json.dumps(value), flush=True)
        raise SystemExit(1)


if __name__ == "__main__":
    main()
