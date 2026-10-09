import copy
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location(
    'root_command', Path(__file__).with_name('workspace-bind-root-command-linux.py'))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class CommandEvidenceGuards(unittest.TestCase):
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
        events.append(dict(msg='node.shutdown_failed', error='code: ' + module.REFUSAL))
        health = dict(id='node-a', status='ready', checks=dict(node_registration=dict(
            ready=True, session_id='session-a', lease_epoch=3)))
        return receipt, events, health

    def test_exact_current_commits_and_production_events(self):
        receipt, events, health = self.fixture()
        self.assertTrue(module.verify_command_delivery(receipt, events, health)['matching_rejected'])

    def test_stale_session_epoch_or_other_node_cannot_qualify(self):
        for mutate in (lambda h: h.update(id='other'),
                       lambda h: h['checks']['node_registration'].update(lease_epoch=4),
                       lambda h: h['checks']['node_registration'].update(session_id='stale'),
                       lambda h: h.update(status='unready')):
            receipt, events, health = self.fixture()
            mutate(health)
            with self.assertRaises(ValueError):
                module.verify_command_delivery(receipt, events, health)

    def test_uncommitted_wrong_tuple_or_unsupported_command_cannot_qualify(self):
        for mutate in (lambda r: r['issued'][0]['outcome'].update(status='condition_failed'),
                       lambda r: r['issued'][1]['command'].update(old_access_generation=12),
                       lambda r: r['issued'][0]['command'].update(root_epoch=8),
                       lambda r: r['issued'][0]['command'].update(command_type='InvalidateCache'),
                       lambda r: r['issued'][1]['outcome'].update(revision=20),
                       lambda r: r['original_root'].update(home_session_id='other')):
            receipt, events, health = self.fixture()
            mutate(receipt)
            with self.assertRaises(ValueError):
                module.verify_command_delivery(receipt, events, health)

    def test_missing_duplicate_reordered_or_other_cause_events_cannot_qualify(self):
        receipt, events, health = self.fixture()
        bad = [events[1:], events + [copy.deepcopy(events[0])],
               [events[1], events[0], events[2]], events[:2],
               events[:2] + [dict(msg='node.shutdown_failed', error='physical source replaced')]]
        for rows in bad:
            with self.assertRaises(ValueError):
                module.verify_command_delivery(receipt, rows, health)

    def test_log_revision_root_and_generation_must_match_commits(self):
        for field, value in [('revision', 19), ('root_id', 'foreign'), ('access_generation', 12)]:
            receipt, events, health = self.fixture()
            events[1][field] = value
            with self.assertRaises(ValueError):
                module.verify_command_delivery(receipt, events, health)


if __name__ == '__main__':
    unittest.main()
