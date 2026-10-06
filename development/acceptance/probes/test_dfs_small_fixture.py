"""Pure configuration/path guard regressions. Run with unittest on Linux."""
import tempfile
import tomllib
import unittest
from pathlib import Path
from unittest.mock import patch

from dfs_small_fixture import Fixture, NODES, R2, safe


def original(fixture):
    cfg = fixture.expected()
    cfg["fs"] = "all"
    cfg.pop("experimental_native_workspace")
    cfg["data_dir"] = "/var/lib/afs/" + fixture.name
    cfg["trusted_node_certs"] = {n: "/etc/afs/tls/" + n + ".pem" for n in NODES.values()}
    if fixture.role != "ctl":
        cfg["ownerfs_mount"] = "/mnt/afs/ownerfs"
    return cfg


def render(cfg):
    return "\n".join(k + " = " + Fixture.toml(v) for k, v in cfg.items()) + "\n"


class ConfigTests(unittest.TestCase):
    def test_every_role_dfs_only_r2_and_guest_paths(self):
        for role in ("ctl", "a", "b", "c"):
            fixture = Fixture(role)
            result = tomllib.loads(fixture.patch(render(original(fixture))))
            self.assertEqual(result, fixture.expected())
            self.assertNotIn("ownerfs_mount", result)
            self.assertFalse(result["experimental_native_workspace"])
            for key, value in R2.items():
                self.assertEqual(result[key], value)
            if role != "ctl":
                self.assertEqual(result["dfs_mount"], str(fixture.root / "mount/dfs"))
                self.assertEqual(result["data_mode"], "grpc")
                self.assertFalse(result["allow_volatile_meta"])

    def test_rejects_wrong_role_backend_replication_or_trust(self):
        for role, key, value in (("a", "id", "dfs-b-r1"), ("ctl", "meta_store", "memory"),
                                 ("a", "dfs_sync_required_copies", 1), ("a", "dfs_local_copy", "optional"),
                                 ("a", "trusted_node_certs", {"dfs-a-r1": "/etc/afs/tls/dfs-a-r1.pem"})):
            fixture = Fixture(role)
            cfg = original(fixture)
            cfg[key] = value
            with self.subTest(key=key), self.assertRaises(RuntimeError):
                fixture.patch(render(cfg))

    def test_rejects_trust_certificate_swap_despite_common_sans(self):
        fixture = Fixture("b")
        cfg = original(fixture)
        cfg["trusted_node_certs"]["dfs-a-r1"] = "/etc/afs/tls/dfs-b-r1.pem"
        with self.assertRaisesRegex(RuntimeError, "trust identity"):
            fixture.patch(render(cfg))

    def test_rejects_unknown_native_or_external_settings(self):
        fixture = Fixture("c")
        cfg = original(fixture)
        cfg["native_workspace_state_dir"] = "/old/fixture"
        with self.assertRaisesRegex(RuntimeError, "unexpected generated"):
            fixture.patch(render(cfg))

    def test_rejects_nonflat_or_already_prepared_config(self):
        fixture = Fixture("a")
        with self.assertRaises(RuntimeError):
            fixture.patch("[nested]\nfs = 'all'\n")
        with self.assertRaises(RuntimeError):
            fixture.patch(render(fixture.expected()))

    def test_path_guard_rejects_escape_traversal_and_symlink(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "link").symlink_to("/tmp")
            for path in (root.parent / "outside", root / ".." / "outside", root / "link" / "new"):
                with self.subTest(path=str(path)), self.assertRaises(RuntimeError):
                    safe(path, root, exists=False)
            self.assertEqual(safe(root / "new", root, exists=False), root / "new")

    def test_prepare_preserves_original_and_refuses_second_prepare(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture("a")
            fixture.volume = Path(directory)
            fixture.root = Path(directory) / "fixture"
            fixture.config = fixture.root / "etc/node.toml"
            fixture.config.parent.mkdir(parents=True)
            text = render(original(fixture))
            fixture.config.write_text(text)
            with patch.object(fixture, "capacity", return_value={}), patch.object(fixture, "idle", return_value={}):
                fixture.prepare()
                self.assertEqual(fixture.config.with_name("node.original.toml").read_text(), text)
                self.assertEqual(tomllib.loads(fixture.config.read_text()), fixture.expected())
                with self.assertRaisesRegex(RuntimeError, "exclusive"):
                    fixture.prepare()

    def test_prepare_active_fixture_fails_before_writing(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture("a")
            fixture.volume = Path(directory)
            fixture.root = Path(directory) / "fixture"
            fixture.config = fixture.root / "etc/node.toml"
            fixture.config.parent.mkdir(parents=True)
            text = render(original(fixture))
            fixture.config.write_text(text)
            with patch.object(fixture, "idle", side_effect=RuntimeError("new fixture mount already active")):
                with self.assertRaises(RuntimeError):
                    fixture.prepare()
            self.assertEqual(fixture.config.read_text(), text)
            self.assertFalse(fixture.config.with_name("node.original.toml").exists())


if __name__ == "__main__":
    unittest.main()
