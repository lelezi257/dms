from __future__ import annotations

import tempfile
from pathlib import Path
import sys
import unittest


SCRIPT_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(SCRIPT_DIR))

import native_filesystem_workload as workload  # noqa: E402
import run_native_filesystem_vs_glue_3vm as harness  # noqa: E402


class NativeFilesystemWorkloadTest(unittest.TestCase):
    def test_payload_is_deterministic_and_patch_is_range_local(self):
        original = workload.expected_bytes(65536, 7, 6701, patched=False)
        patched = workload.expected_bytes(65536, 7, 6701, patched=True)
        self.assertEqual(original[: workload.MIDDLE_OFFSET], patched[: workload.MIDDLE_OFFSET])
        end = workload.MIDDLE_OFFSET + workload.MIDDLE_LENGTH
        self.assertEqual(original[end:], patched[end:])
        self.assertNotEqual(original[workload.MIDDLE_OFFSET:end], patched[workload.MIDDLE_OFFSET:end])

    def test_create_read_overwrite_and_verify(self):
        previous = workload.FILES_BY_SIZE
        workload.FILES_BY_SIZE = {4096: 2, 65536: 2, 1048576: 1}
        try:
            with tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                recorder = workload.Recorder("create")
                workload.create(root, recorder, 6701)
                self.assertEqual(5, len(recorder.samples))

                workload.read_all(root, workload.Recorder("read"), 6701, "local_hot_read", False)
                workload.overwrite(root, workload.Recorder("overwrite"), 6701)
                workload.read_all(
                    root,
                    workload.Recorder("verify"),
                    6701,
                    "remote_after_overwrite",
                    True,
                )
        finally:
            workload.FILES_BY_SIZE = previous

    def test_size_filter_keeps_only_requested_case(self):
        self.assertEqual([(65536, 50)], list(workload.selected_files(65536)))
        self.assertEqual(3, len(list(workload.selected_files(None))))

    def test_metadata_open_and_readdir_cases(self):
        previous = workload.FILES_BY_SIZE
        workload.FILES_BY_SIZE = {4096: 2}
        try:
            with tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                workload.create(root, workload.Recorder("create"), 6701)

                metadata = workload.Recorder("metadata-hot")
                workload.metadata_hot(root, metadata, 4096)
                self.assertEqual(2, len(metadata.samples))

                opened = workload.Recorder("open-close")
                workload.open_close(root, opened, 4096)
                self.assertEqual(2, len(opened.samples))

                directory_reads = workload.Recorder("readdir")
                workload.readdir(root, directory_reads)
                self.assertEqual(40, len(directory_reads.samples))
        finally:
            workload.FILES_BY_SIZE = previous

    def test_harness_case_ids_are_stable_and_filterable(self):
        self.assertEqual("local_hot_read.4096", harness.workload_case_id("local-hot", 4096))
        self.assertEqual("readdir.root", harness.workload_case_id("readdir", None))


if __name__ == "__main__":
    unittest.main()
