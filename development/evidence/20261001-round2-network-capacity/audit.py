#!/usr/bin/env python3
"""Linux semantic audit of scoped business-network and physical ENOSPC proof."""
import argparse
import datetime
import json
import pathlib
import platform

NODE = 'd28d17faa8bb71eb8c0a28241e8556396e440b204cf0242d2af8133700f6a494'
META = '64d85fc3933637ccd7fdf4eafded5e5c8cb2354e73b4c8d1a27b2b8d43f4c7e7'
PAYLOAD = '7fa1366968edf9a8490fffac3eab7e0b1619480a152d8e75c7471f753a8c1231'
OLD = '34f3a65c0a388c191f0110ed1b5c204a17bffbc5213f0855579c87b9c1ad74b2'
NEW = 'd4d0bd90fd628a80f43c9ad6c8a97725bd7da3c07676c68e17ecc899a4b1eafc'


def evaluate(root):
    checks = []
    def need(name, value):
        checks.append({'name': name, 'ok': bool(value)})
        assert value, name
    def load(name): return json.loads((root/name).read_text())
    def inner(name):
        value = load(name)
        need(name+' command succeeded', value['exit'] == 0)
        return json.loads(value['stdout'])
    def instant(value): return datetime.datetime.fromisoformat(value.replace('Z', '+00:00'))
    def healthy_read(name, reader, digest=PAYLOAD, size=4194321):
        value = inner(name)
        need(name+' complete correct bytes', value['outcome'] == 'success' and value['status'] == 'PASS'
             and value['bytes'] == size and value['sha256'] == digest and value['elapsed_seconds'] < 30)
        need(name+' exact reader and mounts', value['identity'] == reader
             and {r['source'] for r in value['mounts']} == {'afs-ownerfs', 'afs-dfs'})
        return value

    for node in ('ctl', 'a', 'c'):
        before = load('network/'+node+'/identity-before-reused.json')['identity']
        after = load(f'network/{node}/round2-network-v79b-{node}-identity-final.json')['identity']
        need(node+' original process continuity', before == after and before['sha256'] == (META if node == 'ctl' else NODE))
    prior = inner('network/b/identity-before.json')['identity']
    cold = inner('network/b/identity-cold.json')['identity']
    need('B cold Node before fault', cold['sha256'] == prior['sha256'] == NODE
         and cold['pid'] != prior['pid'] and cold['start_ticks'] != prior['start_ticks']
         and cold['boot_id'] == prior['boot_id'] and cold['exe'] == prior['exe'])
    need('B recovered without another restart', inner('network/b/identity-final.json')['identity'] == cold
         and load('network/b/identity-complete.json')['identity'] == cold)
    jump = load('network/b/jump-install.json')
    remove = load('network/b/jump-remove.json')
    need('actual owned business port', jump['exit'] == remove['exit'] == 0
         and jump['argv'] == ['iptables', '-I', 'OUTPUT', '1', '-d', '192.168.109.12', '-p', 'tcp', '--dport', '19982', '-j', 'AFS_R2_V79B'])
    start, end = instant(jump['utc']), instant(remove['utc'])
    need('bounded fault window', 0 < (end-start).total_seconds() < 90)
    hits = [row.split() for row in load('network/b/iptables-during.json')['stdout'].splitlines() if ' DROP ' in row]
    need('real DROP hits', len(hits) == 1 and int(hits[0][0]) > 0 and int(hits[0][1]) > 0)
    for name, expectation in [('tcp-before.json', 'connected'), ('tcp-blocked.json', 'error'), ('tcp-after.json', 'connected')]:
        value = load('network/b/'+name)
        need(name+' actual endpoint outcome', value['destination'] == '192.168.109.12' and value['port'] == 19982 and value['outcome'] == expectation)
    owner = inner('network/b/owner-blocked.json')
    need('Owner unavailable within operation budget', owner['identity'] == cold and owner['outcome'] == 'error'
         and owner['errno'] == 113 and owner['bytes_before_error'] == 0 and owner['elapsed_seconds'] < 30)
    need('Owner failed inside injected window', start <= instant(owner['utc']) < end)
    dfs = healthy_read('network/b/dfs-during.json', cold)
    need('DFS read inside injected window', start <= instant(dfs['utc']) < end)
    c = healthy_read('network/c/round2-network-v79b-c-during.json', load('network/c/identity-before-reused.json')['identity'])
    need('unaffected Home route during fault', start <= instant(c['utc']) < end)
    recovered = healthy_read('network/b/owner-recovered.json', cold)
    need('Owner recovery after rule removed', instant(recovered['utc']) >= end
         and (instant(recovered['utc'])-end).total_seconds()+recovered['elapsed_seconds'] < 60)
    healthy_read('network/b/dfs-recovered.json', cold)
    after = load('network/b/iptables-after.json')
    need('firewall rollback', after['exit'] == 0 and 'AFS_R2_V79B' not in after['stdout'] and load('network/b/rollback.json')['restored'])
    need('bounded detached watchdog', load('network/b/watchdog.json')['ttl_seconds'] == 90)
    need('network scope not inflated', load('network/b/result.json')['status'] == 'PASS'
         and not load('network/b/result.json')['rdma_link_fault'])

    prep = load('capacity/a/evidence/prepare.json')['prepare']
    config = prep['configuration']
    need('independent physical capacity cohort', '/round2-capacity-v79/volume/node' in config['data_dir']
         and config['meta_endpoint'] == 'https://192.168.109.11:20080' and config['data_mode'] == 'grpc')
    need('replication contract retained', config['dfs_desired_copies'] == 2 and config['dfs_sync_required_copies'] == 1 and config['dfs_local_copy'] == 'required')
    mounts = json.loads(load('capacity/a/evidence/seed.json')['volume']['stdout'])['filesystems']
    need('real ext4 fault filesystem', len(mounts) == 1 and mounts[0]['fstype'] == 'ext4' and mounts[0]['source'].startswith('/dev/loop'))
    seed = load('capacity/a/evidence/seed.json')
    for kind in ('ownerfs', 'dfs'):
        need(kind+' initial watermark', seed[kind]['bytes'] == seed[kind]['length'] == 8192 and seed[kind]['sha256'] == OLD)
    fill = load('capacity/a/evidence/fill.json')
    need('actual physical ENOSPC, not sparse exhaustion', fill['write_errno'] == 28
         and fill['after']['available_bytes'] == 0 and 0 < fill['written_bytes'] <= 128*1024**2
         and fill['allocated_bytes'] >= fill['written_bytes'])
    need('filler automatic restoration armed', fill['watchdog']['ttl_seconds'] == 180
         and fill['watchdog']['owned_filler'].endswith('/volume/owned-capacity-filler'))
    fault_identity = seed['identity']
    for kind in ('ownerfs', 'dfs'):
        fault = load('capacity/a/evidence/fault-'+kind+'.json')
        need(kind+' actual fault Node', fault['identity'] == fault_identity and fault['volume']['available_bytes'] == 0)
        ops = {r['operation']: r for r in fault['operations']}
        need(kind+' all operation outcomes retained', set(ops) == {'write', 'fdatasync', 'fsync', 'close'}
             and all(r['elapsed_seconds'] < 30 for r in ops.values()))
        if kind == 'ownerfs':
            need('Owner no incorrect successful append', ops['write']['outcome'] == 'error' and ops['write']['errno'] == 28)
        else:
            need('DFS write only buffered', ops['write']['outcome'] == 'success' and ops['write']['value'] == 8192)
            for op in ('fdatasync', 'fsync', 'close'):
                need('DFS '+op+' explicit ENOSPC', ops[op]['outcome'] == 'error' and ops[op]['errno'] == 28)
    killed = load('capacity/a/evidence/post-error-kill.json')
    need('dirty visibility is not durable success', killed['identity'] == fault_identity and killed['same_mount_dfs_dirty_view']['sha256'] == NEW
         and killed['volume']['available_bytes'] == 0 and killed['signal'] == 'SIGKILL')
    cold_read = load('capacity/a/evidence/cold-committed-read.json')
    need('cold reload during capacity fault', cold_read['identity']['pid'] != fault_identity['pid']
         and cold_read['identity']['start_ticks'] != fault_identity['start_ticks']
         and cold_read['identity']['boot_id'] == fault_identity['boot_id'] and cold_read['identity']['sha256'] == NODE
         and cold_read['volume']['available_bytes'] == 0)
    for kind in ('ownerfs', 'dfs'):
        need(kind+' original committed content retained', cold_read['reads'][kind]['sha256'] == OLD
             and cold_read['reads'][kind]['bytes'] == cold_read['reads'][kind]['length'] == 8192)
    meta = load('capacity/ctl/evidence/start.json')['identity']
    need('capacity authority continuous', meta == load('capacity/ctl/evidence/identity.json')['identity'] and meta['sha256'] == META)
    recovered = load('capacity/a/evidence/recover.json')
    verified = load('capacity/a/evidence/verify.json')
    need('capacity freed before successful recovery', recovered['before']['available_bytes'] == 0 and recovered['after']['available_bytes'] > 64*1024**2)
    need('successful writes survive later normal restart', recovered['identity']['pid'] != verified['identity']['pid']
         and recovered['identity']['boot_id'] == verified['identity']['boot_id'])
    for kind in ('ownerfs', 'dfs'):
        need(kind+' recovered durable bytes', recovered[kind]['sha256'] == verified[kind]['sha256'] == NEW
             and recovered[kind]['bytes'] == verified[kind]['bytes'] == verified[kind]['length'] == 8192)
    for node in ('a', 'ctl'):
        need(node+' capacity process normal stop', load('capacity/'+node+'/evidence/stop.json')['controller']['exit'] == 0
             and 'stopped exit_code=0' in load('capacity/'+node+'/evidence/stop.json')['controller']['stdout'])
    cleanup = load('capacity/a/evidence/cleanup.json')
    need('isolated fault volume cleaned', cleanup['auto_detached'] and cleanup['umount']['exit'] == 0
         and cleanup['host_volume_after']['available_bytes'] >= 4*1024**3)
    need('physical data remains inspectable', len(cleanup['physical']) >= 4 and all(len(r['sha256']) == 64 for r in cleanup['physical']))
    return checks


if __name__ == '__main__':
    assert platform.system() == 'Linux'
    parser = argparse.ArgumentParser(); parser.add_argument('root', type=pathlib.Path); args = parser.parse_args()
    print(json.dumps({'status': 'PASS', 'level': 'scoped fault integration', 'checks': evaluate(args.root),
                      'round2_complete': False, 'formal_acceptance': 'NOT_RUN', 'environment': 'PREPARING'}, indent=2))
