import copy
import importlib.util
from pathlib import Path
import tempfile
import tomllib
import unittest

spec = importlib.util.spec_from_file_location(
    'runc_root_command', Path(__file__).with_name('workspace-runc-root-command-linux.py'))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class RuncRootCommandGuards(unittest.TestCase):
    def fixture(self):
        grant = dict(root_id='root-776f726b7370616365', root_epoch=7,
                     home_node_id='node-a', home_session_id='session-a', access_generation=11)
        original = {key: value for key, value in grant.items() if key != 'access_generation'}
        commands = [dict(original, command_id=name, old_access_generation=generation,
                         command_type='RevokeAccess')
                    for name, generation in [('wrong', 12), ('matching', 11)]]
        receipt = dict(home_grant=grant, original_root=original,
                       home_session=dict(node_id='node-a', session_id='session-a', lease_epoch=3),
                       issued=[dict(command=command, outcome=dict(status='committed', revision=revision))
                               for command, revision in zip(commands, (20, 21))])
        events = [dict(msg=message, command_id=command['command_id'], revision=revision,
                       root_id=command['root_id'], root_epoch=7,
                       access_generation=command['old_access_generation'])
                  for command, revision, message in zip(commands, (20, 21), (
                      'ownerfs.workspace_bind_root_command_ignored',
                      'ownerfs.workspace_bind_root_command_rejected'))]
        events.append(dict(msg='node.shutdown_failed', error='code: ' + module.root_command.REFUSAL))
        health = dict(id='node-a', status='ready', checks=dict(node_registration=dict(
            ready=True, session_id='session-a', lease_epoch=3)))
        return receipt, events, health

    def test_managed_delivery_accepts_wrong_then_matching_shutdown(self):
        receipt, events, health = self.fixture()
        proof = module.verify_managed_command_delivery(receipt, events, health)
        self.assertTrue(proof['wrong_retained_until_matching'])
        self.assertEqual(proof['positions'], [0, 1])

    def test_shutdown_before_matching_command_rejects_evidence(self):
        receipt, events, health = self.fixture()
        bad = [events[0], events[2], events[1]]
        with self.assertRaises(ValueError):
            module.verify_managed_command_delivery(receipt, bad, health)

    def test_managed_config_keeps_host_bind_off_and_native_table_exact(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            text = '\n'.join([
                'experimental_ownerfs_workspace_bind = true',
                'experimental_native_workspace = false',
                'fs = "ownerfs"',
                'ownerfs_mount = "/mnt/ownerfs"',
                '',
                '[ownerfs_workspace_bind]',
                'workspace = "workspace"',
                '',
                '[native_workspace]',
                'control_dir = "/old"',
            ]) + '\n'
            enabled = module.managed_node_config(text, root, True, ['/idle'], ['/identity'])
            parsed = tomllib.loads(enabled)
            self.assertTrue(parsed['experimental_native_workspace'])
            self.assertFalse(parsed['experimental_ownerfs_workspace_bind'])
            self.assertEqual(parsed['native_workspace']['control_dir'], str(root / 'control'))
            self.assertEqual(parsed['native_workspace']['runtime'], '/usr/local/sbin/runc')
            self.assertEqual(parsed['native_workspace']['idle_command'], ['/idle'])
            self.assertNotIn('ownerfs_workspace_bind', parsed)
            disabled = tomllib.loads(module.managed_node_config(enabled, root, False, ['/idle'], ['/identity']))
            self.assertFalse(disabled['experimental_native_workspace'])
            self.assertFalse(disabled['experimental_ownerfs_workspace_bind'])
            self.assertNotIn('native_workspace', disabled)

    def test_strip_table_preserves_following_tables(self):
        text = 'a = 1\n[native_workspace]\ncontrol_dir = "/x"\n[next]\nb = 2\n'
        self.assertEqual(module.strip_table(text, 'native_workspace'), 'a = 1\n[next]\nb = 2')

    def test_truncated_or_corrupt_proof_cannot_pass(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "proof"
            for content in (module.PROOF[:-1], b"x" * len(module.PROOF)):
                path.write_bytes(content)
                with self.assertRaises(ValueError):
                    module.verify_content(path, module.PROOF)


if __name__ == '__main__':
    unittest.main()
