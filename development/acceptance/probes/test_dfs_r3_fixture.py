"""Linux-only pure guards; never start or contact any product service."""
import copy
import json
import os
from pathlib import Path
import platform
import socket
import tempfile
import tomllib
import unittest
from types import SimpleNamespace
from unittest.mock import patch

from dfs_r3_fixture import Fixture, NODES, POLICY, SHA, allocated, digest, render, safe, validate_elf, validate_policy

CURRENT_PUBLIC_SHA = {'meta': '76a1e34c91697382cba9b3ad1e7bc5a758dd90ca8ededc823fefb9f13708cba2',
                      'node': 'c47be268dc894aac089a020130f7527c58fbc6ada0152ff57f99061c73997fb7'}


def original(fixture):
    cfg = fixture.expected()
    cfg['fs'] = 'all'
    for key, value in POLICY.items():
        cfg[key] = 2 if type(value) is int else value
    cfg.pop('experimental_native_workspace')
    cfg.pop('experimental_ownerfs_workspace_bind')
    if fixture.role != 'ctl':
        cfg['ownerfs_mount'] = '/mnt/afs/ownerfs'
    return cfg


@unittest.skipUnless(platform.system() == 'Linux', 'Linux validation only')
class FixtureGuards(unittest.TestCase):
    def test_all_roles_exact_three_and_owner_workspace_off(self):
        for role in ('ctl', 'a', 'b', 'c'):
            fixture = Fixture(role)
            cfg = tomllib.loads(fixture.patch_text(render(original(fixture))))
            self.assertEqual(cfg, fixture.expected())
            validate_policy(cfg)
            self.assertEqual(set(cfg['trusted_node_certs']), set(NODES.values()))
            self.assertNotIn('ownerfs_mount', cfg)
            self.assertFalse(cfg['experimental_native_workspace'])
            self.assertFalse(cfg['experimental_ownerfs_workspace_bind'])
            self.assertEqual(fixture.ports(), (24700, 24701) if role == 'ctl' else (24800, 24801))

    def test_current_fresh_name_and_sha_override_keep_default_generated_r2(self):
        name = 'dfs-r3-current-7e6-recovery-20261007-r1'
        fixture = Fixture('a', name, CURRENT_PUBLIC_SHA)
        cfg = tomllib.loads(fixture.patch_text(render(original(fixture))))
        self.assertEqual(fixture.fixture_name, name)
        self.assertEqual(fixture.root, Path('/mnt/lima-afsadata/afs-delivery') / name)
        self.assertEqual(fixture.sha, CURRENT_PUBLIC_SHA)
        self.assertEqual(cfg, fixture.expected())
        validate_policy(cfg)

    def test_capacity_override_keeps_historical_defaults_and_rejects_invalid_values(self):
        old = Fixture('a')
        self.assertEqual((old.ceiling, old.floor), (2**30, 2**30))
        for ceiling, floor in [(0, 2**30), (-1, 2**30), (True, 2**30),
                               (2**28, 0), (2**28, -1), (2**28, False)]:
            with self.subTest(ceiling=ceiling, floor=floor), self.assertRaisesRegex(RuntimeError, 'capacity budget'):
                Fixture('a', ceiling_bytes=ceiling, floor_bytes=floor)

    def test_capacity_override_enforces_allocation_and_full_remaining_reservation(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture('a', ceiling_bytes=2**28, floor_bytes=2**30)
            fixture.root = fixture.volume = Path(directory)
            row = {'target': directory, 'fstype': 'ext4'}
            with patch.object(fixture, 'command', return_value=json.dumps({'filesystems': [row]})):
                for used, free, permitted in [(2**27, 2**30 + 2**27, True),
                                               (2**28 + 1, 2**31, False),
                                               (2**27, 2**30 + 2**27 - 1, False)]:
                    with self.subTest(used=used, free=free), patch('dfs_r3_fixture.allocated', return_value=used), \
                            patch('dfs_r3_fixture.os.statvfs', return_value=SimpleNamespace(f_bavail=free, f_frsize=1)):
                        if permitted:
                            result = fixture.budget()
                            self.assertEqual((result['ceiling_bytes'], result['floor_bytes']), (2**28, 2**30))
                        else:
                            with self.assertRaisesRegex(RuntimeError, 'allocation/reserve'):
                                fixture.budget()

    def test_desired_three_sync_one_and_other_weakened_policies_rejected(self):
        for key in POLICY:
            cfg = copy.deepcopy(POLICY)
            cfg[key] = 1 if type(POLICY[key]) is int else 'optional'
            with self.subTest(key=key), self.assertRaises(RuntimeError):
                validate_policy(cfg)
        with self.assertRaises(RuntimeError):
            validate_policy({**POLICY, 'dfs_desired_copies': True})

    def test_generated_role_endpoint_r2_backend_and_exact_trust_rejected(self):
        fixture = Fixture('a')
        for key, value in (('id', 'dfs-b-r3'), ('grpc_listen', '0.0.0.0:23200'),
                           ('dfs_sync_required_copies', 1), ('data_mode', 'rdma'),
                           ('allow_volatile_meta', True), ('experimental_ownerfs_workspace_bind', True),
                           ('trusted_node_certs', {'dfs-a-r3': '/etc/afs/tls/dfs-a-r3.pem'})):
            cfg = original(fixture)
            cfg[key] = value
            with self.subTest(key=key), self.assertRaises(RuntimeError):
                fixture.patch_text(render(cfg))
        cfg = original(fixture)
        cfg['trusted_node_certs']['dfs-a-r3'] = '/etc/afs/tls/dfs-b-r3.pem'
        with self.assertRaises(RuntimeError):
            fixture.patch_text(render(cfg))

    def test_symlink_and_escape_refused(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'link').symlink_to('/tmp')
            for path in (root / '../outside', root.parent / 'outside', root / 'link/new'):
                with self.subTest(path=str(path)), self.assertRaises(RuntimeError):
                    safe(path, root, exists=False)

    def fixture(self, directory):
        fixture = Fixture('a')
        fixture.volume = Path(directory)
        fixture.root = Path(directory) / 'fixture'
        fixture.config = fixture.root / 'etc/node.toml'
        fixture.config.parent.mkdir(parents=True)
        fixture.config.write_text(render(original(fixture)))
        return fixture

    def test_initialized_state_refused_before_original_or_config_write(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = self.fixture(directory)
            before = fixture.config.read_bytes()
            state = fixture.root / 'state/node'
            state.mkdir(parents=True)
            (state / 'durable').write_bytes(b'preserve')
            with self.assertRaisesRegex(RuntimeError, 'already initialized'):
                fixture.patch()
            self.assertEqual(fixture.config.read_bytes(), before)
            self.assertFalse(fixture.config.with_name('node.original.toml').exists())

    def test_stopped_runtime_uds_also_forbids_policy_patch(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = self.fixture(directory)
            (fixture.root / 'run').mkdir()
            before = fixture.config.read_bytes()
            with socket.socket(socket.AF_UNIX) as leftover:
                leftover.bind(str(fixture.root / 'run/node.sock'))
                with self.assertRaisesRegex(RuntimeError, 'already initialized'):
                    fixture.patch()
            self.assertEqual(fixture.config.read_bytes(), before)
            self.assertFalse(fixture.config.with_name('node.original.toml').exists())

    def test_exclusive_backup_preserved_and_cannot_repatch(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = self.fixture(directory)
            before = fixture.config.read_bytes()
            with patch.object(fixture, 'fresh'):
                fixture.patch()
                self.assertEqual(fixture.config.with_name('node.original.toml').read_bytes(), before)
                self.assertEqual(tomllib.loads(fixture.config.read_text()), fixture.expected())
                with self.assertRaisesRegex(RuntimeError, 'backup exists'):
                    fixture.patch()

    def test_fixture_name_cannot_escape_volume(self):
        for name in ('../escape', '/tmp/escape', '', 'child/name'):
            with self.subTest(name=name), self.assertRaisesRegex(RuntimeError, 'invalid fixture name'):
                Fixture('a', name)

    def test_current_sha_and_arch_required_for_elf(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'elf'
            path.write_bytes(b'\x7fELF\x02\x01' + bytes(12) + b'\xb7\x00')
            path.chmod(0o700)
            validate_elf(path, digest(path))
            for expected in SHA.values():
                with self.assertRaisesRegex(RuntimeError, 'wrong current ELF'):
                    validate_elf(path, expected)
            path.write_bytes(b'\x7fELF\x02\x01' + bytes(12) + b'\x3e\x00')
            with self.assertRaisesRegex(RuntimeError, 'ARM64'):
                validate_elf(path, digest(path))

    def test_budget_does_not_stat_or_descend_mount_subtree(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'data').write_bytes(b'confirmed')
            (root / 'mount/dfs').mkdir(parents=True)
            (root / 'mount/dfs/symlink').symlink_to('/proc')
            expected = root.stat().st_blocks * 512 + (root / 'data').stat().st_blocks * 512
            actual_lstat = Path.lstat
            def guarded(path):
                if path.is_relative_to(root / 'mount'):
                    raise AssertionError('must not observe a mounted FUSE subtree')
                return actual_lstat(path)
            with patch.object(Path, 'lstat', guarded):
                self.assertEqual(allocated(root), expected)

    def test_only_exact_owned_node_uds_permitted_in_budget(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'run').mkdir()
            with socket.socket(socket.AF_UNIX) as own, socket.socket(socket.AF_UNIX) as foreign:
                own.bind(str(root / 'run/node.sock'))
                allocated(root)
                foreign.bind(str(root / 'run/unknown.sock'))
                with self.assertRaisesRegex(RuntimeError, 'nonregular'):
                    allocated(root)

    def test_exact_installer_aliases_count_without_following(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'etc').mkdir()
            (root / 'etc/node.toml').write_bytes(b'fixed config')
            aliases = [root / 'etc' / name for name in ('node-dfs.toml', 'node-ownerfs.toml')]
            for alias in aliases:
                alias.symlink_to('node.toml')
            expected = sum(path.lstat().st_blocks * 512 for path in [root, root / 'etc', root / 'etc/node.toml', *aliases])
            actual_stat = Path.stat
            def guarded(path, *args, **kwargs):
                if path in aliases and kwargs.get('follow_symlinks') is not False:
                    raise AssertionError('installer aliases must not be followed')
                return actual_stat(path, *args, **kwargs)
            with patch.object(Path, 'stat', guarded):
                self.assertEqual(allocated(root), expected)

    def test_installer_alias_wrong_target_escape_and_unknown_link_refused(self):
        for name, target in (('node-dfs.toml', '../node.toml'), ('node-ownerfs.toml', '/tmp/node.toml'),
                             ('node-dfs.toml', 'other.toml'), ('unknown.toml', 'node.toml')):
            with self.subTest(name=name, target=target), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                (root / 'etc').mkdir()
                (root / 'etc/node.toml').write_bytes(b'fixed')
                (root / 'etc' / name).symlink_to(target)
                with self.assertRaises(RuntimeError):
                    allocated(root)


if __name__ == '__main__':
    unittest.main()
