"""Focused config/path/process-identity guards; execute on Linux only."""
import importlib.util
import json
import os
from pathlib import Path
import tempfile
import tomllib
import unittest
from unittest.mock import Mock, patch

spec = importlib.util.spec_from_file_location('remote_fixture', Path(__file__).with_name('owner_remote_fixture.py'))
f = importlib.util.module_from_spec(spec)
spec.loader.exec_module(f)

# Captured installed-helper stdout; deliberately no literal "Usage:" prefix.
FUSERMOUNT_HELP = '''/usr/bin/fusermount3: [options] mountpoint
Options:
 -h print help
 -V print version
 -o opt[,opt...] mount options
 -u unmount
 -q quiet
 -z lazy unmount
'''


class FixtureGuards(unittest.TestCase):
    def test_current_direct_argv_does_not_mutate_legacy_cache_or_ports(self):
        with patch.object(f, 'tool', side_effect=lambda rel: f.MOOSE / rel):
            current = f.Fixture('a', f.CURRENT_READ_FIXTURE)
            old = f.Fixture('a', f.WRITE_FIXTURE)
            argv = current.moose_argv()
            self.assertIn('mfscachemode=DIRECT', argv[argv.index('-o') + 1])
            self.assertEqual(argv[argv.index('-P') + 1], '24942')
            self.assertIn('mfscachemode=AUTO', old.moose_argv()[7])
            self.assertEqual(old.port('node_grpc'), 22900)
            self.assertNotEqual(current.root, old.root)

    def test_backing_hot_variant_is_auto_and_preserves_direct_history(self):
        with patch.object(f, 'tool', side_effect=lambda rel: f.MOOSE / rel):
            fresh = f.Fixture('a', f.BACKING_HOT_READ_FIXTURE)
            old = f.Fixture('a', f.CURRENT_READ_FIXTURE)
            self.assertIn('mfscachemode=AUTO', fresh.moose_argv()[7])
            self.assertIn('mfscachemode=DIRECT', old.moose_argv()[7])
            self.assertNotEqual(fresh.root, old.root)
            for role in ('ctl', 'a', 'b'):
                self.assertTrue(set(f.PORT_PLAN[f.BACKING_HOT_READ_FIXTURE][role].values()).isdisjoint(
                    f.PORT_PLAN[f.CURRENT_READ_FIXTURE][role].values()))
            self.assertEqual(fresh.moose_argv()[5], '25242')
            self.assertIn(f.BACKING_HOT_READ_FIXTURE, f.MUTABLE_FIXTURES)
            self.assertIn(f.BACKING_HOT_READ_FIXTURE, f.F03_READ_FIXTURES)

    def test_backing_hot_config_uses_new_ports_and_explicitly_disables_bind(self):
        raw = '''id = "remote-b-r1"
fs = "all"
experimental_native_workspace = true
experimental_ownerfs_workspace_bind = true
tls_ca_certificate = "/etc/afs/tls/ca.pem"
tls_identity_certificate = "/etc/afs/tls/remote-b-r1.pem"
tls_identity_private_key = "/etc/afs/tls/remote-b-r1-key.pem"
trusted_node_certs = { remote-a-r1 = "/etc/afs/tls/remote-a-r1.pem", remote-b-r1 = "/etc/afs/tls/remote-b-r1.pem" }
'''
        fixture = f.Fixture('b', f.BACKING_HOT_READ_FIXTURE)
        cfg = tomllib.loads(fixture.patch_toml(raw))
        self.assertIs(cfg['experimental_native_workspace'], False)
        self.assertIs(cfg['experimental_ownerfs_workspace_bind'], False)
        self.assertEqual(cfg['meta_endpoint'], 'https://192.168.109.11:25080')
        self.assertEqual(cfg['advertise_endpoint'], 'https://192.168.109.13:25180')
        self.assertEqual(set(cfg['trusted_node_certs']), {'remote-a-r1', 'remote-b-r1'})
        self.assertTrue(all(str(fixture.root) in x for x in cfg['trusted_node_certs'].values()))

    def test_current_config_overrides_both_on_switches_and_keeps_peer_trust(self):
        raw = '''id = "remote-a-r1"
fs = "all"
experimental_native_workspace = true
experimental_ownerfs_workspace_bind = true
tls_ca_certificate = "/etc/afs/tls/ca.pem"
tls_identity_certificate = "/etc/afs/tls/remote-a-r1.pem"
tls_identity_private_key = "/etc/afs/tls/remote-a-r1-key.pem"
trusted_node_certs = { remote-a-r1 = "/etc/afs/tls/remote-a-r1.pem", remote-b-r1 = "/etc/afs/tls/remote-b-r1.pem" }
'''
        fixture = f.Fixture('a', f.CURRENT_READ_FIXTURE)
        cfg = tomllib.loads(fixture.patch_toml(raw))
        self.assertIs(cfg['experimental_native_workspace'], False)
        self.assertIs(cfg['experimental_ownerfs_workspace_bind'], False)
        self.assertEqual(cfg['meta_endpoint'], 'https://192.168.109.11:24780')
        self.assertEqual(set(cfg['trusted_node_certs']), {'remote-a-r1', 'remote-b-r1'})
        self.assertTrue(all(str(fixture.root) in x for x in cfg['trusted_node_certs'].values()))
        with self.assertRaisesRegex(RuntimeError, 'unexpected trust'):
            fixture.patch_toml(raw.replace('remote-b-r1 =', 'intruder ='))

    def test_current_capacity_uses_own_budget_and_keeps_free_floor(self):
        with tempfile.TemporaryDirectory() as tmp:
            volume = Path(tmp)
            fixture = f.Fixture('a', f.CURRENT_READ_FIXTURE)
            fixture.volume = volume
            fixture.root = volume / 'fresh'
            fixture.root.mkdir()
            fixture.run = Mock(return_value=json.dumps({'filesystems': [{'target': str(volume), 'fstype': 'ext4'}]}))
            info = Mock(f_bavail=1600, f_frsize=1024**2)
            with patch.object(f.os, 'statvfs', return_value=info):
                result = fixture.capacity(admission=True)
                self.assertEqual(result['working_budget_bytes'], 512 * 1024**2)
                self.assertEqual(result['remaining_free_floor_bytes'], 1024**3)
                info.f_bavail = 1535
                with self.assertRaisesRegex(RuntimeError, 'insufficient new case'):
                    fixture.capacity(admission=True)
                fixture.fixture = f.WRITE_FIXTURE
                self.assertEqual(fixture.capacity(admission=True)['working_budget_bytes'], 256 * 1024**2)

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
            fixture.run = Mock()
            with patch.object(f.signal, 'pidfd_send_signal') as send:
                with self.assertRaisesRegex(RuntimeError, 'child role/root differs'):
                    fixture.stop(1)
                send.assert_not_called()
                fixture.run.assert_not_called()

    def test_new_fixture_has_independent_root_and_propagates_to_supervisor(self):
        old, new = f.Fixture('a'), f.Fixture('a', f.WRITE_FIXTURE)
        self.assertNotEqual(old.root, new.root)
        self.assertFalse(new.root.is_relative_to(old.root))
        argv = new.supervisor_argv(new.root / 'run/lifecycle')
        self.assertEqual(argv[argv.index('--fixture') + 1], f.WRITE_FIXTURE)
        self.assertEqual(argv[argv.index('--lifecycle') + 1], str(new.root / 'run/lifecycle'))
        with patch.object(f, 'tool', side_effect=lambda rel: f.MOOSE / rel):
            self.assertIn(str(new.root / 'mount/moose'), new.moose_argv())
        with self.assertRaises(RuntimeError):
            f.Fixture('a', '../old-case')


    def test_delete_fixture_has_independent_root_ports_and_commands(self):
        read_fixture = f.Fixture('a')
        write_fixture = f.Fixture('a', f.WRITE_FIXTURE)
        delete_fixture = f.Fixture('a', f.DELETE_FIXTURE)
        self.assertNotEqual(delete_fixture.root, read_fixture.root)
        self.assertNotEqual(delete_fixture.root, write_fixture.root)
        self.assertFalse(delete_fixture.root.is_relative_to(read_fixture.root))
        self.assertFalse(delete_fixture.root.is_relative_to(write_fixture.root))
        self.assertEqual(f.PORT_PLAN[f.DELETE_FIXTURE]['ctl'],
                         {'meta_grpc': 23400, 'meta_rest': 23401, 'matoml': 23640, 'matocs': 23641, 'matocl': 23642})
        self.assertEqual(f.PORT_PLAN[f.DELETE_FIXTURE]['a'], {'node_grpc': 23500, 'node_rest': 23501})
        self.assertEqual(f.PORT_PLAN[f.DELETE_FIXTURE]['b'], {'node_grpc': 23500, 'node_rest': 23501, 'chunk': 23643})
        self.assertTrue(set(f.PORT_PLAN[f.DELETE_FIXTURE]['ctl'].values()).isdisjoint(f.PORT_PLAN[f.WRITE_FIXTURE]['ctl'].values()))
        self.assertTrue(set(f.PORT_PLAN[f.DELETE_FIXTURE]['a'].values()).isdisjoint(f.PORT_PLAN[f.WRITE_FIXTURE]['a'].values()))
        self.assertTrue(set(f.PORT_PLAN[f.DELETE_FIXTURE]['b'].values()).isdisjoint(f.PORT_PLAN[f.WRITE_FIXTURE]['b'].values()))
        with patch.object(f, 'tool', side_effect=lambda rel: f.MOOSE / rel):
            argv = delete_fixture.moose_argv()
        self.assertEqual(argv[argv.index('-P') + 1], '23642')
        self.assertIn(str(delete_fixture.root / 'mount/moose'), argv)


    def test_start_bind_ports_are_selected_without_cross_role_lookup(self):
        self.assertEqual(f.Fixture('ctl', f.DELETE_FIXTURE).start_bind_ports(), [23640, 23641, 23642])
        self.assertEqual(f.Fixture('a', f.DELETE_FIXTURE).start_bind_ports(), [])
        self.assertEqual(f.Fixture('b', f.DELETE_FIXTURE).start_bind_ports(), [23643])
        self.assertEqual(f.Fixture('ctl', f.WRITE_FIXTURE).start_bind_ports(), [23040, 23041, 23042])
        self.assertEqual(f.Fixture('a', f.WRITE_FIXTURE).start_bind_ports(), [])
        self.assertEqual(f.Fixture('b', f.WRITE_FIXTURE).start_bind_ports(), [23043])

    def test_delete_fixture_config_uses_delete_ports_without_touching_write_fixture(self):
        raw = '''id = "remote-a-r1"
fs = "all"
dfs_mount = "/mnt/afs/dfs"
tls_ca_certificate = "/etc/afs/tls/ca.pem"
tls_identity_certificate = "/etc/afs/tls/remote-a-r1.pem"
tls_identity_private_key = "/etc/afs/tls/remote-a-r1-key.pem"
trusted_node_certs = { remote-a-r1 = "/etc/afs/tls/remote-a-r1.pem", remote-b-r1 = "/etc/afs/tls/remote-b-r1.pem" }
'''
        delete_cfg = tomllib.loads(f.Fixture('a', f.DELETE_FIXTURE).patch_toml(raw))
        write_cfg = tomllib.loads(f.Fixture('a', f.WRITE_FIXTURE).patch_toml(raw))
        self.assertEqual(delete_cfg['meta_endpoint'], 'https://192.168.109.11:23400')
        self.assertEqual(delete_cfg['advertise_endpoint'], 'https://192.168.109.12:23500')
        self.assertEqual(delete_cfg['grpc_listen'], '0.0.0.0:23500')
        self.assertEqual(delete_cfg['rest_listen'], '0.0.0.0:23501')
        self.assertEqual(write_cfg['meta_endpoint'], 'https://192.168.109.11:22800')
        self.assertEqual(write_cfg['advertise_endpoint'], 'https://192.168.109.12:22900')

    def test_historical_fixture_cannot_be_prepared_or_readmitted(self):
        fixture = f.Fixture('a')
        with self.assertRaisesRegex(RuntimeError, 'historical read fixture'):
            fixture.prepare()
        with self.assertRaisesRegex(RuntimeError, 'historical read fixture'):
            fixture.preflight(fixture.root / 'tools/io')

    def test_supervisor_rejects_foreign_fixture_before_spawn(self):
        with tempfile.TemporaryDirectory() as tmp:
            fixture = f.Fixture('a', f.WRITE_FIXTURE)
            fixture.root = Path(tmp)
            life = fixture.root / 'run/lifecycle'
            life.mkdir(parents=True)
            (life / 'launch.json').write_text(json.dumps({'role': 'a', 'root': str(fixture.root),
                'fixture': f.FIXTURE, 'boot_id': Path('/proc/sys/kernel/random/boot_id').read_text().strip()}))
            with patch.object(f.subprocess, 'Popen') as spawn:
                with self.assertRaisesRegex(RuntimeError, 'fixture/boot'):
                    fixture.supervise(life)
                spawn.assert_not_called()

    def make_stop_fixture(self, root, role='a'):
        fixture = f.Fixture(role, f.WRITE_FIXTURE)
        fixture.root = root
        life = root / 'run/lifecycle'
        life.mkdir(parents=True)
        mount = {'target': str(root / 'mount/moose'), 'source': 'mfs#192.168.109.11:23042',
                 'fstype': 'fuse.mfs', 'options': 'rw', 'id': 777}
        identity = {'pid': 12345, 'role': role, 'root': str(root), 'fixture': f.WRITE_FIXTURE,
                    'script_sha256': f.digest(Path(f.__file__)), 'mount': mount, 'start_ticks': 101,
                    'boot_id': Path('/proc/sys/kernel/random/boot_id').read_text().strip(), 'argv': ['fixed'], 'config_sha256': {},
                    'exe_path': '/fixed', 'exe_dev': 1, 'exe_inode': 2, 'exe_sha256': 'fixed-sha'}
        (life / 'child.json').write_text(json.dumps(identity))
        fixture.current_lifecycle = lambda: life
        fixture.validate_child = Mock()
        fixture.admitted_unmount = Mock(return_value='/usr/bin/fusermount3')
        fixture.moose_argv = Mock(return_value=['fixed'])
        fixture.config_hashes = Mock(return_value={})
        return fixture, life, identity

    def test_foreign_mount_refused_before_unmount_or_signal(self):
        with tempfile.TemporaryDirectory() as tmp:
            fixture, _, identity = self.make_stop_fixture(Path(tmp))
            fixture.exact_mount = Mock(return_value={**identity['mount'], 'id': 778})
            fixture.run = Mock()
            with patch.object(f.os, 'pidfd_open', return_value=42), patch.object(f.os, 'close'), patch.object(f.signal, 'pidfd_send_signal') as send:
                with self.assertRaisesRegex(RuntimeError, 'mount incarnation'):
                    fixture.stop(0.01)
                fixture.run.assert_not_called()
                send.assert_not_called()

    def test_unmount_failure_retains_child_without_term_fallback(self):
        with tempfile.TemporaryDirectory() as tmp:
            fixture, life, identity = self.make_stop_fixture(Path(tmp))
            fixture.exact_mount = Mock(return_value=identity['mount'])
            fixture.run = Mock(side_effect=RuntimeError('unmount failed'))
            with patch.object(f.os, 'pidfd_open', return_value=42), patch.object(f.os, 'close'), patch.object(f.signal, 'pidfd_send_signal') as send:
                with self.assertRaisesRegex(RuntimeError, 'unmount failed'):
                    fixture.stop(0.01)
                fixture.run.assert_called_once_with(['/usr/bin/fusermount3', '-u', fixture.root / 'mount/moose'], timeout=0.01)
                self.assertFalse((life / 'exit.json').exists())
                self.assertTrue((life / 'child.json').exists())
                send.assert_not_called()

    def test_normal_unmount_requires_real_wait_zero_and_mount_gone(self):
        with tempfile.TemporaryDirectory() as tmp:
            fixture, life, identity = self.make_stop_fixture(Path(tmp))
            fixture.exact_mount = Mock(side_effect=[identity['mount'], None])
            def unmount(argv, **kwargs):
                (life / 'exit.json').write_text(json.dumps({'role': 'a', 'root': str(fixture.root),
                    'fixture': f.WRITE_FIXTURE, 'pid': identity['pid'], 'identity': identity,
                    'wait_completed': True, 'exit_code': 0}))
            fixture.run = Mock(side_effect=unmount)
            with patch.object(f.os, 'pidfd_open', return_value=42), patch.object(f.os, 'close'), patch.object(f.signal, 'pidfd_send_signal') as send:
                self.assertEqual(fixture.stop(0.01)['status'], 'STOPPED')
                send.assert_not_called()

    def test_previous_nonzero_wait_receipt_remains_failure(self):
        with tempfile.TemporaryDirectory() as tmp:
            fixture, life, identity = self.make_stop_fixture(Path(tmp))
            fixture.exact_mount = Mock(return_value=None)
            fixture.run = Mock()
            (life / 'exit.json').write_text(json.dumps({'role': 'a', 'root': str(fixture.root),
                'fixture': f.WRITE_FIXTURE, 'pid': identity['pid'], 'identity': identity,
                'wait_completed': True, 'exit_code': 1}))
            with self.assertRaisesRegex(RuntimeError, 'nonzero real wait exit: 1'):
                fixture.stop(0.01)
            fixture.run.assert_not_called()

    def test_help_exit_one_with_normal_usage_is_retained_and_admitted(self):
        fixture = f.Fixture('a', f.WRITE_FIXTURE)
        fixture.unmount_identity = Mock(return_value={'path': '/usr/bin/fusermount3'})
        def run(argv, allowed=(0,), **kwargs):
            is_help = argv[-1] == '-h'
            code = 1 if is_help else 0
            self.assertIn(code, allowed)
            stdout = FUSERMOUNT_HELP if is_help else ''
            fixture.commands.append({'argv': argv, 'returncode': code, 'stdout': stdout, 'stderr': ''})
            return stdout
        fixture.run = Mock(side_effect=run)
        self.assertEqual(fixture.inspect_unmount(), {'path': '/usr/bin/fusermount3'})
        self.assertEqual(fixture.commands[0]['returncode'], 1)
        self.assertEqual(fixture.commands[0]['stdout'], FUSERMOUNT_HELP)
        fixture.run.assert_any_call(['/usr/bin/fusermount3', '-h'], allowed=(0, 1))
        fixture.run.assert_any_call(['/usr/bin/fusermount3', '-V'])

    def test_help_exit_one_without_usage_or_unmount_option_is_rejected(self):
        for text in ('fusermount3: error', FUSERMOUNT_HELP.replace(' -u unmount\n', ''),
                     FUSERMOUNT_HELP.replace(' -V print version\n', '')):
            fixture = f.Fixture('a', f.WRITE_FIXTURE)
            fixture.unmount_identity = Mock(return_value={'path': '/usr/bin/fusermount3'})
            def run(argv, **kwargs):
                fixture.commands.append({'argv': argv, 'returncode': 1, 'stdout': text, 'stderr': ''})
                return text
            fixture.run = Mock(side_effect=run)
            with self.subTest(text=text), self.assertRaisesRegex(RuntimeError, 'capability missing'):
                fixture.inspect_unmount()
            self.assertEqual(fixture.run.call_count, 1)

    def test_nonzero_version_is_not_covered_by_help_exit_exception(self):
        fixture = f.Fixture('a', f.WRITE_FIXTURE)
        fixture.unmount_identity = Mock(return_value={'path': '/usr/bin/fusermount3'})
        def run(argv, allowed=(0,), **kwargs):
            if argv[-1] == '-h':
                fixture.commands.append({'argv': argv, 'returncode': 1, 'stdout': FUSERMOUNT_HELP, 'stderr': ''})
                return FUSERMOUNT_HELP
            self.assertEqual(argv[-1], '-V')
            self.assertEqual(allowed, (0,))
            raise RuntimeError('version exited 1')
        fixture.run = Mock(side_effect=run)
        with self.assertRaisesRegex(RuntimeError, 'version exited 1'):
            fixture.inspect_unmount()


if __name__ == '__main__':
    unittest.main()
