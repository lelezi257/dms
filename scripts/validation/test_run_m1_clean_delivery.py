#!/usr/bin/env python3
"""Clean-install runner 的轻量回归测试。"""

from __future__ import annotations

import importlib.util
import unittest
from pathlib import Path
from unittest import mock


MODULE_PATH = Path(__file__).with_name("run_m1_clean_delivery.py")
SPEC = importlib.util.spec_from_file_location("run_m1_clean_delivery", MODULE_PATH)
assert SPEC is not None and SPEC.loader is not None
clean = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(clean)


class CleanDeliveryTests(unittest.TestCase):
    def test_render_config_preserves_comments_and_overrides_existing_keys(self) -> None:
        example = "# comment\nDMS_NODE_ID=node-n1\nDMS_FUSE_MOUNTPOINT=\n"
        rendered = clean.render_config(
            example,
            {
                "DMS_NODE_ID": "node-a",
                "DMS_FUSE_MOUNTPOINT": "/mnt/dms",
                "DMS_META_ENDPOINT": "http://meta:19300",
            },
        )
        self.assertIn("# comment\n", rendered)
        self.assertIn("DMS_NODE_ID=node-a\n", rendered)
        self.assertIn("DMS_FUSE_MOUNTPOINT=/mnt/dms\n", rendered)
        self.assertIn("DMS_META_ENDPOINT=http://meta:19300\n", rendered)

    def test_safe_run_id_removes_shell_metacharacters(self) -> None:
        self.assertEqual(clean.safe_run_id("m1 clean;$HOME/.."), "m1-clean--HOME")

    def test_render_config_can_set_short_run_dir(self) -> None:
        rendered = clean.render_config("DMS_RUN_DIR=\n", {"DMS_RUN_DIR": "/tmp/dms-run-node-a"})
        self.assertEqual(rendered, "DMS_RUN_DIR=/tmp/dms-run-node-a\n")

    def test_host_path_in_vm_preserves_shared_relative_path(self) -> None:
        self.assertEqual(
            clean.host_path_in_vm(
                Path("/host/source/evidence/run"),
                Path("/host/source"),
                Path("/workspace/dms/source"),
            ),
            Path("/workspace/dms/source/evidence/run"),
        )

    def test_host_path_in_vm_rejects_unshared_path(self) -> None:
        with self.assertRaisesRegex(RuntimeError, "outside shared source root"):
            clean.host_path_in_vm(
                Path("/other/run"),
                Path("/host/source"),
                Path("/workspace/dms/source"),
            )

    def test_lima_cleanup_removes_only_current_run_paths(self) -> None:
        harness = clean.LimaHarness.__new__(clean.LimaHarness)
        harness.remote = "/tmp/dms-m1-clean-run-1"
        harness.output = Path("/tmp/dms-clean-delivery-test-output")
        harness.run_dir = {
            "A": "/tmp/dms-run-run-1-a",
            "B": "/tmp/dms-run-run-1-b",
            "C": "/tmp/dms-run-run-1-c",
        }
        harness.shell = mock.Mock(return_value=mock.Mock(returncode=0, stderr=""))

        harness.cleanup_workspace()

        self.assertEqual(3, harness.shell.call_count)
        for role, call in zip(("A", "B", "C"), harness.shell.call_args_list, strict=True):
            self.assertEqual(role, call.args[0])
            self.assertIn(harness.remote, call.args[1])
            self.assertIn(harness.run_dir[role], call.args[1])
            self.assertFalse(call.kwargs["check"])

    def test_lima_cleanup_failure_is_visible_evidence(self) -> None:
        with unittest.mock.patch.object(clean.Path, "write_text") as write_text:
            harness = clean.LimaHarness.__new__(clean.LimaHarness)
            harness.remote = "/tmp/dms-m1-clean-run-1"
            harness.output = Path("/tmp/dms-clean-delivery-test-output")
            harness.run_dir = {
                "A": "/tmp/dms-run-run-1-a",
                "B": "/tmp/dms-run-run-1-b",
                "C": "/tmp/dms-run-run-1-c",
            }
            harness.shell = mock.Mock(
                side_effect=[
                    mock.Mock(returncode=0, stderr=""),
                    mock.Mock(returncode=17, stderr="rm failed"),
                    mock.Mock(returncode=0, stderr=""),
                ]
            )

            harness.cleanup_workspace()

            rendered = write_text.call_args.args[0]
            self.assertIn("cleanup_workspace", rendered)
            self.assertIn("rm failed", rendered)


if __name__ == "__main__":
    unittest.main()
