from __future__ import annotations

import errno
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import Mock

SCRIPT_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(SCRIPT_DIR))

import filesystem_namespace_workload as workload


class FilesystemNamespaceWorkloadTests(unittest.TestCase):
    def test_payload_is_deterministic_per_round(self) -> None:
        self.assertEqual(
            workload.deterministic_payload(3, "anchor"),
            workload.deterministic_payload(3, "anchor"),
        )
        self.assertNotEqual(
            workload.deterministic_payload(3, "anchor"),
            workload.deterministic_payload(4, "anchor"),
        )

    def test_assert_errno_accepts_expected_error_only(self) -> None:
        workload.assert_errno(lambda: (_ for _ in ()).throw(OSError(errno.ENOTEMPTY, "x")), errno.ENOTEMPTY, "case")
        with self.assertRaises(AssertionError):
            workload.assert_errno(lambda: None, errno.ENOTEMPTY, "case")

    def test_missing_remote_directory_means_not_yet_visible(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            self.assertEqual(workload.names(Path(directory) / "not-yet-visible"), set())

    def test_recovery_read_retries_temporary_unreachable_replica(self) -> None:
        path = Mock()
        path.exists.return_value = True
        path.read_bytes.side_effect = OSError(errno.EHOSTUNREACH, "replica lease pending")
        self.assertFalse(workload.read_matches_during_recovery(path, b"payload"))

        path.read_bytes.side_effect = OSError(errno.EIO, "corrupt payload")
        with self.assertRaises(OSError):
            workload.read_matches_during_recovery(path, b"payload")

    def test_single_round_uses_plain_posix_semantics(self) -> None:
        with tempfile.TemporaryDirectory() as left, tempfile.TemporaryDirectory() as right:
            # 单进程目录只能验证 workload 自身的 POSIX 操作顺序。真正的跨 Node 可见性
            # 由 run_filesystem_namespace_e2e.sh 在两个 FUSE mount 上执行同一函数。
            mount_a = Path(left)
            mount_b = Path(right)
            # 用 copytree 模拟 B 在每个可见性等待点最终看到 A 的变化，避免单元测试依赖 FUSE。
            original_wait = workload.wait_until

            def mirrored_wait(name, deadline_seconds, check):
                if mount_b.exists():
                    for child in mount_b.iterdir():
                        if child.is_dir():
                            import shutil

                            shutil.rmtree(child)
                        else:
                            child.unlink()
                import shutil

                for child in mount_a.iterdir():
                    target = mount_b / child.name
                    if child.is_dir():
                        shutil.copytree(child, target)
                    else:
                        target.write_bytes(child.read_bytes())
                return original_wait(name, deadline_seconds, check)

            workload.wait_until = mirrored_wait
            try:
                result = workload.run_round(mount_a, mount_b, 1)
            finally:
                workload.wait_until = original_wait
            operations = {check["operation"] for check in result["checks"]}
            self.assertEqual(workload.REQUIRED_OPERATIONS, operations)


if __name__ == "__main__":
    unittest.main()
