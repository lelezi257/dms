#!/usr/bin/env python3
"""在三台 VM 上运行 P3 等语义同步写与稳定读取基准。

本脚本复用既有 DMS/MooseFS 部署、进程采样和 metrics 采集能力，只替换
workload 与 case 编排。每轮交替后端启动顺序，避免固定的先后顺序偏差。
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import shlex

from run_native_vs_moosefs_3vm import Harness


ROOT = Path(__file__).resolve().parents[2]
WORKLOAD = ROOT / "scripts/performance/native_fs_write_through_workload.py"
MEASURED_PHASES = (
    *((f"write-no-holder-{size}", "A") for size in ("4k", "64k", "1m", "8m")),
    *((f"write-holder-{size}", "A") for size in ("4k", "64k", "1m", "8m")),
    ("stat-4k", "A"),
    *((f"local-read-{size}", "A") for size in ("4k", "64k", "1m", "8m")),
    *((f"peer-repeat-{size}", "B") for size in ("4k", "64k", "1m", "8m")),
    ("sync-stream-512m", "A"),
)


class WriteThroughHarness(Harness):
    """P3 case 编排；底层部署与采样继续使用已经验证的公共 Harness。"""

    def prepare(self) -> None:
        super().prepare()
        for role in ("A", "B", "C"):
            self.copy_to(role, WORKLOAD, self.remote_base + "/workload.py")

    def run_workload(
        self,
        lane: str,
        round_id: int,
        backend: str,
        phase: str,
        role: str,
        destination: Path,
    ) -> None:
        root = self.experiment_root(lane, round_id, backend, role)
        remote_result = root + f"/{phase}.json"
        self.shell(
            role,
            shlex.join(
                [
                    "python3",
                    self.remote_base + "/workload.py",
                    "--root",
                    root + "/mnt",
                    "--phase",
                    phase,
                    "--seed",
                    "7703",
                    "--output",
                    remote_result,
                ]
            ),
        )
        self.copy_from(role, remote_result, destination)

    def run_cases(self, lane: str, round_id: int, backend: str) -> None:
        round_dir = self.output / lane / f"round-{round_id}" / backend
        setup_dir = round_dir / "setup"
        setup_dir.mkdir(parents=True)

        # A 创建数据。holder 授权在每个对应写 case 紧前方单独建立，避免长轮次
        # 中租约自然到期，把“有远端 holder”误测成“没有 holder”。
        self.run_workload(lane, round_id, backend, "prepare", "A", setup_dir / "prepare.json")

        peer_warmed = False
        for phase, role in MEASURED_PHASES:
            if phase.startswith("write-holder-"):
                size = phase.rsplit("-", 1)[-1]
                warm_phase = f"holder-warm-{size}"
                self.run_workload(
                    lane,
                    round_id,
                    backend,
                    warm_phase,
                    "B",
                    setup_dir / f"{warm_phase}.json",
                )
            if phase.startswith("peer-repeat-") and not peer_warmed:
                self.run_workload(
                    lane, round_id, backend, "peer-warm", "B", setup_dir / "peer-warm.json"
                )
                peer_warmed = True
            case_dir = round_dir / phase
            case_dir.mkdir(parents=True)
            self.snapshot(lane, round_id, backend, case_dir, "before")
            self.run_workload(
                lane, round_id, backend, phase, role, case_dir / "workload.json"
            )
            self.snapshot(lane, round_id, backend, case_dir, "after")

    def execute(self) -> None:
        rounds = int(self.profile.get("lane_rounds", {}).get("memory", 5))
        orders = (("dms", "moosefs"), ("moosefs", "dms"))
        try:
            self.prepare()
            for round_id in range(rounds):
                for backend in orders[round_id % 2]:
                    try:
                        if backend == "dms":
                            self.start_dms("memory", round_id)
                        else:
                            self.start_moosefs("memory", round_id)
                        self.run_cases("memory", round_id, backend)
                    finally:
                        self.cleanup("memory", round_id, backend)
        finally:
            self.collect_logs()
            for role in ("A", "B", "C"):
                self.shell(
                    role,
                    f"findmnt -rn -o TARGET | grep '^{shlex.quote(self.remote_base)}' | "
                    "sort -r | while read -r target; do sudo umount -l \"$target\" 2>/dev/null || true; done; "
                    f"rm -rf -- {shlex.quote(self.remote_base)}",
                )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--profile", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    profile = json.loads(args.profile.read_text(encoding="utf-8"))
    harness = WriteThroughHarness(profile, args.output.resolve())
    harness.execute()
    print(json.dumps({"ok": True, "output": str(harness.output)}, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
