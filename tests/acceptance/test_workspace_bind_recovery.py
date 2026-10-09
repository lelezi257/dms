import ast
import importlib.util
import inspect
import os
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('bind_recovery', Path(__file__).with_name('workspace-bind-recovery-linux.py'))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


def health(session, epoch, persistent=True):
    return dict(id='node-a', status='ready', checks=dict(
        node_registration=dict(ready=True, session_id=session, lease_epoch=epoch),
        meta_persistence=dict(persistent_ready=persistent)))


class RecoveryCaseGuards(unittest.TestCase):
    def test_inherited_receipt_check_api_receives_evidence_for_every_call(self):
        signature = inspect.signature(module.Run.check)
        tree = ast.parse(Path(module.__file__).read_text())
        for call in ast.walk(tree):
            if isinstance(call, ast.Call) and isinstance(call.func, ast.Attribute) and call.func.attr == 'check':
                with self.subTest(line=call.lineno):
                    signature.bind(None, *([None] * len(call.args)), **{item.arg: None for item in call.keywords})

    def test_actual_file_oracle_checks_full_content_eof_and_alias_inode(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'proof'; alias = Path(directory) / 'alias'
            path.write_bytes(module.PAYLOAD);os.link(path, alias)
            before = module.proof(path)
            module.verify_proof(before, module.proof(alias))
            self.assertEqual(before['size'], 4096)
            self.assertEqual(before['sha256'], module.DIGEST)
            self.assertTrue(before['eof'])

    def test_oracle_rejects_truncation_trailing_bytes_and_corruption(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'proof'
            for data in (module.PAYLOAD[:-1], module.PAYLOAD + b'x', b'x' + module.PAYLOAD[1:]):
                with self.subTest(size=len(data)):
                    path.write_bytes(data)
                    with self.assertRaises(ValueError):module.proof(path)

    def test_oracle_refuses_a_symlink_instead_of_following_recreated_payload(self):
        with tempfile.TemporaryDirectory() as directory:
            target, alias = Path(directory) / 'target', Path(directory) / 'alias'
            target.write_bytes(module.PAYLOAD);alias.symlink_to(target)
            with self.assertRaises(OSError):module.proof(alias)

    def test_recovery_does_not_accept_changed_ownership_mode_or_physical_inode(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'proof';path.write_bytes(module.PAYLOAD);before = module.proof(path)
            for field in ('uid','gid','mode','inode','device'):
                with self.subTest(field=field),self.assertRaises(ValueError):
                    module.verify_proof(before,dict(before,**{field:before[field]+1}))
            with self.assertRaises(ValueError):module.verify_proof(dict(before,content_matches=1),dict(before,content_matches=1))

    def test_recovered_session_is_fresh_and_exceeds_both_prior_sessions(self):
        module.verify_session(health('original',2),health('trigger',3),health('recovered',5))

    def test_recovered_session_rejects_stale_trigger_volatile_and_wrong_node(self):
        for current in (health('recovered',3),health('trigger',5),health('original',5),
                        health('recovered',5,False),dict(health('recovered',5),id='other')):
            with self.subTest(current=current),self.assertRaises(ValueError):
                module.verify_session(health('original',2),health('trigger',3),current)

    def test_immutable_input_capture_detects_configuration_or_executable_changes(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);(root/'etc').mkdir();(root/'prefix/bin').mkdir(parents=True)
            config=root/'etc/node.toml';binary=root/'prefix/bin/afs-node'
            config.write_text('data_dir = "state"');binary.write_bytes(b'fixed ELF placeholder')
            before=module.immutable_inputs(root)
            config.write_text('data_dir = "other"');self.assertNotEqual(before,module.immutable_inputs(root))
            config.write_text('data_dir = "state"');binary.write_bytes(b'replaced ELF placeholder')
            self.assertNotEqual(before,module.immutable_inputs(root))


if __name__ == '__main__':unittest.main()
