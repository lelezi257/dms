import importlib.util
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest import mock

spec = importlib.util.spec_from_file_location('source_replace', Path(__file__).with_name('workspace-bind-source-replace-linux.py'))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class SourceReplacementGuards(unittest.TestCase):
    def fixture(self, directory):
        source, retained = [Path(directory) / name for name in ('source', 'retained')]
        source.mkdir(mode=0o700); retained.mkdir(mode=0o700)
        (source / 'proof').write_bytes(module.PROOF)
        return source, retained, module.directory_identity(source), module.directory_identity(retained)

    def test_atomic_exchange_preserves_and_restores_complete_source(self):
        with tempfile.TemporaryDirectory() as directory:
            source, retained, old, new = self.fixture(directory)
            module.exchange(source, retained)
            module.verify_preserved(source, retained, old, new, 'proof')
            module.restore_source(source, retained, old, new, 'proof')
            self.assertEqual(module.directory_identity(source), old)
            self.assertEqual((source / 'proof').read_bytes(), module.PROOF)
            self.assertFalse(retained.exists())

    def test_restore_refuses_foreign_data_without_mutation(self):
        with tempfile.TemporaryDirectory() as directory:
            source, retained, old, new = self.fixture(directory)
            module.exchange(source, retained)
            (source / 'foreign').write_bytes(b'keep')
            with self.assertRaises(ValueError):
                module.restore_source(source, retained, old, new, 'proof')
            self.assertEqual((source / 'foreign').read_bytes(), b'keep')
            self.assertEqual(module.directory_identity(retained), old)

    def test_preservation_rejects_changed_proof_permissions_or_identity(self):
        with tempfile.TemporaryDirectory() as directory:
            source, retained, old, new = self.fixture(directory)
            module.exchange(source, retained)
            for bad in (dict(old, inode=old['inode']+1), dict(old, mode=0o755)):
                with self.assertRaises(ValueError):
                    module.verify_preserved(source, retained, bad, new, 'proof')
            (retained / 'proof').write_bytes(b'incomplete')
            with self.assertRaises(ValueError):
                module.verify_preserved(source, retained, old, new, 'proof')

    def test_identity_refuses_symlink_directories_and_proof(self):
        with tempfile.TemporaryDirectory() as directory:
            source, retained, old, new = self.fixture(directory)
            alias = Path(directory) / 'alias'; alias.symlink_to(source)
            with self.assertRaises(ValueError):module.directory_identity(alias)
            module.exchange(source, retained)
            (retained / 'proof').rename(retained / 'real')
            (retained / 'proof').symlink_to(retained / 'real')
            with self.assertRaises(ValueError):
                module.restore_source(source, retained, old, new, 'proof')
            self.assertTrue((retained / 'real').exists())

    def test_source_scope_refuses_foreign_path_name_and_changed_identity(self):
        root = Path('/opt/owned-case')
        source = root / 'state/ownerfs/root-776f726b7370616365-e2'
        old = dict(device=1, inode=2, mode=0o700, uid=501, gid=501)
        with mock.patch.object(module, 'directory_identity', return_value=old):
            module.verify_fixture_source(root, source, old)
            for other in (Path('/opt/foreign/state/ownerfs')/source.name,
                          root/'state/ownerfs/root-other', source.parent/'root-776f726b7370616365-e0'):
                with self.assertRaises(ValueError):module.verify_fixture_source(root, other, old)
            with self.assertRaises(ValueError):
                module.verify_fixture_source(root, source, dict(old, inode=3))

    def test_error_oracle_accepts_only_exact_source_identity_failure(self):
        event = dict(msg='node.shutdown_failed', error=module.SOURCE_ERROR)
        module.verify_source_error(json.dumps(event)+'\nError: terminal failure\n')
        for bad in (dict(event, msg='unrelated'), dict(event, error='native Home source was replaced'),
                    dict(event, error=module.SOURCE_ERROR.replace('0x020b0001','0x020b0002'))):
            with self.assertRaises(ValueError):module.verify_source_error(json.dumps(bad))


if __name__ == '__main__':
    unittest.main()
