import copy
import importlib.util
import json
import pathlib
import platform
import unittest

root = pathlib.Path(__file__).parent
spec = importlib.util.spec_from_file_location('audit', root / 'audit.py')
audit = importlib.util.module_from_spec(spec)
spec.loader.exec_module(audit)
names = ('write-b.json', 'read-b-cold.json', 'patch-b.json', 'read-b-patched-cold.json',
         'collect-a-after-write.json', 'collect-a-after-read.json', 'collect-a-final.json',
         'collect-b-after-write.json', 'collect-b-after-patch.json', 'collect-b-final.json')


class EvidenceRejection(unittest.TestCase):
    def setUp(self):
        self.rows = {name: json.loads((root / name).read_text()) for name in names}

    def test_retained_real_flow(self):
        self.assertEqual(audit.prove(self.rows)['content_and_eof'], 'PASS')

    def rejected(self, name, mutate):
        altered = copy.deepcopy(self.rows)
        mutate(altered[name])
        with self.assertRaises(AssertionError):
            audit.prove(altered)

    def test_changed_physical_bytes(self):
        self.rejected('collect-a-final.json', lambda row: row['physical_files'][0].update(sha256='0' * 64))

    def test_no_new_reader_process(self):
        self.rejected('read-b-cold.json', lambda row: row['identity']['processes'].update(self.rows['write-b.json']['identity']['processes']))

    def test_same_guest(self):
        self.rejected('collect-a-final.json', lambda row: row.update(boot_id=self.rows['collect-b-final.json']['boot_id']))

    def test_missing_read_dma_completion(self):
        self.rejected('collect-a-final.json', lambda row: row.update(node_log=row['node_log'].replace('AFS_RDMA_COMPLETE op=WRITE bytes=17', 'missing-completion', 1)))

    def test_wrong_home(self):
        self.rejected('write-b.json', lambda row: row['home'].update(home_node_id='owner-metrics-node-b'))

    def test_retained_idle_resource(self):
        self.rejected('collect-b-final.json', lambda row: row['owned_resources']['mr'].append({'pid': row['processes']['node']['pid']}))

    def test_absent_payload_counters(self):
        self.rejected('collect-a-final.json', lambda row: row.update(metrics='\n'.join(
            line for line in row['metrics'].splitlines()
            if not line.startswith('afs_ownerfiles_payload_bytes_total'))))

    def test_absent_rdma_client_timer(self):
        self.rejected('collect-b-final.json', lambda row: row.update(metrics='\n'.join(
            line for line in row['metrics'].splitlines()
            if not (line.startswith('afs_ownerfiles_rpc_duration_seconds_count{')
                    and 'method="read"' in line and 'side="client"' in line))))

    def test_wrong_middle_process(self):
        self.rejected('collect-b-after-patch.json', lambda row: row.update(
            processes=self.rows['collect-b-after-write.json']['processes']))


if __name__ == '__main__':
    if platform.system() != 'Linux':
        raise SystemExit('evidence validation runs in Linux')
    unittest.main()
