#!/usr/bin/env python3
"""三 VM Agent workspace 小文件混合压力验收入口。

拓扑：A 执行 workspace mutation，B 立即按 reference model 验证跨节点可见性，
C 运行 Meta。这个脚本不调用 DMS SDK，只通过真实 FUSE mount 触发产品路径。
"""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import shlex
import time
from typing import Any


BASE_PATH = Path(__file__).with_name("run_filesystem_size_semantics_3vm.py")
BASE_SPEC = importlib.util.spec_from_file_location("filesystem_size_runner", BASE_PATH)
assert BASE_SPEC is not None and BASE_SPEC.loader is not None
BASE = importlib.util.module_from_spec(BASE_SPEC)
BASE_SPEC.loader.exec_module(BASE)

WORKLOAD_PATH = Path(__file__).with_name("filesystem_agent_workspace_workload.py")
WORKLOAD_SPEC = importlib.util.spec_from_file_location("filesystem_agent_workspace_workload", WORKLOAD_PATH)
assert WORKLOAD_SPEC is not None and WORKLOAD_SPEC.loader is not None
WORKLOAD = importlib.util.module_from_spec(WORKLOAD_SPEC)
WORKLOAD_SPEC.loader.exec_module(WORKLOAD)

EVALUATOR = Path(__file__).with_name("evaluate_m1_agent_workspace.py")


def _peer_file_verification_source(path: str, expected: bytes) -> str:
    """生成固定大小的远端校验程序，不把文件内容塞进 SSH 命令行。

    workspace 用例会生成少量 64 KiB 文件。若把期望内容做 base64 后嵌入
    ``python -c``，命令本身会超过 Lima SSH 的消息上限，测试失败发生在传输
    测试程序之前，根本没有验证 DMS。这里只传递 size 与 SHA-256；实际 bytes
    仍由 B 节点从 FUSE mount 读取，因此数据完整性语义没有减弱。
    """

    expected_digest = hashlib.sha256(expected).hexdigest()
    return (
        "import hashlib\nfrom pathlib import Path\n"
        f"path=Path({path!r})\n"
        f"expected_size={len(expected)}\n"
        f"expected_digest={expected_digest!r}\n"
        "actual=path.read_bytes()\n"
        "actual_digest=hashlib.sha256(actual).hexdigest()\n"
        "raise SystemExit(0 if len(actual) == expected_size and actual_digest == expected_digest "
        "else f'peer bytes mismatch for {path}: size={len(actual)}/{expected_size} ' "
        "+ f'sha256={actual_digest}/{expected_digest}')\n"
    )


def _parse_prometheus(text: str) -> dict[str, float]:
    metrics: dict[str, float] = {}
    for raw_line in text.splitlines():
        line = raw_line.strip()
        if not line or line.startswith("#"):
            continue
        parts = line.split()
        if len(parts) < 2:
            continue
        name = parts[0]
        try:
            value = float(parts[1])
        except ValueError:
            continue
        if name.startswith(("dms_node_", "dms_meta_")):
            metrics[name] = value
    return metrics


def _metrics_delta(before: dict[str, float], after: dict[str, float]) -> dict[str, float]:
    names = set(before) | set(after)
    return {
        name: after.get(name, 0.0) - before.get(name, 0.0)
        for name in sorted(names)
        if after.get(name, 0.0) - before.get(name, 0.0) != 0.0
    }


class Harness(BASE.Harness):
    def __init__(self, args: argparse.Namespace) -> None:
        super().__init__(args)
        self.remote = f"/tmp/dms-agent-workspace-{args.run_id}"
        self.mount = {role: f"{self.remote}/mnt" for role in ("A", "B")}
        self.output = args.output.resolve()
        self.model: dict[str, bytes] = {}
        self.plan: list[dict[str, Any]] = WORKLOAD.build_plan(args.seed, args.operations)
        self.latencies: list[dict[str, Any]] = []
        self.operation_counts: dict[str, int] = {}
        self.cross_node_verifications = 0
        self.metrics_before: dict[str, dict[str, float]] = {}
        self.metrics_after: dict[str, dict[str, float]] = {}

    def prepare(self) -> None:
        if self.output.exists():
            raise RuntimeError(f"output already exists: {self.output}")
        self.output.mkdir(parents=True)
        for binary in (self.args.dms_node, self.args.dms_meta):
            if not binary.is_file():
                raise RuntimeError(f"binary is missing: {binary}")
        profile = {
            "schema": "dms.filesystem.agent-workspace-3vm-profile.v1",
            "run_id": self.args.run_id,
            "seed": self.args.seed,
            "operations": self.args.operations,
            "vms": self.vms,
            "ips": self.ips,
            "artifacts": {
                "dms_node": {"path": str(self.args.dms_node), "sha256": BASE.digest(self.args.dms_node)},
                "dms_meta": {"path": str(self.args.dms_meta), "sha256": BASE.digest(self.args.dms_meta)},
            },
        }
        (self.output / "profile.json").write_text(json.dumps(profile, ensure_ascii=False, indent=2) + "\n")
        WORKLOAD.write_plan(self.output / "agent-workspace-plan.json", self.args.seed, self.args.operations, self.plan)

        for role in ("A", "B", "C"):
            self.shell(role, f"test ! -e {shlex.quote(self.remote)} && mkdir -p {shlex.quote(self.remote + '/bin')}")
        for role in ("A", "B"):
            self.copy_to(role, self.args.dms_node, self.remote + "/bin/dms-node")
            self.shell(role, f"chmod 755 {shlex.quote(self.remote + '/bin/dms-node')}")
            self.shell(role, f"mkdir -p {shlex.quote(self.mount[role])}")
        self.copy_to("C", self.args.dms_meta, self.remote + "/bin/dms-meta")
        self.shell("C", f"chmod 755 {shlex.quote(self.remote + '/bin/dms-meta')}; mkdir -p {shlex.quote(self.remote + '/journal')}")

    def _remote_python(self, role: str, source: str, *, capture: bool = False) -> str:
        result = self.shell(role, "python3 -c " + shlex.quote(source), capture=capture)
        return result.stdout if capture else ""

    def _rel(self, role: str, path: str) -> str:
        return f"{self.mount[role]}/{path}"

    def _mutate_on_a(self, operation: dict[str, Any]) -> None:
        op = operation["op"]
        if op in {"create", "overwrite"}:
            self._remote_python(
                "A",
                "import base64\nfrom pathlib import Path\n"
                f"path=Path({self._rel('A', operation['path'])!r})\n"
                "path.parent.mkdir(parents=True, exist_ok=True)\n"
                f"path.write_bytes(base64.b64decode({operation['data']!r}))\n",
            )
        elif op == "pwrite":
            self._remote_python(
                "A",
                "import base64, os\nfrom pathlib import Path\n"
                f"path=Path({self._rel('A', operation['path'])!r})\n"
                "path.parent.mkdir(parents=True, exist_ok=True)\n"
                "fd=os.open(path, os.O_RDWR)\n"
                "try:\n"
                f"    data=base64.b64decode({operation['data']!r})\n"
                f"    written=os.pwrite(fd, data, {int(operation['offset'])})\n"
                "    assert written == len(data)\n"
                "finally:\n"
                "    os.close(fd)\n",
            )
        elif op == "append":
            self._remote_python(
                "A",
                "import base64\nfrom pathlib import Path\n"
                f"path=Path({self._rel('A', operation['path'])!r})\n"
                "path.parent.mkdir(parents=True, exist_ok=True)\n"
                f"with path.open('ab') as stream: stream.write(base64.b64decode({operation['data']!r}))\n",
            )
        elif op == "rename":
            self._remote_python(
                "A",
                "import os\nfrom pathlib import Path\n"
                f"src=Path({self._rel('A', operation['src'])!r})\n"
                f"dst=Path({self._rel('A', operation['dst'])!r})\n"
                "dst.parent.mkdir(parents=True, exist_ok=True)\n"
                "os.replace(src, dst)\n",
            )
        elif op == "unlink":
            self._remote_python("A", "from pathlib import Path\n" f"Path({self._rel('A', operation['path'])!r}).unlink()\n")
        elif op in {"read", "stat", "readdir"}:
            return
        else:
            raise AssertionError(f"unknown operation: {op}")

    def _verify_peer_file(self, path: str) -> None:
        expected = self.model.get(path)
        if expected is None:
            self._remote_python(
                "B",
                "from pathlib import Path\n"
                f"path=Path({self._rel('B', path)!r})\n"
                "raise SystemExit(0 if not path.exists() else f'peer path should be absent: {path}')\n",
            )
        else:
            self._remote_python(
                "B",
                _peer_file_verification_source(self._rel("B", path), expected),
            )
        self.cross_node_verifications += 1

    def _verify_readdir(self, path: str) -> None:
        prefix = path.rstrip("/") + "/"
        expected = sorted(
            rel[len(prefix):].split("/", 1)[0]
            for rel in self.model
            if rel.startswith(prefix) and rel != prefix
        )
        expected = sorted(set(expected))
        self._remote_python(
            "B",
            "import json\nfrom pathlib import Path\n"
            f"path=Path({self._rel('B', path)!r})\n"
            f"expected={expected!r}\n"
            "actual=[] if not path.exists() else sorted(child.name for child in path.iterdir())\n"
            "raise SystemExit(0 if actual == expected else 'readdir mismatch ' + json.dumps({'actual': actual, 'expected': expected}))\n",
        )
        self.cross_node_verifications += 1

    def _tree_manifest(self, role: str) -> dict[str, dict[str, Any]]:
        source = (
            "import hashlib, json\n"
            "from pathlib import Path\n"
            f"root=Path({self._rel(role, 'workspace')!r})\n"
            "result={}\n"
            "if root.exists():\n"
            "    for path in sorted(item for item in root.rglob('*') if item.is_file()):\n"
            "        data=path.read_bytes()\n"
            "        result[path.relative_to(root.parent).as_posix()]={'size': len(data), 'sha256': hashlib.sha256(data).hexdigest()}\n"
            "print(json.dumps(result, sort_keys=True))\n"
        )
        return json.loads(self._remote_python(role, source, capture=True))

    def _snapshot_metrics(self, suffix: str) -> None:
        targets = (
            ("A", f"http://{self.ips['A']}:{self.args.node_health_port}/metrics", f"node-a-{suffix}.prom"),
            ("B", f"http://{self.ips['B']}:{self.args.node_health_port}/metrics", f"node-b-{suffix}.prom"),
            ("C", f"http://{self.ips['C']}:{self.args.meta_health_port}/metrics", f"meta-{suffix}.prom"),
        )
        for role, url, filename in targets:
            result = self.shell(role, f"curl -fsS {shlex.quote(url)}", capture=True)
            (self.output / filename).write_text(result.stdout, encoding="utf-8")
            parsed = _parse_prometheus(result.stdout)
            if suffix == "before":
                self.metrics_before[role] = parsed
            else:
                self.metrics_after[role] = parsed

    def _request_amplification_summary(self) -> dict[str, Any]:
        deltas = {
            role: _metrics_delta(self.metrics_before.get(role, {}), self.metrics_after.get(role, {}))
            for role in ("A", "B", "C")
        }
        interesting = {
            role: {
                name: value
                for name, value in values.items()
                if any(token in name for token in ("grpc", "filesystem", "fuse", "peer", "cache", "meta", "watch"))
            }
            for role, values in deltas.items()
        }
        return {
            "operation_count": len(self.plan),
            "metric_delta_series": sum(len(values) for values in deltas.values()),
            "interesting_deltas": interesting,
        }

    def run_workload(self) -> None:
        self._snapshot_metrics("before")
        for index, operation in enumerate(self.plan):
            started = time.perf_counter_ns()
            self._mutate_on_a(operation)
            WORKLOAD.apply_operation(self.model, operation)

            op = operation["op"]
            if op in {"create", "overwrite", "pwrite", "append", "read", "stat"}:
                self._verify_peer_file(operation["path"])
            elif op == "rename":
                self._verify_peer_file(operation["src"])
                self._verify_peer_file(operation["dst"])
            elif op == "unlink":
                self._verify_peer_file(operation["path"])
            elif op == "readdir":
                self._verify_readdir(operation["path"])
            else:
                raise AssertionError(f"unknown operation: {op}")

            elapsed = time.perf_counter_ns() - started
            self.latencies.append({"index": index, "operation": op, "elapsed_ns": elapsed})
            self.operation_counts[op] = self.operation_counts.get(op, 0) + 1

            if (index + 1) % self.args.full_verify_interval == 0:
                self._verify_full_tree()

        self._snapshot_metrics("after")
        self._write_result()

    def _verify_full_tree(self) -> None:
        expected = WORKLOAD.model_manifest(self.model)
        for role in ("A", "B"):
            actual = self._tree_manifest(role)
            if actual != expected:
                detail = {"role": role, "expected": expected, "actual": actual}
                raise AssertionError("full tree mismatch: " + json.dumps(detail, ensure_ascii=False)[:4000])

    def _write_result(self) -> None:
        self._verify_full_tree()
        model_manifest = WORKLOAD.model_manifest(self.model)
        tree_a = self._tree_manifest("A")
        tree_b = self._tree_manifest("B")
        result = {
            "schema": WORKLOAD.SCHEMA,
            "status": "passed",
            "deployment": "three-vm",
            "seed": self.args.seed,
            "operation_count": len(self.plan),
            "operation_counts": self.operation_counts,
            "cross_node_verifications": self.cross_node_verifications,
            "latency_summary": WORKLOAD.latency_summary([item["elapsed_ns"] for item in self.latencies]),
            "latency_by_operation": {
                op: WORKLOAD.latency_summary([item["elapsed_ns"] for item in self.latencies if item["operation"] == op])
                for op in sorted(self.operation_counts)
            },
            "model_digest": WORKLOAD.model_digest(self.model),
            "tree_digest_a": WORKLOAD.tree_digest(tree_a),
            "tree_digest_b": WORKLOAD.tree_digest(tree_b),
            "model_manifest": model_manifest,
            "tree_a": tree_a,
            "tree_b": tree_b,
            "request_amplification_summary": self._request_amplification_summary(),
            "latencies": self.latencies,
        }
        (self.output / "agent-workspace.json").write_text(
            json.dumps(result, ensure_ascii=False, indent=2) + "\n",
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
            BASE.command(["python3", str(EVALUATOR), str(self.output), "--output", str(self.output / "evaluation.json")])
            (self.output / "result.txt").write_text(
                "PASS\ndeployment=three-vm\nagent workspace stress verified\n",
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
    parser.add_argument("--run-id", type=BASE.validated_run_id, default=BASE.validated_run_id(f"agent-ws-{int(time.time())}"))
    parser.add_argument("--seed", type=int, default=20260916)
    parser.add_argument("--operations", type=int, default=WORKLOAD.DEFAULT_OPERATIONS)
    parser.add_argument("--full-verify-interval", type=int, default=30)
    parser.add_argument("--vm-a", default="g003-n1")
    parser.add_argument("--vm-b", default="g003-n2")
    parser.add_argument("--vm-c", default="g003-n3")
    parser.add_argument("--ip-a", default="192.168.104.8")
    parser.add_argument("--ip-b", default="192.168.104.9")
    parser.add_argument("--ip-c", default="192.168.104.10")
    parser.add_argument("--worker-port", type=int, default=30477)
    parser.add_argument("--node-health-port", type=int, default=30478)
    parser.add_argument("--meta-port", type=int, default=30577)
    parser.add_argument("--meta-health-port", type=int, default=30578)
    parser.add_argument("--log-level", default="warn")
    args = parser.parse_args()
    Harness(args).execute()
    print(args.output.resolve())
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
