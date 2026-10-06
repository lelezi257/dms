"""Focused config/path/process-identity guards; execute on Linux only."""
import importlib.util
import json
import os
from pathlib import Path
import tempfile
import tomllib
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('remote_fixture', Path(__file__).with_name('owner_remote_fixture.py'))
f = importlib.util.module_from_spec(spec)
spec.loader.exec_module(f)


class FixtureGuards(unittest.TestCase):
    def test_owner_only_config_preserves_trust_and_removes_dfs_mount(self):
        raw = '''id = "remote-a-r1"
fs = "all"
dfs_mount = "/mnt/afs/dfs"
tls_ca_certificate = "/etc/afs/tls/ca.pem"
tls_identity_certificate = "/etc/afs/tls/remote-a-r1.pem"
tls_identity_private_key = "/etc/afs/tls/remote-a-r1-key.pem"
trusted_node_certs = { remote-a-r1 = "/etc/afs/tls/remote-a-r1.pem", remote-b-r1 = "/etc/afs/tls/remote-b-r1.pem" }
'''
        fixture = f.Fixture('a')
        cfg = tomllib.loads(fixture.patch_toml(raw))
        self.assertEqual(cfg['fs'], 'ownerfs')
        self.assertFalse(cfg['experimental_native_workspace'])
        self.assertNotIn('dfs_mount', cfg)
        self.assertEqual(set(cfg['trusted_node_certs']), {'remote-a-r1', 'remote-b-r1'})
        self.assertEqual(cfg['ownerfs_mount'], str(fixture.root / 'mount/ownerfs'))
        with self.assertRaises(RuntimeError):
            fixture.patch_toml(raw.replace('id = "remote-a-r1"', 'id = "old-node"'))

    def test_path_escape_and_dangling_symlink_are_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            with self.assertRaises(RuntimeError):
                f.safe(root / '../outside', root, exists=False)
            link = root / 'link'
            link.symlink_to(root / 'missing')
            with self.assertRaises(RuntimeError):
                f.safe(link / 'state', root, exists=False)

    def test_scheduling_state_is_not_identity_but_pid_reuse_is(self):
        before = {'state': 'R', 'start_ticks': 1, 'exe_path': '/fixed', 'exe_dev': 2, 'exe_inode': 3}
        self.assertTrue(f.same_incarnation(before, {**before, 'state': 'S'}))
        self.assertFalse(f.same_incarnation(before, {**before, 'start_ticks': 4}))

    def test_foreign_child_is_rejected_before_signal(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            life = root / 'run/moose-mount-test'
            life.mkdir(parents=True)
            (life / 'child.json').write_text(json.dumps({'pid': os.getpid(), 'role': 'b', 'root': str(root)}))
            fixture = f.Fixture('a')
            fixture.root = root
            fixture.current_lifecycle = lambda: life
            with patch.object(f.signal, 'pidfd_send_signal') as send:
                with self.assertRaisesRegex(RuntimeError, 'child role/root differs'):
                    fixture.stop(1)
                send.assert_not_called()


if __name__ == '__main__':
    unittest.main()
