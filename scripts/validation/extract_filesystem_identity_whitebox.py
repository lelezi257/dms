#!/usr/bin/env python3
"""从结构化日志提取 M1.3 inode 引用生命周期与持久回收事实。"""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any


SCHEMA = "dms.filesystem.identity-lifecycle-whitebox.v1"


def load_records(paths: list[Path]) -> list[dict[str, Any]]:
    records: list[dict[str, Any]] = []
    for path in paths:
        if not path.is_file():
            continue
        for line_number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
            try:
                record = json.loads(line)
            except json.JSONDecodeError:
                continue
            record["_source"] = path.name
            record["_line"] = line_number
            records.append(record)
    return records


def orphan_inode(workload: dict[str, Any]) -> int:
    for check in workload.get("checks", []):
        if check.get("operation") == "orphan_lifecycle_observed":
            inode = check.get("inode")
            if isinstance(inode, int) and inode > 1:
                return inode
    raise ValueError("workload does not contain the orphan inode")


def matching(records: list[dict[str, Any]], event: str, inode: int) -> list[dict[str, Any]]:
    return [
        record
        for record in records
        if record.get("event") == event and int(record.get("inode", -1)) == inode
    ]


def extract(result_dir: Path) -> dict[str, Any]:
    workload = json.loads((result_dir / "identity-workload.json").read_text(encoding="utf-8"))
    inode = orphan_inode(workload)
    node_records = load_records(
        [
            result_dir / "node-a.log",
            result_dir / "node-a-restarted.log",
        ]
    )
    meta_records = load_records(
        [
            result_dir / "meta.log",
            result_dir / "meta-restarted.log",
        ]
    )
    acquired = matching(node_records, "node.filesystem.reference.acquired", inode)
    released = matching(node_records, "node.filesystem.reference.released_final", inode)
    reaped = matching(meta_records, "meta.filesystem.orphan.reaped", inode)
    local_order = bool(
        acquired
        and released
        and acquired[0]["_source"] == released[-1]["_source"]
        and acquired[0]["_line"] < released[-1]["_line"]
    )
    return {
        "schema": SCHEMA,
        "inode": inode,
        "acquire_seen": bool(acquired),
        "final_release_seen": bool(released),
        "durable_reap_seen": bool(reaped),
        "node_local_order_verified": local_order,
        "reference_lifecycle_and_reap_seen": local_order and bool(reaped),
        "evidence": {
            "acquire": acquired[:1],
            "final_release": released[-1:],
            "durable_reap": reaped[:1],
        },
        "interpretation": (
            "同一 Node 日志证明 acquire 先于 final release；Meta 日志只在 WAL append+apply "
            "成功后记录 durable reap。该证据证明两个必要事实均出现，不宣称跨进程日志的"
            "全序关系；真正的先后约束由 Meta owner 的引用检查和 WAL 单元测试证明。"
        ),
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("result_dir", type=Path)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    evidence = extract(args.result_dir)
    rendered = json.dumps(evidence, ensure_ascii=False, indent=2) + "\n"
    output = args.output or args.result_dir / "identity-whitebox.json"
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(rendered, encoding="utf-8")
    print(rendered, end="")
    return 0 if evidence["reference_lifecycle_and_reap_seen"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
