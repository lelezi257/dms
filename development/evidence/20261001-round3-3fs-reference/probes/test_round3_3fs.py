#!/usr/bin/env python3
"""Focused unit coverage for round3-3fs helper control-flow contracts."""
from __future__ import annotations

import errno
import importlib.util
import pathlib
import tempfile
import unittest
from unittest import mock


SCRIPT = pathlib.Path(__file__).with_name("round3-3fs.py")
SPEC = importlib.util.spec_from_file_location("round3_3fs", SCRIPT)
round3 = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(round3)


def make_templates(path: pathlib.Path) -> None:
    files = {
        "admin_cli.toml": "cluster_id = 'afs_3fs_patched_qualify32'\n[fdb]\nclusterFile = '/old/data/foundationdb/fdb.cluster'\n[user_info]\ntoken = 'old'\n",
        "hf3fs_fuse_main.toml": "fsync_length_hint = true\n[[common.log.handlers]]\nfile_path = '/old/log/fuse.log'\n[mgmtd]\nmgmtd_server_addresses = [ 'RDMA://192.168.109.12:19001' ]\n",
        "hf3fs_fuse_main_launcher.toml": "token_file = '/old/config/token'\ncluster_id = 'afs_3fs_patched_qualify32'\n",
        "storage_main.toml": "[[server.base.groups]]\n[server.base.groups.listener]\nlisten_port = 19003\n[server.targets]\ntarget_paths = [ '/old/data/storage/data1' ]\n",
        "storage_main_app.toml": "node_id = 10000\n",
        "storage_main_launcher.toml": "cluster_id = 'afs_3fs_patched_qualify32'\n",
        "mgmtd_main.toml": "[[server.base.groups]]\n[server.base.groups.listener]\nlisten_port = 19001\n",
        "mgmtd_main_app.toml": "node_id = 1\n",
        "mgmtd_main_launcher.toml": "cluster_id = 'afs_3fs_patched_qualify32'\n[kv_engine.fdb]\nclusterFile = '/old/data/foundationdb/fdb.cluster'\n",
        "meta_main.toml": "[[server.base.groups]]\n[server.base.groups.listener]\nlisten_port = 19002\n[server.fdb]\nclusterFile = '/old/data/foundationdb/fdb.cluster'\n",
        "meta_main_app.toml": "node_id = 50\n",
        "meta_main_launcher.toml": "cluster_id = 'afs_3fs_patched_qualify32'\n",
    }
    for name, text in files.items():
        (path / name).write_text(text, encoding="utf-8")


class Round3Tests(unittest.TestCase):
    def test_prepare_writes_cluster_after_parent_creation(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            base = pathlib.Path(tmp)
            templates = base / "templates"
            templates.mkdir()
            make_templates(templates)
            volume = base / "vol"
            volume.mkdir()
            with mock.patch.dict(round3.VOLUMES, {"a": volume}, clear=True), \
                mock.patch.object(round3, "TEMPLATE", templates), \
                mock.patch.object(round3, "preflight", return_value={"ok": True}):
                result = round3.prepare("a")
            run = volume / round3.ROOT_NAME
            self.assertTrue(round3.FDB_KEY.isalnum())
            self.assertEqual((run / "data/foundationdb/fdb.cluster").read_text(encoding="utf-8"), f"round3:{round3.FDB_KEY}@192.168.109.11:19000\n")
            self.assertIn("fdb.cluster", result["config_sha256"])

    def test_init_chain_writes_single_abc_chain_row(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            run = pathlib.Path(tmp) / round3.ROOT_NAME
            (run / "config").mkdir(parents=True)
            (run / "run").mkdir()
            (run / "run/control-initialized.json").write_text("{}\n", encoding="utf-8")
            calls = []
            with mock.patch.object(round3, "root", return_value=run), \
                mock.patch.object(round3, "admin", side_effect=lambda _run, label, args, timeout=60: calls.append((label, args)) or {"label": label}):
                result = round3.init_chain()
            self.assertEqual((run / "config/chains.csv").read_text(encoding="utf-8"), "ChainId,TargetId,TargetId,TargetId\n1,1000001001,1000101001,1000201001\n")
            self.assertEqual(result["replicas"], 3)
            self.assertEqual([label for label, _args in calls[:3]], ["admin-create-target-a", "admin-create-target-b", "admin-create-target-c"])

    def test_fdb_configured_marker_prevents_reconfigure_after_bootstrap_failure(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            run = pathlib.Path(tmp) / round3.ROOT_NAME
            for name in ("config", "data/foundationdb", "run"):
                (run / name).mkdir(parents=True, exist_ok=True)
            for name in ("mgmtd_main_app.toml", "mgmtd_main_launcher.toml", "mgmtd_main.toml", "meta_main_app.toml", "meta_main_launcher.toml", "meta_main.toml"):
                (run / "config" / name).write_text("", encoding="utf-8")
            configure_calls = []

            def fake_fdb_cli(_run: pathlib.Path, label: str, command: str, timeout: int = 60) -> dict:
                configure_calls.append((label, command))
                return {"label": label, "command": command}

            admin_calls = 0

            def fake_admin(*_args, **_kwargs):
                nonlocal admin_calls
                admin_calls += 1
                raise round3.DriverError("bootstrap failed")

            with mock.patch.object(round3, "root", return_value=run), \
                mock.patch.object(round3, "start_service", return_value={"service": "fdb"}), \
                mock.patch.object(round3, "fdb_cli", side_effect=fake_fdb_cli), \
                mock.patch.object(round3, "admin", side_effect=fake_admin), \
                mock.patch.object(round3.time, "sleep"):
                with self.assertRaises(round3.DriverError):
                    round3.start_control()
                with self.assertRaises(round3.DriverError):
                    round3.start_control()
            self.assertTrue((run / "run/fdb-configured.json").exists())
            self.assertEqual([c for c in configure_calls if c[1] == "configure new ssd single"], [("fdb-configure-ssd-single", "configure new ssd single")])
            self.assertEqual(admin_calls, 2)

    def test_main_stop_bypasses_preflight_and_volume_guard(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            run = pathlib.Path(tmp) / round3.ROOT_NAME
            run.mkdir()
            with mock.patch.object(round3, "require_guest"), \
                mock.patch.object(round3, "guard_volume", side_effect=AssertionError("guard should not run")), \
                mock.patch.object(round3, "preflight", side_effect=AssertionError("preflight should not run")), \
                mock.patch.object(round3, "root", return_value=run), \
                mock.patch.object(round3, "stop", return_value={"status": "PASS"}), \
                mock.patch.object(round3, "save_json"), \
                mock.patch.object(round3.sys, "argv", ["round3-3fs.py", "a", "stop"]):
                result = round3.main()
            self.assertEqual(result["status"], "PASS")

    def test_stop_fails_when_process_survives_sigkill(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            run = pathlib.Path(tmp) / round3.ROOT_NAME
            (run / "run").mkdir(parents=True)
            (run / "run/storage.pid").write_text("123\n", encoding="utf-8")
            (run / "run/storage.identity.json").write_text("{}\n", encoding="utf-8")
            with mock.patch.object(round3, "root", return_value=run), \
                mock.patch.object(round3, "terminated", return_value=False), \
                mock.patch.object(round3, "verify_saved_identity", return_value={"pid": 123}), \
                mock.patch.object(round3, "proc_state", return_value="S"), \
                mock.patch.object(round3.os, "kill"), \
                mock.patch.object(round3.time, "monotonic", side_effect=[0, 11, 11, 17]):
                with self.assertRaises(round3.DriverError):
                    round3.stop("a")

    def test_stop_tolerates_processlookup_race_only_after_termination(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            run = pathlib.Path(tmp) / round3.ROOT_NAME
            (run / "run").mkdir(parents=True)
            (run / "run/storage.pid").write_text("123\n", encoding="utf-8")
            (run / "run/storage.identity.json").write_text("{}\n", encoding="utf-8")
            with mock.patch.object(round3, "root", return_value=run), \
                mock.patch.object(round3, "terminated", side_effect=[False, True, True, True, True]), \
                mock.patch.object(round3, "verify_saved_identity", return_value={"pid": 123}), \
                mock.patch.object(round3, "proc_state", return_value=None), \
                mock.patch.object(round3.os, "kill", side_effect=ProcessLookupError), \
                mock.patch.object(round3.time, "monotonic", return_value=0):
                result = round3.stop("a")
            storage = [item for item in result["stopped"] if item.get("service") == "storage"][0]
            self.assertEqual(storage["signals_sent"], ["SIGTERM"])
            self.assertFalse(storage["forced_kill"])

    def test_require_owned_mount_rejects_absent_mount(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            run = pathlib.Path(tmp) / round3.ROOT_NAME
            (run / "run").mkdir(parents=True)
            (run / "run/fuse.pid").write_text("123\n", encoding="utf-8")
            with mock.patch.object(round3, "root", return_value=run), \
                mock.patch.object(round3, "verify_saved_identity", return_value={"pid": 123}), \
                mock.patch.object(round3.os.path, "ismount", return_value=False):
                with self.assertRaises(round3.DriverError):
                    round3.require_owned_mount("a")

    def test_write_records_directory_fsync_enosys_gap(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            mount = pathlib.Path(tmp) / "mount/test"
            mount.mkdir(parents=True)
            expected = b"abc"
            with mock.patch.object(round3, "payload", return_value=expected), \
                mock.patch.object(round3, "require_owned_mount", return_value={"mounted": True}), \
                mock.patch.object(round3, "qualifier_path", return_value=mount / "file"), \
                mock.patch.object(round3.os, "fdatasync", return_value=None), \
                mock.patch.object(round3.os, "fsync", side_effect=[None, OSError(errno.ENOSYS, "not implemented")]):
                result = round3.write_qualifier("a", "file")
            self.assertEqual(result["status"], "PASS")
            self.assertTrue(result["directory_fsync_qualification_gap"])
            self.assertTrue(result["file_content_qualified"])

    def test_read_loops_partial_os_read_until_eof(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = pathlib.Path(tmp) / "file"
            data = b"abcdef"
            path.write_bytes(data)
            reads = iter([b"ab", b"cd", b"ef", b""])
            with mock.patch.object(round3, "payload", return_value=data), \
                mock.patch.object(round3, "require_owned_mount", return_value={"mounted": True}), \
                mock.patch.object(round3, "qualifier_path", return_value=path), \
                mock.patch.object(round3.os, "pread", side_effect=lambda fd, size, offset: data[offset:offset + size]), \
                mock.patch.object(round3.os, "read", side_effect=lambda fd, size: next(reads)):
                result = round3.read_qualifier("a", "file")
            self.assertEqual(result["status"], "PASS")
            self.assertEqual(result["bytes"], len(data))


if __name__ == "__main__":
    unittest.main()
