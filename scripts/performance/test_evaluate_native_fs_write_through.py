#!/usr/bin/env python3

import importlib.util
import unittest
from pathlib import Path


MODULE_PATH = Path(__file__).with_name("evaluate_native_fs_write_through.py")
SPEC = importlib.util.spec_from_file_location("evaluate_native_fs_write_through", MODULE_PATH)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)


SIZES = ("4k", "64k", "1m", "8m")


def write_case(samples=10, throughput=100.0, callbacks=10.0, ack=0.0):
    rpc = {"CommitFilesystemVersion": callbacks}
    if ack:
        rpc["AcknowledgeNodeEvent"] = ack
    return {
        "correctness": True,
        "samples": samples,
        "p50_us": 100.0,
        "mean_us": 110.0,
        "throughput_mib_s": throughput,
        "segment_mean_us": {"open": 3.0, "pwrite": 80.0, "fdatasync": 25.0, "close": 2.0},
        "segment_coverage_fraction": 1.0,
        "rpc_counts": rpc,
        "whitebox": {"A:dms_node_fuse_callbacks_total{operation=write}": callbacks},
    }


def stable_case(p50=3.0):
    return {
        "correctness": True,
        "samples": 10,
        "p50_us": p50,
        "mean_us": p50,
        "throughput_mib_s": 100.0,
        "rpc_counts": {},
        "whitebox": {},
    }


def result(run_id: str, source="source", node_hash="node"):
    dms = {}
    mfs = {}
    for size in SIZES:
        dms[f"sync_write.no_holder.{size}"] = write_case(throughput=100.0)
        dms[f"sync_write.holder.{size}"] = write_case(throughput=90.0, ack=10.0)
        mfs[f"sync_write.no_holder.{size}"] = write_case(throughput=100.0)
        mfs[f"sync_write.holder.{size}"] = write_case(throughput=100.0)
        dms[f"stable_read.local.{size}"] = stable_case()
        dms[f"stable_read.peer.{size}"] = stable_case()
        mfs[f"stable_read.local.{size}"] = stable_case()
        mfs[f"stable_read.peer.{size}"] = stable_case()
    dms["sync_write.no_holder.512m_stream"] = write_case(
        samples=1, throughput=100.0, callbacks=512.0
    )
    mfs["sync_write.no_holder.512m_stream"] = write_case(
        samples=1, throughput=100.0, callbacks=512.0
    )
    dms["stable.stat.4k"] = stable_case()
    mfs["stable.stat.4k"] = stable_case()
    return {
        "schema": "dms.native-fs-write-through-result.v1",
        "run_id": run_id,
        "environment": {
            "source_sha": source,
            "resolved_hashes": {"dms_node": node_hash, "dms_meta": "meta"},
        },
        "semantics": {"operation": "open -> pwrite -> fdatasync -> close", "writeback": False},
        "backends": {
            "dms": {"correctness": True, "cases": dms},
            "moosefs": {"correctness": True, "cases": mfs},
        },
        "workload_matrix": [{"case": "x"}],
        "holder_cost": {"4k": {}},
        "write_stage_accounting": {"sync_write.no_holder.1m": {}},
        "recommendation_contract": {"architectural_reason": "x"},
    }


class WriteThroughEvaluatorTest(unittest.TestCase):
    def setUp(self):
        self.contract = MODULE.load_json(MODULE.DEFAULT_CONTRACT)

    def test_accepts_two_independent_exact_runs(self):
        evaluation = MODULE.evaluate(self.contract, [result("one"), result("two")])
        self.assertEqual(evaluation["status"], "PASS", evaluation["errors"])

    def test_segmented_proof_accepts_large_write_below_ratio(self):
        one = result("one")
        two = result("two")
        for payload in (one, two):
            payload["backends"]["dms"]["cases"]["sync_write.no_holder.1m"][
                "throughput_mib_s"
            ] = 70.0
        evaluation = MODULE.evaluate(self.contract, [one, two])
        self.assertEqual(evaluation["status"], "PASS", evaluation["errors"])
        self.assertEqual(
            evaluation["segmented_proofs"]["one/sync_write.no_holder.1m"]["status"],
            "PASS",
        )

    def test_low_coverage_cannot_excuse_slow_large_write(self):
        one = result("one")
        case = one["backends"]["dms"]["cases"]["sync_write.no_holder.1m"]
        case["throughput_mib_s"] = 70.0
        case["segment_coverage_fraction"] = 0.5
        evaluation = MODULE.evaluate(self.contract, [one, result("two")])
        self.assertEqual(evaluation["status"], "FAIL")
        self.assertTrue(any("segmented proof failed" in error for error in evaluation["errors"]))

    def test_rejects_extra_commit_or_missing_holder_ack(self):
        one = result("one")
        cases = one["backends"]["dms"]["cases"]
        cases["sync_write.no_holder.4k"]["rpc_counts"]["CommitFilesystemVersion"] = 20
        cases["sync_write.holder.4k"]["rpc_counts"].pop("AcknowledgeNodeEvent")
        evaluation = MODULE.evaluate(self.contract, [one, result("two")])
        self.assertEqual(evaluation["status"], "FAIL")
        self.assertTrue(any("callbacks" in error for error in evaluation["errors"]))
        self.assertTrue(any("ACK per logical write" in error for error in evaluation["errors"]))

    def test_rejects_stable_path_rpc_and_writeback(self):
        one = result("one")
        one["semantics"]["writeback"] = True
        one["backends"]["dms"]["cases"]["stable_read.local.4k"]["rpc_counts"] = {
            "GetFilesystemInode": 1
        }
        evaluation = MODULE.evaluate(self.contract, [one, result("two")])
        self.assertEqual(evaluation["status"], "FAIL")
        self.assertTrue(any("writeback" in error for error in evaluation["errors"]))
        self.assertTrue(any("stable path RPCs" in error for error in evaluation["errors"]))


if __name__ == "__main__":
    unittest.main()
