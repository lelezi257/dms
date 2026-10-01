import importlib.util
import io
import json
import pathlib
import platform
import tomllib
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('owner_runtime', pathlib.Path(__file__).with_name('owner-metrics-runtime-v73.py'))
runtime = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runtime)


class ProbeContracts(unittest.TestCase):
    def node_template(self, which):
        return f"""id = 'memory-node-{which}'
fs = 'all'
meta_endpoint = 'https://192.168.109.12:18280'
data_dir = '{runtime.TEMPLATE[which]}/state/node'
ownerfs_mount = '{runtime.TEMPLATE[which]}/mount-ownerfs'
dfs_mount = '{runtime.TEMPLATE[which]}/mount-dfs'
data_mode = 'grpc'
trusted_node_certs = {{ memory-node-a = '/a.pem', memory-node-b = '/b.pem' }}
"""

    def test_both_node_configs_isolate_and_require_rdma(self):
        for which in ('a', 'b'):
            cfg = tomllib.loads(runtime.config(which, 'node', self.node_template(which)))
            self.assertEqual(cfg['fs'], 'ownerfs')
            self.assertNotIn('dfs_mount', cfg)
            self.assertEqual(cfg['data_mode'], 'rdma')
            self.assertEqual(cfg['rdma_device'], 'rxe0')
            self.assertEqual(cfg['id'], 'owner-metrics-node-' + which)
            self.assertEqual(cfg['meta_endpoint'], 'https://192.168.109.12:18880')
            self.assertTrue(cfg['ownerfs_mount'].startswith(runtime.RUN[which] + '/'))

    def test_meta_requires_memory(self):
        text = self.node_template('a') + "meta_store = 'memory'\n"
        cfg = tomllib.loads(runtime.config('a', 'meta', text))
        self.assertEqual(cfg['id'], 'owner-metrics-meta')
        with self.assertRaises(AssertionError):
            runtime.config('a', 'meta', text.replace("'memory'", "'etcd'"))

    def test_scalar_is_unique_and_parseable(self):
        text = runtime.scalar("data_mode = 'grpc'\n", 'data_mode', "'rdma'")
        text = runtime.scalar(text, 'rdma_device', "'rxe0'")
        self.assertEqual(text.count('data_mode ='), 1)
        self.assertEqual(tomllib.loads(text)['rdma_device'], 'rxe0')

    def test_existing_runtime_rejected_before_any_command(self):
        with patch.object(pathlib.Path, 'exists', return_value=True), patch.object(runtime, 'command') as command:
            with self.assertRaises(AssertionError):
                runtime.prepare('a')
            command.assert_not_called()

    def test_file_io_rejects_home_origin(self):
        with self.assertRaises(AssertionError):
            runtime.file_action('a', 'write')

    def test_home_receipt_uses_encoded_root_identity(self):
        reply = {'home_node_id': 'owner-metrics-node-a', 'home_serving': True, 'home_grpc_addr': 'https://192.168.109.12:18882'}
        with patch.object(runtime.urllib.request, 'ProxyHandler') as proxy, patch.object(runtime.urllib.request, 'build_opener') as build:
            request = build.return_value.open
            request.return_value = io.StringIO(json.dumps(reply))
            self.assertEqual(runtime.rest(), reply)
            proxy.assert_called_once_with({})
            request.assert_called_once_with('http://192.168.109.12:18881/v1/roots/root-' + runtime.WORKSPACE.encode().hex(), timeout=10)

    def test_patch_preserves_length_and_changes_only_range(self):
        old = runtime.payload()
        new = runtime.payload(True)
        start = 1024 * 1024 - 13
        self.assertEqual(len(new), 4 * 1024 * 1024 + 17)
        self.assertEqual(old[:start], new[:start])
        self.assertEqual(new[start:start + 4096], b'P' * 4096)
        self.assertEqual(old[start + 4096:], new[start + 4096:])


if __name__ == '__main__':
    if platform.system() != 'Linux':
        raise SystemExit('probe regression runs in Linux')
    unittest.main()
