#!/usr/bin/env python3
"""Linux evidence checks for the short cross-VM OwnerFs production flow."""
import argparse
import hashlib
import json
import pathlib
import platform
import re

SIZE = 4194321
ORIGINAL = '7fa1366968edf9a8490fffac3eab7e0b1619480a152d8e75c7471f753a8c1231'
PATCHED = '33752442cf488a550283e95caed9fe804542daf32d6fcdda6df43c11e68f2d9b'


def totals(log):
    result = {'READ': 0, 'WRITE': 0}
    for direction, value in re.findall(r'^AFS_RDMA_COMPLETE op=(READ|WRITE) bytes=(\d+)$', log, re.MULTILINE):
        count = int(value)
        assert 0 < count <= 1048576
        result[direction] += count
    assert not re.search(r'AFS_RDMA_(ERROR|TEARDOWN)|completion (failed|timeout)', log)
    return result


def metric(text, name, **labels):
    found = []
    for line in text.splitlines():
        if not line.startswith(name + '{'):
            continue
        header, value = line.rsplit(' ', 1)
        actual = dict(re.findall(r'(\w+)="([^"]*)"', header))
        if actual == labels:
            found.append(int(value))
    assert len(found) == 1, (name, labels, found)
    return found[0]


def payload(text, side, direction, plane):
    return metric(text, 'afs_ownerfiles_payload_bytes_total', side=side,
                  direction=direction, plane=plane)


def prove(rows):
    write, read = rows['write-b.json'], rows['read-b-cold.json']
    patch, read2 = rows['patch-b.json'], rows['read-b-patched-cold.json']
    server1, server2, server3 = [rows[x] for x in ('collect-a-after-write.json', 'collect-a-after-read.json', 'collect-a-final.json')]
    client = rows['collect-b-final.json']
    for receipt, expected in ((write, ORIGINAL), (read, ORIGINAL), (patch, PATCHED), (read2, PATCHED)):
        assert receipt['bytes'] == SIZE and receipt['sha256'] == expected
        assert receipt['home']['home_node_id'] == 'owner-metrics-node-a'
        assert receipt['home']['home_serving']
        assert receipt['home']['root_id'] == 'root-6f776e65722d6d6574726963732d763733'
        assert receipt['home']['home_grpc_addr'] == 'https://192.168.109.12:18882'
        assert receipt['identity']['boot_id'] == client['boot_id']
        assert receipt['identity']['boot_id'] != server3['boot_id']
        assert receipt['identity']['configs'] == client['configs']
    processes = [receipt['identity']['processes']['node'] for receipt in (write, read, read2)]
    assert len({(p['pid'], p['start_ticks']) for p in processes}) == 3
    assert len({p['sha256'] for p in processes}) == 1
    assert patch['identity']['processes'] == read['identity']['processes']
    assert read2['identity']['processes'] == client['processes']
    for server in (server1, server2, server3):
        assert server['processes'] == server3['processes']
        assert server['configs'] == server3['configs']
        assert server['boot_id'] == server3['boot_id']
        assert len(server['physical_files']) == 1
        assert server['physical_files'][0]['bytes'] == SIZE
        expected = PATCHED if server is server3 else ORIGINAL
        assert server['physical_files'][0]['sha256'] == expected
        assert server['physical_files'][0]['path'].endswith('/root-6f776e65722d6d6574726963732d763733-e1/data.bin')
        assert 'afs_ownerfiles_rpc_duration_seconds_count{method="OwnerFiles.Write",side="server"}' in server['metrics']
    assert client['physical_files'] == []
    assert 'afs_ownerfiles_rpc_duration_seconds_count{method="open",side="client"}' in client['metrics']
    phase1, phase2, phase3 = [totals(s['node_log']) for s in (server1, server2, server3)]
    assert phase1 == {'READ': SIZE, 'WRITE': 0}, phase1
    assert phase2 == {'READ': SIZE, 'WRITE': SIZE}, phase2
    assert phase3 == {'READ': SIZE + 4096, 'WRITE': SIZE * 2}, phase3
    for server, written, read_bytes in ((server1, SIZE, 0), (server2, SIZE, SIZE),
                                        (server3, SIZE + 4096, SIZE * 2)):
        assert payload(server['metrics'], 'server', 'write', 'rdma') == written
        assert payload(server['metrics'], 'server', 'read', 'rdma') == read_bytes
        for direction in ('write', 'read'):
            assert payload(server['metrics'], 'server', direction, 'grpc') == 0
    initial = rows['collect-b-after-write.json']
    middle = rows['collect-b-after-patch.json']
    assert initial['processes'] == write['identity']['processes']
    assert middle['processes'] == patch['identity']['processes']
    for observation, write, read in ((initial, SIZE, 0), (middle, 4096, SIZE),
                                    (client, 0, SIZE)):
        assert payload(observation['metrics'], 'client', 'write', 'rdma') == write
        assert payload(observation['metrics'], 'client', 'read', 'rdma') == read
        for direction, amount in (('write', write), ('read', read)):
            assert payload(observation['metrics'], 'client', direction, 'grpc') == 0
            if amount:
                assert metric(observation['metrics'], 'afs_ownerfiles_rpc_duration_seconds_count',
                              side='client', method=direction) > 0
    for observation in (server1, server2, server3, client, initial, middle):
        assert set(observation['owned_resources']) == {'qp', 'cq', 'mr', 'pd', 'ctx'}
        assert not any(observation['owned_resources'].values())
    return {'content_and_eof': 'PASS', 'distinct_vms_and_cold_restart': 'PASS',
            'home_physical_bytes': 'PASS', 'actual_server_verbs': phase3,
            'idle_owned_resources': 'PASS', 'endpoint_payload_metrics': 'PASS',
            'rdma_client_timers': 'PASS'}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('evidence')
    parser.add_argument('qualified_inputs')
    parser.add_argument('probe')
    args = parser.parse_args()
    assert platform.system() == 'Linux'
    root = pathlib.Path(args.evidence)
    names = ('write-b.json', 'read-b-cold.json', 'patch-b.json', 'read-b-patched-cold.json',
             'collect-a-after-write.json', 'collect-a-after-read.json', 'collect-a-final.json',
             'collect-b-after-write.json', 'collect-b-after-patch.json', 'collect-b-final.json')
    rows = {name: json.loads((root / name).read_text()) for name in names}
    checks = prove(rows)
    inputs = json.loads((root / 'build-inputs.json').read_text())
    frozen = json.loads(pathlib.Path(args.qualified_inputs).read_text())
    assert inputs['files'] == frozen['files'] and inputs['file_count'] == 143
    assert inputs['rustc'] == frozen['rustc']
    artifact_hashes = {parts[1]: parts[0]
                       for line in (root / 'artifacts.txt').read_text().splitlines()
                       if len(parts := line.split()) == 2 and re.fullmatch('[0-9a-f]{64}', parts[0])}
    node_sha = artifact_hashes['/home/lzc.guest/afs-build/artifacts/owner-metrics-v73-r2/afs-node']
    meta_sha = artifact_hashes['/home/lzc.guest/afs-build/artifacts/owner-metrics-v73-r2/afs-meta']
    for which in ('a', 'b'):
        assert rows[f'collect-{which}-final.json']['processes']['node']['sha256'] == node_sha
    assert rows['collect-a-final.json']['processes']['meta']['sha256'] == meta_sha
    for phase in ('restart-b.log', 'restart-b-after-patch.log', 'stopped-b.txt', 'stopped-a.txt'):
        text = (root / phase).read_text()
        assert 'node stopped exit_code=0' in text
        if phase == 'stopped-a.txt':
            assert 'meta stopped exit_code=0' in text
    for which in ('a', 'b'):
        observation = rows[f'collect-{which}-final.json']
        proc = observation['processes']['node']
        saved = (root / f'stopped-{which}.txt').read_text()
        run = observation['run']
        for line in (f'pid={proc["pid"]}', f'start_ticks={proc["start_ticks"]}',
                     f'boot_id={observation["boot_id"]}',
                     f'cmdline={run}/prefix/bin/afs-node --config {run}/etc/node.toml'):
            assert line + '\n' in saved
        stopped = json.loads((root / f'stopped-{which}.json').read_text())
        assert stopped['run'] == run and stopped['boot_id'] == observation['boot_id']
        assert stopped['mount_absent'] and not any(stopped['owned_retained'].values())
        for role, original in observation['processes'].items():
            receipt = stopped['receipts'][role]
            assert receipt['exit_code'] == '0' and receipt['pid'] == str(original['pid'])
            assert receipt['start_ticks'] == original['start_ticks']
    for name in ('write-b.stderr', 'read-b-cold.stderr', 'patch-b.stderr', 'read-b-patched-cold.stderr'):
        assert (root / name).read_text() == ''
    assert 'Ran 7 tests' in (root / 'script-tests-final.log').read_text()
    hashes = {str(path.relative_to(root)): hashlib.sha256(path.read_bytes()).hexdigest()
              for path in sorted(root.rglob('*')) if path.is_file() and path.name != 'audit-report.json'
              and '__pycache__' not in path.parts}
    assert not any(part.startswith('._') for name in hashes for part in pathlib.Path(name).parts)
    print(json.dumps({'status': 'PASS', 'level': 'STAGE_INTEGRATION',
                      'checks': checks, 'qualified_source_inputs': 143, 'files': hashes,
                      'probe_sha256': hashlib.sha256(pathlib.Path(args.probe).read_bytes()).hexdigest(),
                      'formal': '69 NOT_RUN / ENV PREPARING',
                      'metrics_scope': 'Successful logical bytes at each endpoint; server verbs logs '
                                       'independently establish actual payload DMA.',
                      'limits': 'Short healthy production flow and idle resources; not posted-DMA cancellation, '
                                'provider teardown faults, Meta restart, disk loss, performance or soak.'}, indent=2))


if __name__ == '__main__':
    main()
