import importlib.util
from pathlib import Path
import tempfile
import unittest


MODULE_PATH = Path(__file__).with_name("native_vs_moosefs_workload.py")
SPEC = importlib.util.spec_from_file_location("native_vs_moosefs_workload", MODULE_PATH)
assert SPEC and SPEC.loader
workload = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(workload)


class WorkloadTest(unittest.TestCase):
    def test_workspace_paths_and_patch_are_deterministic(self):
        root = Path("/mnt/test")
        self.assertEqual(
            workload.workspace_path(root, 4096, 9),
            root / "workspace/task-01/file-4096-0009.bin",
        )
        original = workload.workspace_bytes(4096, 9, 6701)
        patched = workload.workspace_bytes(4096, 9, 6701, patched=True)
        self.assertEqual(original[: workload.PATCH_OFFSET], patched[: workload.PATCH_OFFSET])
        self.assertNotEqual(
            original[workload.PATCH_OFFSET : workload.PATCH_OFFSET + workload.PATCH_LENGTH],
            patched[workload.PATCH_OFFSET : workload.PATCH_OFFSET + workload.PATCH_LENGTH],
        )

    def test_small_workspace_phases_preserve_expected_content(self):
        original_layout = workload.WORKSPACE_LAYOUT
        workload.WORKSPACE_LAYOUT = ((4096, 3), (65536, 2))
        try:
            with tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                workload.prepare_workspace_directories(root)
                for group in range(8):
                    self.assertTrue((root / "workspace" / f"task-{group:02d}").is_dir())
                self.assertTrue((root / "workspace" / "ephemeral").is_dir())
                create = workload.Recorder("create")
                workload.create_workspace(root, create, 7)
                self.assertEqual(len(create.samples), 5)

                read = workload.Recorder("read")
                workload.read_workspace(root, read, 7, "workspace.local_hot")
                self.assertEqual(len(read.samples), 5)

                patch = workload.Recorder("patch")
                workload.patch_workspace(root, patch, 7)
                self.assertEqual(len(patch.samples), 3)
                actual = workload.workspace_path(root, 4096, 1).read_bytes()
                self.assertEqual(actual, workload.workspace_bytes(4096, 1, 7, patched=True))
        finally:
            workload.WORKSPACE_LAYOUT = original_layout

    def test_recorder_reports_tail_latency_and_throughput(self):
        recorder = workload.Recorder("test")
        for _ in range(4):
            recorder.measure("case", 4, Path("x"), lambda: 4)
        summary = recorder.summary()["case"]
        self.assertEqual(summary["samples"], 4)
        self.assertGreater(summary["p99_us"], 0)
        self.assertGreater(summary["throughput_mib_s"], 0)


if __name__ == "__main__":
    unittest.main()
