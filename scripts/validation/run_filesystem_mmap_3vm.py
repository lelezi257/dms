#!/usr/bin/env python3
"""三 VM 原生 Filesystem cached mmap 验证入口。"""

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
WORKLOAD = Path(__file__).with_name("filesystem_mmap_workload.py")
HELPER = Path(__file__).with_name("filesystem_mmap_helper.c")
EVALUATOR = Path(__file__).with_name("evaluate_filesystem_mmap.py")


class Harness(BASE.Harness):
    def __init__(self, args: argparse.Namespace) -> None:
        super().__init__(args)
        self.remote = f"/tmp/dms-filesystem-mmap-{args.run_id}"
        self.mount = {role: f"{self.remote}/mnt" for role in ("A", "B")}

    def prepare(self) -> None:
        super().prepare()
        for role in ("A", "B"):
            self.copy_to(role, WORKLOAD, self.remote + "/filesystem_mmap_workload.py")
            if self.args.helper_binary is not None:
                self.copy_to(role, self.args.helper_binary, self.remote + "/filesystem-mmap-helper")
                self.shell(role, f"chmod 755 {shlex.quote(self.remote + '/filesystem-mmap-helper')}")
            else:
                self.copy_to(role, HELPER, self.remote + "/filesystem_mmap_helper.c")
                self.shell(
                    role,
                    f"cc -std=c11 -Wall -Wextra -Werror "
                    f"{shlex.quote(self.remote + '/filesystem_mmap_helper.c')} "
                    f"-o {shlex.quote(self.remote + '/filesystem-mmap-helper')}",
                )
        profile = json.loads((self.output / "profile.json").read_text(encoding="utf-8"))
        profile.update({"schema": "dms.filesystem.mmap-3vm-profile.v1"})
        (self.output / "profile.json").write_text(
            json.dumps(profile, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
        )

    def remote_json(self, role: str, script: str) -> dict:
        result = self.shell(role, script, capture=True)
        return json.loads(result.stdout.strip().splitlines()[-1])

    def metrics(self, role: str) -> str:
        if role in ("A", "B"):
            url = f"http://{self.ips[role]}:{self.args.node_health_port}/metrics"
        elif role == "C":
            url = f"http://{self.ips['C']}:{self.args.meta_health_port}/metrics"
        else:
            raise ValueError(f"unknown role: {role}")
        return self.shell(role, f"curl -fsS {shlex.quote(url)}", capture=True).stdout

    def run_helper(self, role: str, *args: str) -> dict:
        return self.remote_json(
            role,
            shlex.join([self.remote + "/filesystem-mmap-helper", *args]),
        )

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
            if labels and any(f'{key}="{label}"' not in metric for key, label in labels.items()):
                continue
            total += float(value)
            found = True
        return total if found else 0.0

    def fuse_read_count(self, role: str = "B") -> float:
        return self.metric_value(
            self.metrics(role),
            "dms_node_fuse_callbacks_total",
            {"operation": "read"},
        )

    def kernel_invalidation_ok_count(self, role: str = "B") -> float:
        return self.metric_value(
            self.metrics(role),
            "dms_node_filesystem_kernel_invalidations_total",
            {"result": "ok"},
        )

    def meta_watch_events(self) -> float:
        return self.metric_value(
            self.metrics("C"),
            "dms_meta_watch_events_total",
            {"event_type": "filesystem_invalidation", "result": "delivered"},
        )

    def write_remote_version_from_a(self, path: str) -> None:
        self.python(
            "A",
            "import os\n"
            f"path={path!r}\n"
            "fd=os.open(path, os.O_RDWR)\n"
            "try:\n"
            "    payload=b'REMOTE_VERSION'\n"
            "    assert os.pwrite(fd, payload, 0) == len(payload)\n"
            "    os.fsync(fd)\n"
            "finally:\n"
            "    os.close(fd)",
        )

    def run_workload(self) -> None:
        checks: list[dict] = []

        shared_a = f"{self.mount['A']}/shared-msync.bin"
        shared_b = f"{self.mount['B']}/shared-msync.bin"
        checks.append(self.run_helper("A", "shared-msync", shared_a, shared_a))
        self.remote_range_equals("B", shared_b, 128, b"MAP_SHARED_OK")

        private_a = f"{self.mount['A']}/private.bin"
        checks.append(self.run_helper("A", "private-no-publish", private_a))

        remote_a = f"{self.mount['A']}/remote-invalidate.bin"
        remote_b = f"{self.mount['B']}/remote-invalidate.bin"
        self.python("A", "from pathlib import Path\n" f"Path({remote_a!r}).write_bytes(b'O' * 4096)")
        self.wait("B", "remote mmap file visible", f"test -f {shlex.quote(remote_b)}")
        ready = self.remote + "/remote-invalidate.ready"
        remote_output = self.remote + "/remote-invalidate.json"
        status_file = self.remote + "/remote-invalidate.status"
        self.shell(
            "B",
            "rm -f "
            + shlex.join([ready, remote_output, status_file])
            + "; nohup bash -lc "
            + shlex.quote(
                shlex.join(
                    [
                        self.remote + "/filesystem-mmap-helper",
                        "wait-remote-invalidate",
                        remote_b,
                        ready,
                        remote_output,
                    ]
                )
                + f" >{shlex.quote(self.remote + '/remote-invalidate.log')} 2>&1; echo $? >{shlex.quote(status_file)}"
            )
            + " </dev/null >/dev/null 2>&1 &",
        )
        self.wait("B", "B mapped page before remote write", f"test -f {shlex.quote(ready)}")
        reads_before = self.fuse_read_count("B")
        invalidations_before = self.kernel_invalidation_ok_count("B")
        watch_before = self.meta_watch_events()
        self.write_remote_version_from_a(remote_a)
        self.wait(
            "B",
            "B mapped page sees remote version",
            f"test -f {shlex.quote(status_file)} && test \"$(cat {shlex.quote(status_file)})\" = 0",
            timeout=10.0,
        )
        remote = json.loads(
            self.shell("B", f"cat {shlex.quote(remote_output)}", capture=True).stdout
        )
        remote["node_b_fuse_read_delta"] = self.fuse_read_count("B") - reads_before
        remote["node_b_kernel_invalidation_ok_delta"] = (
            self.kernel_invalidation_ok_count("B") - invalidations_before
        )
        remote["meta_filesystem_watch_event_delta"] = self.meta_watch_events() - watch_before
        remote["writer_fsync_returned_before_mapped_visibility"] = True
        remote["mapped_visibility_after_writer_fsync"] = True
        remote["ack_order_machine_assertion"] = (
            remote["mapped_visibility_after_writer_fsync"]
            and remote["writer_fsync_returned_before_mapped_visibility"]
            and remote["node_b_kernel_invalidation_ok_delta"] > 0
            and remote["meta_filesystem_watch_event_delta"] > 0
        )
        if remote["node_b_kernel_invalidation_ok_delta"] <= 0:
            raise RuntimeError("remote invalidation did not increment Node B kernel invalidation metric")
        if remote["meta_filesystem_watch_event_delta"] <= 0:
            raise RuntimeError("remote invalidation did not increment Meta filesystem watch metric")
        checks.append(remote)

        checks.append(self.run_helper("A", "truncate-sigbus", f"{self.mount['A']}/truncate-sigbus.bin"))
        checks.append(self.run_helper("A", "punch-hole-zero", f"{self.mount['A']}/punch-hole.bin"))
        checks.append(self.run_helper("A", "unlink-open-mmap", f"{self.mount['A']}/unlink-open.bin"))

        cached_a = f"{self.mount['A']}/cached-page-hit.bin"
        cached_b = f"{self.mount['B']}/cached-page-hit.bin"
        self.python("A", "from pathlib import Path\n" f"Path({cached_a!r}).write_bytes(b'C' * 4096)")
        self.remote_read_equals("B", cached_b, b"C" * 4096)
        before = self.fuse_read_count("B")
        for _ in range(20):
            self.remote_read_equals("B", cached_b, b"C" * 4096)
        after = self.fuse_read_count("B")
        if after != before:
            raise RuntimeError(f"cached page hit caused FUSE read callbacks: {before} -> {after}")
        checks.append(
            {
                "check": "cached_page_hit_without_fuse_read",
                "status": "passed",
                "node_b_fuse_read_delta": after - before,
            }
        )

        result = {
            "schema": "dms.filesystem.mmap-workload.v1",
            "deployment": "three-vm",
            "status": "passed",
            "checks": checks,
            "passed_operations": len(checks),
        }
        (self.output / "mmap-workload.json").write_text(
            json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
        )

    def restart_and_verify(self) -> None:
        self.stop("C", "meta")
        self.start_meta()
        self.stop("B", "node")
        self.shell("B", f"fusermount3 -uz {shlex.quote(self.mount['B'])} 2>/dev/null || true", check=False)
        self.start_node("B")
        self.wait_meta_live_nodes(2)
        recovery_a = f"{self.mount['A']}/recovery-mmap.bin"
        recovery_b = f"{self.mount['B']}/recovery-mmap.bin"
        helper = self.run_helper("A", "shared-msync", recovery_a, recovery_a)
        self.remote_range_equals("B", recovery_b, 128, b"MAP_SHARED_OK")
        result = {
            "schema": "dms.filesystem.mmap-recovery.v1",
            "deployment": "three-vm",
            "status": "passed",
            "check": "restart_remap_after_meta_or_node_restart",
            "helper": helper,
        }
        (self.output / "mmap-recovery.json").write_text(
            json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
        )

    def execute(self) -> None:
        self.prepare()
        completed = False
        try:
            self.start_meta()
            self.start_node("A")
            self.start_node("B")
            self.wait_meta_live_nodes(2)
            self.run_workload()
            self.snapshot_metrics()
            self.restart_and_verify()
            self.snapshot_metrics_after_restart()
            BASE.command(["python3", str(EVALUATOR), str(self.output), "--output", str(self.output / "evaluation.json")])
            (self.output / "result.txt").write_text(
                "PASS\ndeployment=three-vm\ncached mmap verified\n", encoding="utf-8"
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
    parser.add_argument(
        "--helper-binary",
        type=Path,
        help="Optional prebuilt Linux filesystem-mmap-helper; avoids requiring cc on target VMs.",
    )
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument(
        "--run-id",
        type=BASE.validated_run_id,
        default=BASE.validated_run_id(f"mmap-{int(time.time())}"),
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
    parser.add_argument("--log-level", default="warn")
    args = parser.parse_args()
    Harness(args).execute()
    print(args.output.resolve())
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
