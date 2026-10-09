"""Linux-only narrow contract checks; live lifecycle still requires real VM evidence."""
import platform
import os
import tempfile
from types import SimpleNamespace
import unittest
from pathlib import Path
from unittest.mock import MagicMock, patch

import moose_direct_read_fixture as adapter


@unittest.skipUnless(platform.system() == "Linux", "acceptance checks run only in Linux")
class DirectReadFixtureTests(unittest.TestCase):
    def fixture(self, actual_role):
        role = "a" if actual_role == "c" else actual_role
        fixture = adapter.Fixture.__new__(adapter.Fixture)
        fixture.actual_role = actual_role
        fixture.role = role
        fixture.fixture = adapter.RUN
        fixture.volume = Path(adapter.VOLUMES[role])
        fixture.root = fixture.volume / "afs-delivery" / adapter.RUN
        fixture.old_root = fixture.volume / "afs-delivery" / adapter.OLD
        return fixture

    def test_master_reuses_persistent_state_and_exports_only_actual_c(self):
        fixture = self.fixture("ctl")
        configs = fixture.moose_configs()
        self.assertIn(f"DATA_PATH = {fixture.old_root}/state/moose/master\n", configs["mfsmaster.cfg"])
        self.assertIn(f"EXPORTS_FILENAME = {fixture.root}/config/mfsexports.cfg\n", configs["mfsmaster.cfg"])
        self.assertEqual(configs["mfsexports.cfg"], "192.168.109.14 / rw,alldirs,admin,maproot=0:0\n")

    def test_chunk_paths_reuse_old_data_without_readahead_tuning(self):
        fixture = self.fixture("b")
        configs = fixture.moose_configs()
        self.assertIn(f"DATA_PATH = {fixture.old_root}/state/moose/chunkstate\n", configs["mfschunkserver.cfg"])
        self.assertEqual(configs["mfshdd.cfg"], f"{fixture.old_root}/state/moose/chunks\n")
        self.assertNotIn("HDD_RR", configs["mfschunkserver.cfg"])

    def test_actual_c_mount_uses_fixed_staged_stock_direct_mode(self):
        fixture = self.fixture("c")
        argv = fixture.moose_argv()
        self.assertEqual(argv[0], str(fixture.root / "tools/mfsmount"))
        self.assertEqual(argv[-1], str(fixture.root / "mount/moose"))
        self.assertEqual(argv[argv.index("-o") + 1], "allow_other,mfsnice=0,mfscachemode=DIRECT,mfstimeout=30")
        self.assertEqual(fixture.state_paths(), [])
        self.assertEqual(adapter.base.IPS["a"], "192.168.109.14")

    def test_source_identity_and_frozen_contract_are_all_hashed(self):
        fixture = self.fixture("c")
        fixture.expectations_path = fixture.root / "tools/expectations.json"
        fixture.contract = fixture.root / "tools/contract.json"
        with patch.object(adapter.base, "safe", side_effect=lambda path, boundary: path), patch.object(adapter.base, "digest", side_effect=lambda path: str(path)):
            hashes = fixture.config_hashes()
        self.assertEqual(set(hashes), {str(adapter.WRAPPER), str(adapter.BASE), str(fixture.expectations_path), str(fixture.contract)})

    def test_servers_launch_only_isolated_fixed_executables(self):
        with patch.object(adapter.base, "tool", side_effect=AssertionError("historical executable selected")):
            for role, name in (("ctl", "mfsmaster"), ("b", "mfschunkserver")):
                fixture = self.fixture(role)
                self.assertEqual(fixture.moose_argv(), [str(fixture.root / "tools" / name), "-f", "-c",
                                 str(fixture.root / "config" / (name + ".cfg")), "start"])

    def test_staged_server_rejects_changed_bytes_and_writable_or_foreign_paths(self):
        fixture = self.fixture("ctl")
        with tempfile.TemporaryDirectory() as directory:
            fixture.root = Path(directory)
            (fixture.root / "tools").mkdir()
            path = fixture.root / "tools/mfsmaster"
            path.write_bytes(b"\x7fELFnot-the-stock-binary")
            path.chmod(0o755)
            # check_elf must validate the fixed official digest before ldd/start.
            with patch.object(fixture, "run", side_effect=AssertionError("untrusted ELF reached ldd")):
                with self.assertRaisesRegex(RuntimeError, "fixed ELF SHA mismatch"):
                    fixture.staged_stock()
            for component, uid, mode in ((path, 501, 0o755), (path.parent, 0, 0o777), (fixture.root, 0, 0o775)):
                original_stat = Path.stat
                def altered(p, *args, **kwargs):
                    value = original_stat(p, *args, **kwargs)
                    return SimpleNamespace(st_uid=uid, st_mode=mode) if p == component else value
                with patch.object(Path, "stat", altered), self.assertRaisesRegex(RuntimeError, "untrusted staged stock permissions"):
                    fixture.staged_stock()

    def test_live_child_sha_must_match_fixed_official_binary(self):
        fixture = self.fixture("c")
        fixture.expected = {"mfsmount_sha256": "fixed-stock-sha"}
        with patch.object(adapter.base.Fixture, "validate_child", return_value={"pid": 1}):
            self.assertEqual(fixture.validate_child({"exe_sha256": "fixed-stock-sha"}), {"pid": 1})
            with self.assertRaisesRegex(RuntimeError, "live stock child"):
                fixture.validate_child({"exe_sha256": "changed-after-admission"})

    def test_owned_wait_and_pidfd_lifecycle_remain_inherited(self):
        for method in ("supervise", "stop", "postcheck", "admitted_unmount"):
            self.assertIs(getattr(adapter.Fixture, method), getattr(adapter.base.Fixture, method))
        fixture = self.fixture("c")
        fixture.expectations_path = fixture.root / "tools/expectations.json"
        argv = fixture.supervisor_argv(fixture.root / "run/lifecycle")
        self.assertEqual(argv[1:4], [str(adapter.WRAPPER), "__supervise", "c"])
        self.assertIn(str(fixture.expectations_path), argv)

    def test_shell_c_argument_is_never_opened_as_moose_configuration(self):
        fixture = self.fixture("c")
        proc = MagicMock()
        proc.name = "999999"
        proc.__truediv__.return_value.read_bytes.return_value = b"bash\0-c\0" + b"x" * 10000 + b"\0"
        proc.__truediv__.return_value.iterdir.return_value = []
        proc_root = MagicMock()
        proc_root.iterdir.return_value = [proc]
        def path(value):
            if value == "/proc":
                return proc_root
            self.assertNotEqual(value, "x" * 10000, "shell program parsed as a filesystem path")
            return Path(value)
        with patch.object(adapter, "Path", side_effect=path), patch.object(adapter.os, "readlink", return_value="/usr/bin/bash"):
            self.assertEqual(fixture.old_state_idle()["active_references"], [])


if __name__ == "__main__":
    unittest.main()
