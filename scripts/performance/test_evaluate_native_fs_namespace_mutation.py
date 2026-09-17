#!/usr/bin/env python3

import importlib.util
import unittest
from pathlib import Path


MODULE_PATH = Path(__file__).with_name("evaluate_native_fs_namespace_mutation.py")
SPEC = importlib.util.spec_from_file_location("evaluate_native_fs_namespace_mutation", MODULE_PATH)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)


def metric(method: str, count: float) -> dict[str, float]:
    return {
        f"A:dms_rpc_client_requests_total{{method={method},result=ok,service=FilesystemMetadataService}}": count
    }


def merge(*values: dict[str, float]) -> dict[str, float]:
    result: dict[str, float] = {}
    for value in values:
        result.update(value)
    return result


def operation_duration(operation: str, seconds: float) -> dict[str, float]:
    return {
        f"A:dms_node_filesystem_operation_duration_seconds_sum{{operation={operation}}}": seconds
    }


def rpc_duration(method: str, seconds: float) -> dict[str, float]:
    return {
        f"A:dms_rpc_client_duration_seconds_sum{{method={method},service=FilesystemMetadataService}}": seconds
    }


def case(samples: int, p50: float, whitebox=None, mean=None) -> dict:
    return {
        "samples": samples,
        "p50_us": p50,
        "mean_us": p50 if mean is None else mean,
        "p95_us": p50 * 1.2,
        "correctness": True,
        "whitebox": whitebox or {},
    }


def result(run_id: str, source_sha: str = "source", node_hash: str = "node") -> dict:
    dms = {
        "workspace.stat": case(100, 3.0),
        "workspace.create": case(
            100,
            800.0,
            merge(
                metric("CreateFilesystemInode", 100),
                metric("CommitFilesystemVersion", 100),
                metric("LookupFilesystemEntry", 100),
                metric("AcknowledgeNodeEvent", 200),
            ),
        ),
        "workspace.patch": case(
            100,
            500.0,
            merge(metric("CommitFilesystemVersion", 100), metric("AcknowledgeNodeEvent", 100)),
        ),
        "workspace.create_delete": case(
            100,
            1400.0,
            merge(
                metric("CreateFilesystemInode", 100),
                metric("CommitFilesystemVersion", 100),
                metric("RemoveFilesystemEntry", 100),
                metric("ReleaseFilesystemInodeReference", 100),
                metric("LookupFilesystemEntry", 100),
                metric("AcknowledgeNodeEvent", 300),
            ),
        ),
    }
    moosefs = {
        name: case(value["samples"], value["p50_us"] / 1.1)
        for name, value in dms.items()
    }
    return {
        "run_id": run_id,
        "environment": {
            "source_sha": source_sha,
            "resolved_hashes": {"dms_node": node_hash, "dms_meta": "meta"},
        },
        "lanes": {"memory": {"backends": {"dms": {"cases": dms}, "moosefs": {"cases": moosefs}}}},
    }


class NamespaceMutationEvaluatorTest(unittest.TestCase):
    def setUp(self):
        self.contract = MODULE.load_json(
            MODULE.ROOT / "benchmarks/whitebox/native-fs-namespace-mutation-contract.json"
        )

    def test_accepts_two_independent_runs_with_exact_rpc_budget(self):
        evaluation = MODULE.evaluate(self.contract, [result("one"), result("two")])
        self.assertEqual(evaluation["status"], "PASS", evaluation["errors"])

    def test_rejects_duplicate_run_or_binary_identity(self):
        evaluation = MODULE.evaluate(
            self.contract,
            [result("same"), result("same", node_hash="different")],
        )
        self.assertEqual(evaluation["status"], "FAIL")
        self.assertTrue(any("duplicate run_id" in error for error in evaluation["errors"]))
        self.assertTrue(any("identity differs" in error for error in evaluation["errors"]))

    def test_rejects_duplicate_resolve_and_xattr(self):
        one = result("one")
        create = one["lanes"]["memory"]["backends"]["dms"]["cases"]["workspace.create"]
        create["whitebox"].update(metric("GetFilesystemInode", 100))
        create["whitebox"].update(metric("GetFilesystemXattr", 100))
        evaluation = MODULE.evaluate(self.contract, [one, result("two")])
        self.assertEqual(evaluation["status"], "FAIL")
        self.assertTrue(any("GetFilesystemInode" in error for error in evaluation["errors"]))
        self.assertTrue(any("GetFilesystemXattr" in error for error in evaluation["errors"]))

    def test_rejects_extra_authoritative_publish(self):
        one = result("one")
        patch = one["lanes"]["memory"]["backends"]["dms"]["cases"]["workspace.patch"]
        patch["whitebox"].update(metric("CommitFilesystemVersion", 200))
        evaluation = MODULE.evaluate(self.contract, [one, result("two")])
        self.assertEqual(evaluation["status"], "FAIL")
        self.assertTrue(any("CommitFilesystemVersion per sample" in error for error in evaluation["errors"]))

    def test_rejects_slow_mutation_or_stat_meta_rpc(self):
        one = result("one")
        cases = one["lanes"]["memory"]["backends"]["dms"]["cases"]
        cases["workspace.patch"]["p50_us"] = 700.0
        cases["workspace.stat"]["whitebox"].update(metric("GetFilesystemInode", 1))
        evaluation = MODULE.evaluate(self.contract, [one, result("two")])
        self.assertEqual(evaluation["status"], "FAIL")
        self.assertTrue(any("P0 target" in error for error in evaluation["errors"]))
        self.assertTrue(any("workspace.stat" in error and "GetFilesystemInode" in error for error in evaluation["errors"]))

    def test_accepts_slow_mutation_with_complete_segmented_proof(self):
        one = result("one")
        two = result("two")
        for payload in (one, two):
            patch = payload["lanes"]["memory"]["backends"]["dms"]["cases"]["workspace.patch"]
            patch["p50_us"] = 900.0
            patch["mean_us"] = 920.0
            patch["whitebox"].update(
                merge(
                    operation_duration("open", 0.005),
                    operation_duration("write", 0.065),
                    operation_duration("sync", 0.005),
                    operation_duration("flush", 0.005),
                    operation_duration("close", 0.005),
                    operation_duration("getxattr", 0.0001),
                )
            )
        evaluation = MODULE.evaluate(self.contract, [one, two])
        self.assertEqual(evaluation["status"], "PASS", evaluation["errors"])
        self.assertGreaterEqual(
            evaluation["segmented_proofs"]["one/workspace.patch"]["coverage_fraction"],
            0.85,
        )

    def test_rejects_slow_mutation_with_low_coverage(self):
        one = result("one")
        patch = one["lanes"]["memory"]["backends"]["dms"]["cases"]["workspace.patch"]
        patch["p50_us"] = 900.0
        patch["mean_us"] = 920.0
        patch["whitebox"].update(
            merge(
                operation_duration("open", 0.001),
                operation_duration("write", 0.001),
                operation_duration("sync", 0.001),
                operation_duration("flush", 0.001),
                operation_duration("close", 0.001),
                operation_duration("getxattr", 0.0001),
            )
        )
        evaluation = MODULE.evaluate(self.contract, [one, result("two")])
        self.assertEqual(evaluation["status"], "FAIL")
        self.assertTrue(any("segmented proof coverage" in error for error in evaluation["errors"]))

    def test_segmented_proof_does_not_override_rpc_contract(self):
        one = result("one")
        two = result("two")
        for payload in (one, two):
            patch = payload["lanes"]["memory"]["backends"]["dms"]["cases"]["workspace.patch"]
            patch["p50_us"] = 900.0
            patch["mean_us"] = 920.0
            patch["whitebox"].update(
                merge(
                    operation_duration("open", 0.005),
                    operation_duration("write", 0.065),
                    operation_duration("sync", 0.005),
                    operation_duration("flush", 0.005),
                    operation_duration("close", 0.005),
                    operation_duration("getxattr", 0.0001),
                    metric("ResolveObject", 100),
                )
            )
        evaluation = MODULE.evaluate(self.contract, [one, two])
        self.assertEqual(evaluation["status"], "FAIL")
        self.assertTrue(any("forbidden foreground RPC ResolveObject" in error for error in evaluation["errors"]))


if __name__ == "__main__":
    unittest.main()
