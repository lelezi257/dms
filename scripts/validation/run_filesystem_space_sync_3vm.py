#!/usr/bin/env python3
"""三 VM 原生 Filesystem 空间管理与同步合同验证。

A 运行写入 Node，B 运行读取/接管 Node，C 运行 Meta。脚本复用已有三 VM 进程、
FUSE、制品复制和日志收集 Harness，只实现 M1.5 的业务步骤。
"""

from __future__ import annotations

import argparse
import importlib.util
import json
from pathlib import Path
import shlex
import time


ROOT = Path(__file__).resolve().parents[2]
BASE_PATH = Path(__file__).with_name("run_filesystem_size_semantics_3vm.py")
SPEC = importlib.util.spec_from_file_location("filesystem_size_3vm", BASE_PATH)
assert SPEC is not None and SPEC.loader is not None
BASE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(BASE)
EVALUATOR = Path(__file__).with_name("evaluate_filesystem_space_sync.py")
WORKLOAD = Path(__file__).with_name("filesystem_space_sync_workload.py")


class Harness(BASE.Harness):
    def __init__(self, args: argparse.Namespace) -> None:
        super().__init__(args)
        self.remote = f"/tmp/dms-filesystem-space-sync-{args.run_id}"
        self.mount = {role: f"{self.remote}/mnt" for role in ("A", "B")}

    def prepare(self) -> None:
        original = BASE.WORKLOAD
        try:
            BASE.WORKLOAD = WORKLOAD
            super().prepare()
        finally:
            BASE.WORKLOAD = original
        profile = json.loads((self.output / "profile.json").read_text(encoding="utf-8"))
        profile.update(
            {
                "schema": "dms.filesystem.space-sync-3vm-profile.v1",
                "arena_capacity_bytes": 8 * 1024 * 1024,
                "region_size_bytes": 4 * 1024 * 1024,
            }
        )
        (self.output / "profile.json").write_text(
            json.dumps(profile, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
        )

    def start_node(self, role: str) -> None:
        self.spawn(
            role,
            "node",
            [
                self.remote + "/bin/dms-node",
                "serve",
                "--node-id",
                f"{self.args.run_id}-node-{role.lower()}",
                "--meta-endpoint",
                f"http://{self.ips['C']}:{self.args.meta_port}",
                "--worker-tcp-address",
                f"{self.ips[role]}:{self.args.worker_port}",
                "--health-address",
                f"{self.ips[role]}:{self.args.node_health_port}",
                "--fuse-mountpoint",
                self.mount[role],
                "--arena-capacity-bytes",
                str(8 * 1024 * 1024),
                "--region-size-bytes",
                str(4 * 1024 * 1024),
                "--node-current-cache-bytes",
                str(2 * 1024 * 1024),
                "--log-level",
                "warn",
                "--tracing-enabled",
                "false",
            ],
        )
        self.wait(
            role,
            "Node ready",
            f"curl -fsS http://{self.ips[role]}:{self.args.node_health_port}/readyz >/dev/null",
        )
        self.wait(role, "FUSE mount", f"mountpoint -q {shlex.quote(self.mount[role])}")

    def python_json(self, role: str, source: str) -> dict:
        result = self.shell(
            role,
            "python3 -c " + shlex.quote(source),
            capture=True,
        )
        return json.loads(result.stdout.strip().splitlines()[-1])

    def metrics(self, role: str) -> str:
        port = self.args.meta_health_port if role == "C" else self.args.node_health_port
        return self.shell(
            role,
            f"curl -fsS http://{self.ips[role]}:{port}/metrics",
            capture=True,
        ).stdout

    @staticmethod
    def metric_value(text: str, name: str, labels: dict[str, str] | None = None) -> float:
        total = 0.0
        found = False
        for line in text.splitlines():
            if line.startswith("#") or not line.startswith(name):
                continue
            metric, value = line.rsplit(maxsplit=1)
            if metric != name and not metric.startswith(name + "{"):
                continue
            if labels and any(
                f'{key}="{label}"' not in metric for key, label in labels.items()
            ):
                continue
            total += float(value)
            found = True
        if not found:
            raise RuntimeError(f"metric is missing: {name} {labels}")
        return total

    def arena_snapshot(self, role: str = "A") -> dict[str, float]:
        text = self.metrics(role)
        return {
            "reserved_bytes": self.metric_value(text, "dms_node_arena_reserved_bytes"),
            "reservations": self.metric_value(text, "dms_node_arena_reservations"),
            "free_bytes": self.metric_value(text, "dms_node_arena_free_bytes"),
            "logical_bytes": self.metric_value(text, "dms_node_arena_logical_bytes"),
        }

    def run_workload(self) -> float:
        checks: list[dict] = []
        helper = self.remote + "/filesystem_size_semantics_workload.py"
        import_helper = (
            "import sys; "
            f"sys.path.insert(0, {self.remote!r}); "
            "import filesystem_size_semantics_workload as w\n"
        )

        extend_a = f"{self.mount['A']}/preallocate-extend.bin"
        extend_b = f"{self.mount['B']}/preallocate-extend.bin"
        self.python("A", f"from pathlib import Path\nPath({extend_a!r}).write_bytes(b'')")
        before = self.arena_snapshot()
        timing = self.python_json(
            "A",
            import_helper
            + "import json,time\nfrom pathlib import Path\n"
            + f"start=time.monotonic_ns(); w.fallocate(Path({extend_a!r}), 0, 0, w.RESERVATION_BYTES); "
            + "print(json.dumps({'latency_ms':(time.monotonic_ns()-start)/1e6}))",
        )
        after = self.arena_snapshot()
        self.remote_stat_size("B", extend_b, 1024 * 1024)
        self.remote_range_zero("B", extend_b, 0, 4096)
        checks.append(
            {
                "operation": "preallocate_extend",
                **timing,
                "arena_before": before,
                "arena_after": after,
            }
        )

        keep_a = f"{self.mount['A']}/preallocate-keep-size.bin"
        keep_b = f"{self.mount['B']}/preallocate-keep-size.bin"
        self.python("A", f"from pathlib import Path\nPath({keep_a!r}).write_bytes(b'seed')")
        before = self.arena_snapshot()
        timing = self.python_json(
            "A",
            import_helper
            + "import json,time\nfrom pathlib import Path\n"
            + f"start=time.monotonic_ns(); w.fallocate(Path({keep_a!r}), w.FALLOC_FL_KEEP_SIZE, w.KEEP_SIZE_OFFSET, w.KEEP_SIZE_BYTES); "
            + "print(json.dumps({'latency_ms':(time.monotonic_ns()-start)/1e6}))",
        )
        after = self.arena_snapshot()
        self.remote_stat_size("B", keep_b, 4)
        checks.append(
            {
                "operation": "preallocate_keep_size",
                **timing,
                "arena_before": before,
                "arena_after": after,
            }
        )

        before = self.arena_snapshot()
        timing = self.python_json(
            "A",
            "import json,os,time\n"
            + f"fd=os.open({extend_a!r}, os.O_RDWR); payload=b'R'*4096; start=time.monotonic_ns(); "
            + "written=os.pwrite(fd,payload,0); elapsed=(time.monotonic_ns()-start)/1e6; os.close(fd); "
            + "assert written == len(payload); print(json.dumps({'latency_ms':elapsed,'bytes':len(payload)}))",
        )
        after = self.arena_snapshot()
        self.remote_range_equals("B", extend_b, 0, b"R" * 4096)
        checks.append(
            {
                "operation": "write_consumes_reservation",
                **timing,
                "arena_before": before,
                "arena_after": after,
            }
        )

        timing = self.python_json(
            "A",
            import_helper
            + "import json,time\nfrom pathlib import Path\n"
            + f"start=time.monotonic_ns(); w.fallocate(Path({extend_a!r}), w.FALLOC_FL_PUNCH_HOLE|w.FALLOC_FL_KEEP_SIZE, 0, 4096); "
            + "print(json.dumps({'latency_ms':(time.monotonic_ns()-start)/1e6}))",
        )
        self.remote_stat_size("B", extend_b, 1024 * 1024)
        self.remote_range_zero("B", extend_b, 0, 4096)
        checks.append({"operation": "punch_hole", **timing})

        sync_a = f"{self.mount['A']}/sync-callbacks.bin"
        self.python("A", f"from pathlib import Path\nPath({sync_a!r}).write_bytes(b'sync')")
        meta_before = self.metrics("C")
        sync = self.python_json(
            "A",
            "import json,os,time\n"
            + f"path={sync_a!r}; mount={self.mount['A']!r}; fd=os.open(path,os.O_RDWR); dfd=os.open(mount,os.O_RDONLY|getattr(os,'O_DIRECTORY',0))\n"
            + "def timed(call):\n start=time.monotonic_ns(); call(); return (time.monotonic_ns()-start)/1e6\n"
            + "values={'flush':timed(lambda:os.close(os.dup(fd))),'fdatasync':timed(lambda:os.fdatasync(fd)),'fsync':timed(lambda:os.fsync(fd)),'fsyncdir':timed(lambda:os.fsync(dfd))}\n"
            + "os.close(fd);os.close(dfd);print(json.dumps(values))",
        )
        meta_after = self.metrics("C")
        labels = {"operation": "filesystem_commit_version", "result": "ok"}
        commit_delta = self.metric_value(
            meta_after, "dms_meta_operations_total", labels
        ) - self.metric_value(meta_before, "dms_meta_operations_total", labels)
        checks.append(
            {
                "operation": "sync_callbacks",
                "latency_ms": sync,
                "meta_commit_delta": commit_delta,
            }
        )

        modes = {}
        for name, flag in (("o_sync", "os.O_SYNC"), ("o_dsync", "getattr(os,'O_DSYNC',os.O_SYNC)")):
            path_a = f"{self.mount['A']}/{name}.bin"
            path_b = f"{self.mount['B']}/{name}.bin"
            payload = f"{name}-payload".encode()
            mode = self.python_json(
                "A",
                "import json,os,time\nfrom pathlib import Path\n"
                + f"path={path_a!r}; Path(path).write_bytes(b''); payload={payload!r}; fd=os.open(path,os.O_WRONLY|{flag}); "
                + "start=time.monotonic_ns(); written=os.write(fd,payload); elapsed=(time.monotonic_ns()-start)/1e6; os.close(fd); "
                + "assert written==len(payload); print(json.dumps({'latency_ms':elapsed,'bytes':len(payload)}))",
            )
            self.remote_read_equals("B", path_b, payload)
            modes[name] = mode
        checks.append({"operation": "sync_open_flags", "modes": modes})

        full_a = f"{self.mount['A']}/capacity-exhaustion.bin"
        self.python("A", f"from pathlib import Path\nPath({full_a!r}).write_bytes(b'')")
        before = self.arena_snapshot()
        capacity = self.python_json(
            "A",
            import_helper
            + "import errno,json,time\nfrom pathlib import Path\n"
            + "value=None; start=time.monotonic_ns()\n"
            + "try:\n"
            + f" w.fallocate(Path({full_a!r}), w.FALLOC_FL_KEEP_SIZE, 0, 64*1024*1024)\n"
            + "except OSError as error:\n value=error.errno\n"
            + "print(json.dumps({'errno':value,'latency_ms':(time.monotonic_ns()-start)/1e6}))",
        )
        after = self.arena_snapshot()
        if capacity["errno"] != 28 or before["reserved_bytes"] != after["reserved_bytes"]:
            raise RuntimeError(f"capacity failure is not atomic: {capacity} {before} {after}")
        checks.append(
            {
                "operation": "capacity_exhaustion",
                **capacity,
                "arena_before": before,
                "arena_after": after,
            }
        )

        owner_a = f"{self.mount['A']}/owner-recovery.bin"
        self.python("A", f"from pathlib import Path\nPath({owner_a!r}).write_bytes(b'')")
        self.python(
            "A",
            import_helper
            + "from pathlib import Path\n"
            + f"w.fallocate(Path({owner_a!r}),w.FALLOC_FL_KEEP_SIZE,w.OWNER_RECOVERY_OFFSET,w.OWNER_RECOVERY_BYTES)",
        )
        expected_reserved = self.arena_snapshot()["reserved_bytes"]
        result = {
            "schema": "dms.filesystem.space-sync-workload.v1",
            "deployment": "three-vm",
            "checks": checks,
            "owner_reserved_bytes_before_restart": expected_reserved,
        }
        (self.output / "space-sync-workload.json").write_text(
            json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
        )
        return expected_reserved

    def restart_and_verify(self, expected_reserved: float) -> None:
        helper_import = (
            "import sys; "
            f"sys.path.insert(0, {self.remote!r}); "
            "import filesystem_size_semantics_workload as w\n"
        )
        self.stop("C", "meta")
        self.start_meta()
        self.wait_meta_live_nodes(2)
        owner_a = f"{self.mount['A']}/owner-recovery.bin"
        before = self.arena_snapshot()
        self.python(
            "A",
            helper_import
            + "from pathlib import Path\n"
            + f"w.fallocate(Path({owner_a!r}),w.FALLOC_FL_KEEP_SIZE,w.OWNER_RECOVERY_OFFSET,w.OWNER_RECOVERY_BYTES)",
        )
        after = self.arena_snapshot()
        if before["reserved_bytes"] != expected_reserved or after != before:
            raise RuntimeError(f"Meta recovery changed reservation accounting: {before} {after}")
        (self.output / "space-sync-meta-recovery.json").write_text(
            json.dumps(
                {
                    "schema": "dms.filesystem.space-sync-meta-recovery.v1",
                    "reserved_bytes_before": before["reserved_bytes"],
                    "reserved_bytes_after": after["reserved_bytes"],
                },
                indent=2,
            )
            + "\n",
            encoding="utf-8",
        )

        self.stop("A", "node")
        self.shell(
            "A",
            f"fusermount3 -uz {shlex.quote(self.mount['A'])} 2>/dev/null || true",
            check=False,
        )
        self.start_node("A")
        self.wait_meta_live_nodes(2)
        owner_b = f"{self.mount['B']}/owner-recovery.bin"
        payload = b"new-owner-after-restart"
        self.python(
            "B",
            "import os\n"
            + f"fd=os.open({owner_b!r},os.O_RDWR); payload={payload!r}; written=os.pwrite(fd,payload,4*1024*1024); os.close(fd); assert written==len(payload)",
        )
        self.remote_range_equals("A", owner_a, 4 * 1024 * 1024, payload)
        (self.output / "space-sync-owner-recovery.json").write_text(
            json.dumps(
                {
                    "schema": "dms.filesystem.space-sync-owner-recovery.v1",
                    "path": "/owner-recovery.bin",
                    "offset": 4 * 1024 * 1024,
                    "bytes": len(payload),
                },
                indent=2,
            )
            + "\n",
            encoding="utf-8",
        )

    def execute(self) -> None:
        self.prepare()
        completed = False
        try:
            self.start_meta()
            self.start_node("A")
            self.start_node("B")
            self.wait_meta_live_nodes(2)
            expected_reserved = self.run_workload()
            self.snapshot_metrics()
            self.restart_and_verify(expected_reserved)
            self.snapshot_metrics_after_restart()
            BASE.command(
                [
                    "python3",
                    str(EVALUATOR),
                    str(self.output),
                    "--output",
                    str(self.output / "evaluation.json"),
                ]
            )
            (self.output / "result.txt").write_text(
                "PASS\ndeployment=three-vm\nspace and sync contract verified\n",
                encoding="utf-8",
            )
            completed = True
        finally:
            self.collect_logs()
            self.cleanup()
            if not completed:
                raise


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--dms-node", type=Path, required=True)
    parser.add_argument("--dms-meta", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument(
        "--run-id",
        type=BASE.validated_run_id,
        default=BASE.validated_run_id(f"space-sync-{int(time.time())}"),
    )
    parser.add_argument("--vm-a", default="g003-n1")
    parser.add_argument("--vm-b", default="g003-n2")
    parser.add_argument("--vm-c", default="g003-n3")
    parser.add_argument("--ip-a", default="192.168.104.8")
    parser.add_argument("--ip-b", default="192.168.104.9")
    parser.add_argument("--ip-c", default="192.168.104.10")
    parser.add_argument("--worker-port", type=int, default=30677)
    parser.add_argument("--node-health-port", type=int, default=30678)
    parser.add_argument("--meta-port", type=int, default=30777)
    parser.add_argument("--meta-health-port", type=int, default=30778)
    args = parser.parse_args()
    Harness(args).execute()
    print(args.output.resolve())
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
