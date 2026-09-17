#!/usr/bin/env python3
"""三 VM 跨 Node 文件锁、Meta 恢复与 Node epoch fencing 验收。"""

from __future__ import annotations

import argparse
import importlib.util
import json
from pathlib import Path
import shlex
import time


BASE_PATH = Path(__file__).with_name("run_filesystem_size_semantics_3vm.py")
SPEC = importlib.util.spec_from_file_location("filesystem_size_3vm", BASE_PATH)
assert SPEC is not None and SPEC.loader is not None
BASE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(BASE)
HELPER = Path(__file__).with_name("filesystem_lock_remote_helper.py")


class Harness(BASE.Harness):
    def __init__(self, args: argparse.Namespace) -> None:
        super().__init__(args)
        self.remote = f"/tmp/dms-filesystem-lock-{args.run_id}"
        self.mount = {role: f"{self.remote}/mnt" for role in ("A", "B")}
        self.checks: list[dict[str, object]] = []

    def prepare(self) -> None:
        super().prepare()
        for role in ("A", "B"):
            self.copy_to(role, HELPER, self.remote + "/filesystem_lock_remote_helper.py")
        profile = json.loads((self.output / "profile.json").read_text(encoding="utf-8"))
        profile["schema"] = "dms.filesystem.lock-3vm-profile.v1"
        (self.output / "profile.json").write_text(
            json.dumps(profile, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
        )

    def helper(self, role: str, arguments: list[str], *, capture: bool = False) -> dict | None:
        result = self.shell(
            role,
            "python3 " + shlex.join([self.remote + "/filesystem_lock_remote_helper.py", *arguments]),
            capture=capture,
        )
        if not capture:
            return None
        return json.loads(result.stdout.strip().splitlines()[-1])

    def spawn_helper(self, role: str, name: str, arguments: list[str]) -> None:
        self.spawn(
            role,
            name,
            ["python3", self.remote + "/filesystem_lock_remote_helper.py", *arguments],
        )

    def require_conflict(self, role: str, path: str) -> dict:
        result = self.helper(role, ["try", path], capture=True)
        assert result is not None
        if result != {"status": "error", "errno": 11} and result != {
            "status": "error",
            "errno": 13,
        }:
            raise RuntimeError(f"expected lock conflict on {role}, got {result}")
        return result

    def require_acquired(self, role: str, path: str) -> dict:
        result = self.helper(role, ["try", path], capture=True)
        assert result is not None
        if result != {"status": "acquired"}:
            raise RuntimeError(f"expected lock acquisition on {role}, got {result}")
        return result

    def start_holder(self, path: str, suffix: str) -> tuple[str, str]:
        ready = f"{self.remote}/holder-{suffix}.ready"
        release = f"{self.remote}/holder-{suffix}.release"
        self.spawn_helper(
            "A",
            f"holder-{suffix}",
            ["hold", path, "--ready", ready, "--release", release],
        )
        self.wait("A", f"holder {suffix}", f"test -f {shlex.quote(ready)}")
        return ready, release

    def wait_helper_exit(self, role: str, name: str) -> None:
        pid_file = f"{self.remote}/{name}.pid"
        self.wait(
            role,
            f"helper {name} exit",
            f"pid=$(sed -n '1p' {shlex.quote(pid_file)}); ! kill -0 \"$pid\" 2>/dev/null",
        )

    def run_workload(self) -> None:
        path_a = f"{self.mount['A']}/distributed-locks.bin"
        path_b = f"{self.mount['B']}/distributed-locks.bin"
        self.python("A", f"from pathlib import Path; Path({path_a!r}).write_bytes(b'x'*4096)")
        self.wait("B", "lock file visibility", f"test -f {shlex.quote(path_b)}")

        _, release = self.start_holder(path_a, "basic")
        conflict = self.require_conflict("B", path_b)
        query = self.helper("B", ["query", path_b], capture=True)
        if query is None or query.get("status") != "conflict" or query.get("length") != 0:
            raise RuntimeError(f"unexpected F_GETLK result: {query}")
        self.checks.append({"operation": "cross_node_conflict", "try": conflict, "query": query})

        waiting = f"{self.remote}/waiter.ready"
        acquired = f"{self.remote}/waiter.acquired"
        waiter_release = f"{self.remote}/waiter.release"
        self.spawn_helper(
            "B",
            "waiter",
            [
                "wait",
                path_b,
                "--ready",
                waiting,
                "--acquired",
                acquired,
                "--release",
                waiter_release,
            ],
        )
        self.wait("B", "blocking waiter", f"test -f {shlex.quote(waiting)}")
        if self.shell("B", f"test -e {shlex.quote(acquired)}", check=False).returncode == 0:
            raise RuntimeError("blocking waiter acquired before unlock")
        self.shell("A", f"touch {shlex.quote(release)}")
        self.wait("B", "blocking waiter wakeup", f"test -f {shlex.quote(acquired)}")
        self.shell("B", f"touch {shlex.quote(waiter_release)}")
        self.wait_helper_exit("A", "holder-basic")
        self.wait_helper_exit("B", "waiter")
        self.checks.append({"operation": "blocking_wakeup"})

        _, release = self.start_holder(path_a, "meta-restart")
        self.stop("C", "meta")
        self.start_meta()
        self.wait_meta_live_nodes(2)
        conflict = self.require_conflict("B", path_b)
        self.shell("A", f"touch {shlex.quote(release)}")
        self.wait_helper_exit("A", "holder-meta-restart")
        self.wait(
            "B",
            "unlock after Meta restart",
            "python3 "
            + shlex.join(
                [self.remote + "/filesystem_lock_remote_helper.py", "try", path_b]
            )
            + " | grep -q '\"status\": \"acquired\"'",
        )
        self.checks.append({"operation": "meta_restart_reclaim", "try": conflict})

        _, _release = self.start_holder(path_a, "node-restart")
        self.stop("A", "node")
        # 仅断连不是 unlock：旧 epoch 仍在 Meta 中，B 不能立刻取得锁。
        conflict = self.require_conflict("B", path_b)
        self.stop("A", "holder-node-restart")
        self.shell(
            "A",
            f"fusermount3 -uz {shlex.quote(self.mount['A'])} 2>/dev/null || true",
            check=False,
        )
        self.start_node("A")
        self.wait_meta_live_nodes(2)
        self.wait(
            "B",
            "lock release after Node epoch reclaim",
            "python3 "
            + shlex.join(
                [self.remote + "/filesystem_lock_remote_helper.py", "try", path_b]
            )
            + " | grep -q '\"status\": \"acquired\"'",
            timeout=20,
        )
        self.checks.append({"operation": "node_restart_epoch_fencing", "before_restart": conflict})

        (self.output / "distributed-locks-3vm.json").write_text(
            json.dumps(
                {
                    "schema": "dms.filesystem.distributed-locks-3vm.v1",
                    "status": "passed",
                    "checks": self.checks,
                },
                ensure_ascii=False,
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
            self.run_workload()
            self.snapshot_metrics()
            (self.output / "result.txt").write_text(
                "PASS\ndeployment=three-vm\ndistributed locks and recovery verified\n",
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
        default=BASE.validated_run_id(f"locks-{int(time.time())}"),
    )
    parser.add_argument("--vm-a", default="g003-n1")
    parser.add_argument("--vm-b", default="g003-n2")
    parser.add_argument("--vm-c", default="g003-n3")
    parser.add_argument("--ip-a", default="192.168.104.8")
    parser.add_argument("--ip-b", default="192.168.104.9")
    parser.add_argument("--ip-c", default="192.168.104.10")
    parser.add_argument("--worker-port", type=int, default=30877)
    parser.add_argument("--node-health-port", type=int, default=30878)
    parser.add_argument("--meta-port", type=int, default=30977)
    parser.add_argument("--meta-health-port", type=int, default=30978)
    parser.add_argument("--log-level", default="warn")
    args = parser.parse_args()
    Harness(args).execute()
    print(args.output.resolve())
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
