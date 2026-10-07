import importlib.util
import json
from pathlib import Path
import tomllib
import unittest
from unittest import mock

spec = importlib.util.spec_from_file_location('bind_epoch', Path(__file__).with_name('workspace-bind-epoch-linux.py'))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


def health(session, epoch):
    return dict(status='ready', id='node-a', checks=dict(node_registration=dict(
        session_id=session, lease_epoch=epoch, ready=True)))


class EpochCaseGuards(unittest.TestCase):
    def test_failed_node_receipt_does_not_leave_owned_meta_running_or_erase_failure(self):
        run = module.Run.__new__(module.Run)
        run.ctl = mock.Mock(side_effect=[RuntimeError('node failed exit_code=1'), 'meta stopped exit_code=0'])
        result = dict(status='FAIL', error='authority-error-preserved')
        run.close_remaining(result)
        self.assertEqual(run.ctl.call_args_list, [mock.call('stop', 'node'), mock.call('stop', 'meta')])
        self.assertEqual(result['status'], 'FAIL')
        self.assertEqual(result['error'], 'authority-error-preserved')
        self.assertEqual(result['node_cleanup_error'], 'node failed exit_code=1')
        self.assertNotIn('meta_cleanup_error', result)

    def test_replacement_keeps_identity_but_isolates_resources_and_disables_bind(self):
        text = '\n'.join(f'{key} = "{value}"' for key, value in dict(
            id='node-a', fs='ownerfs', grpc_listen='old', rest_listen='old',
            advertise_endpoint='old', data_dir='/old/data', uds_path='/old/sock',
            ownerfs_mount='/old/fuse', tls_identity_certificate='/old/cert',
            tls_identity_private_key='/old/key').items())
        text += '\nexperimental_ownerfs_workspace_bind = true\n[ownerfs_workspace_bind]\nworkspace = "workspace"\n'
        result = tomllib.loads(module.replacement_config(text, Path('/opt/case/replacement')))
        self.assertEqual(result['id'], 'node-a')
        self.assertEqual(result['tls_identity_certificate'], '/old/cert')
        self.assertEqual(result['tls_identity_private_key'], '/old/key')
        self.assertEqual(result['data_dir'], '/opt/case/replacement/state')
        self.assertEqual(result['uds_path'], '/opt/case/replacement/run/node.sock')
        self.assertEqual(result['fs'], 'dfs')
        self.assertEqual(result['dfs_mount'], '/opt/case/replacement/mount/dfs')
        self.assertNotIn('ownerfs_mount', result)
        self.assertFalse(result['experimental_ownerfs_workspace_bind'])
        self.assertFalse(result['experimental_native_workspace'])
        self.assertNotIn('ownerfs_workspace_bind', result)

    def test_replacement_rejects_missing_runtime_paths(self):
        with self.assertRaisesRegex(ValueError, 'missing/duplicate'):
            module.replacement_config('id = "node-a"', Path('/opt/case/replacement'))

    def test_public_transition_accepts_fresh_session_and_higher_epoch(self):
        module.verify_transition(health('original', 2), health('replacement', 3))

    def test_public_transition_rejects_renewal_other_node_and_degraded_state(self):
        before = health('original', 2)
        for after in (health('original', 3), health('replacement', 2), health('replacement', True),
                      dict(health('replacement', 3), id='other'), dict(health('replacement', 3), status='degraded')):
            with self.subTest(after=after), self.assertRaises(ValueError):
                module.verify_transition(before, after)

    def test_authority_failure_requires_exact_nonzero_incarnation(self):
        installed = '/opt/case/prefix/bin/afs-node'
        child = dict(pid='12', exe=installed, config='/opt/case/etc/node.toml', start_ticks='45',
                     boot_id='boot', lifecycle='/opt/case/run/node.lifecycle.1', supervisor_pid='11')
        observed = dict(pid=12, starttick=45, boot_id='boot', installed=dict(path=installed))
        ready = dict(supervisor_pid='11')
        receipt = dict(child, exit_code='1')
        module.verify_authority_exit(receipt, child, ready, observed, True)
        self.assertEqual(receipt['exit_code'], '1')
        for code in ('0', '124', '137', '143'):
            with self.subTest(code=code), self.assertRaises(ValueError):
                module.verify_authority_exit(dict(receipt, exit_code=code), child, ready, observed, True)
        with self.assertRaises(ValueError):
            module.verify_authority_exit(dict(receipt, start_ticks='46'), child, ready, observed, True)
        with self.assertRaises(ValueError):
            module.verify_authority_exit(receipt, child, ready, observed, False)

    def test_replacement_closure_checks_its_separate_config_and_actual_wait(self):
        child = dict(pid='22', exe='/opt/case/prefix/bin/afs-node', config='/opt/case/replacement/etc/node.toml',
                     start_ticks='55', boot_id='boot', lifecycle='/opt/case/replacement/run/node.lifecycle.2', supervisor_pid='21')
        observed = dict(pid=22, starttick=55, boot_id='boot', installed=dict(path=child['exe']))
        ready, receipt = dict(supervisor_pid='21'), dict(child, exit_code='0')
        config = Path(child['config'])
        module.verify_replacement_exit(receipt, child, ready, observed, True, config)
        for bad in (dict(receipt, exit_code='1'), dict(receipt, config='/old/config'), dict(receipt, pid='23')):
            with self.subTest(bad=bad), self.assertRaises(ValueError):
                module.verify_replacement_exit(bad, child, ready, observed, True, config)

    def test_authority_log_accepts_real_structured_error_and_rejects_other_failures(self):
        error = ('0x04030002 PermissionDenied: Node registration epoch changed; '
                 'the running authority must stop')
        event = dict(msg='node.shutdown_failed', error=error)
        module.verify_authority_log(json.dumps(event) + '\nError: terminal failure\n')
        for wrong in (dict(event, msg='unrelated'), dict(event, error='permission denied'),
                      dict(event, error=error.replace('the running', 'running'))):
            with self.subTest(wrong=wrong), self.assertRaises(ValueError):
                module.verify_authority_log(json.dumps(wrong))


if __name__ == '__main__':
    unittest.main()
