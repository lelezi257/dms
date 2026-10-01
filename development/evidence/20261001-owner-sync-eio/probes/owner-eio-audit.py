#!/usr/bin/env python3
"""Audit retained physical EIO results, without promoting formal cases."""
import argparse
import copy
import json
import pathlib
import platform


def read(root, name):
    return json.loads((root/name).read_text())


def collect(root):
    files = ['original/a/evidence/fault-ownerfs.json', 'original/a/evidence/fault-dfs.json',
             'original/a/evidence/fault-observed.json', 'original/a/evidence/verify-old.json',
             'fixed/a/evidence/fault-observed.json', 'fixed/a/evidence/fault-ownerfs.json',
             'fixed/a/evidence/fault-dfs.json', 'fixed/a/evidence/verify-old.json',
             'fixed/a/evidence/recover-after-session.json', 'fixed/a/evidence/verify-new-final.json',
             'fixed/a/evidence/start.json', 'fixed/a/evidence/cold-start.json',
             'fixed/a/evidence/recovered-restart.json', 'fixed/a/evidence/prepare.json',
             'fixed/ctl/evidence/start.json', 'fixed/ctl/evidence/identity.json',
             'fixed/b/evidence/remote-result.json', 'fixed/a/evidence/resize-cold-first.json',
             'fixed/b/evidence/resize-healthy.json', 'fixed/b/evidence/cold-read-final.json',
             'fixed/a/evidence/fault-remote-observed.json', 'fixed/a/evidence/cleanup-completed.json',
             'fixed/a/evidence/binding-final.json', 'fixed/b/evidence/binding-final.json',
             'fixed/ctl/evidence/binding-after-remote.json', 'build/artifacts/binary-sha256.json']
    return {name: read(root, name) for name in files}


def evaluate(records):
    checks = []
    def check(name, condition): checks.append({'check': name, 'pass': bool(condition)})
    def get(name): return records[name]
    old = get('original/a/evidence/fault-ownerfs.json')['operations']
    check('original-accepted-append', old[0].get('value') == 8192)
    check('original-two-sync-EIO', all(op.get('errno') == 5 for op in old[1:3]))
    check('original-false-close-preserved', old[3]['outcome'] == 'success')
    check('original-unconfirmed-tail-lost', get('original/a/evidence/verify-old.json')['ownerfs']['unconfirmed_tail_bytes'] == 0)
    expected = '00a5396f4984b249c3197e9d41b8973b138ee8dce98ff19666b1d42f15b6b3ae'
    updated = 'c81a3903d7bfda29197036462a4efc7672dbb07246c7cf72369d50aca0b197f7'
    for cohort in ('original', 'fixed'):
        raw = get(cohort+'/a/evidence/fault-observed.json')
        check(cohort+'-actual-dm-error', ' error' in raw['activate'][-1]['stdout'])
        check(cohort+'-kernel-EIO', 'I/O error' in raw['kernel']['stdout'])
        if cohort=='original': check('original-readonly-observed', 'Remounting filesystem read-only' in raw['kernel']['stdout'])
        check(cohort+'-device-restored', ' linear ' in raw['restore'][-1]['stdout'])
    for kind in ('ownerfs', 'dfs'):
        ops = get('fixed/a/evidence/fault-'+kind+'.json')['operations']
        check(kind+'-accepted-ordinary-write', ops[0].get('value') == 8192)
        for op in ops[1:]: check(kind+'-'+op['operation']+'-EIO', op.get('errno') == 5)
        cold = get('fixed/a/evidence/verify-old.json')[kind]
        check(kind+'-cold-acknowledged-watermark', cold.get('sha256', cold.get('acknowledged_prefix_sha256')) == expected)
        for phase in ('recover-after-session', 'verify-new-final'):
            good = get('fixed/a/evidence/'+phase+'.json')[kind]
            check(kind+'-'+phase+'-new-bytes', good['bytes'] == good['length'] == 8192 and good['sha256'] == updated)
    remote = get('fixed/b/evidence/remote-result.json')
    check('retained-overstrict-resize-fixture-failure', remote['status']=='FAIL' and remote['resize_close']['outcome']=='success')
    for mode in ('plain', 'dsync'):
        for op in remote[mode][1:]: check('remote-'+mode+'-'+op['operation']+'-EIO', op.get('errno') == 5)
    check('remote-resize-accepted', remote['resize_before_fault']['outcome']=='success')
    size=get('fixed/a/evidence/resize-cold-first.json')
    check('remote-resize-close-ack-cold-size', size['length']==size['bytes']==16384 and size['zero_tail'])
    remote_fault=get('fixed/a/evidence/fault-remote-observed.json')
    check('remote-actual-dm-error', ' error' in remote_fault['activate'][-1]['stdout'])
    check('remote-kernel-EIO', 'I/O error' in remote_fault['kernel']['stdout'])
    check('remote-device-restored', ' linear ' in remote_fault['restore'][-1]['stdout'])
    check('remote-errors-in-fault-window', remote_fault['activated_utc']<=remote['triggered_utc']<=remote_fault['restored_utc'])
    for file in ('fixed/b/evidence/resize-healthy.json','fixed/b/evidence/cold-read-final.json'):
        value=get(file)
        value=value.get('remote-size.bin',value)
        check(file+'-confirmed-size', value['bytes']==value['length']==24576 and value['sha256']=='bf5df0b8e9ddfca355230f44fc6742239911084d34647f28091840f1abda4ea2')
    check('remote-dsync-write-EIO', remote['dsync'][0].get('errno') == 5)
    before = get('fixed/ctl/evidence/start.json')['identity']
    after = get('fixed/ctl/evidence/identity.json')['identity']
    check('continuous-memory-authority', before == after)
    identities = [get('fixed/a/evidence/'+phase+'.json')['identity'] for phase in ('start', 'cold-start', 'recovered-restart')]
    check('three-distinct-node-incarnations', len({(i['pid'], i['start_ticks']) for i in identities}) == 3)
    check('same-fixed-node-binary', len({i['sha256'] for i in identities}) == 1)
    check('same-guest-boot', len({i['boot_id'] for i in identities}) == 1)
    cfg = get('fixed/a/evidence/prepare.json')['prepare']
    check('small-explicit-volume', cfg['image_bytes'] == 33554432)
    check('retained-data-reserve', cfg['host_volume_before']['available_bytes'] >= 4*1024**3 and cfg['host_volume_after']['available_bytes'] >= 4*1024**3)
    check('grpc-capacity-scope', cfg['configuration']['data_mode'] == 'grpc')
    binaries=get('build/artifacts/binary-sha256.json')
    for role, file in [('node','fixed/a/evidence/binding-final.json'),('node','fixed/b/evidence/binding-final.json'),('meta','fixed/ctl/evidence/binding-after-remote.json')]:
        binding=get(file)
        check(file+'-actual-binary', binding['identity']['sha256']==binaries[role])
        check(file+'-exact-config', binding['argv'][1:]==['--config',{'fixed/a/evidence/binding-final.json':'/mnt/lima-afsadata/afs-delivery/round2-eio-v80-r2/etc/node.toml','fixed/b/evidence/binding-final.json':'/mnt/lima-afsbdata/afs-delivery/owner-eio-v80-r2-b/etc/node.toml','fixed/ctl/evidence/binding-after-remote.json':'/mnt/lima-afsctlstate/afs-delivery/round2-eio-v80-r2/etc/meta.toml'}[file]])
        check(file+'-ready',binding['health']['status']=='ready')
        if role=='node':check(file+'-grpc-only',binding['configuration']['data_mode']=='grpc')
        else:check('meta-memory-only',binding['configuration']['meta_store']=='memory')
    cleanup=get('fixed/a/evidence/cleanup-completed.json')
    check('cleanup-completed',cleanup['status']=='PASS' and all(c['exit']==0 for c in cleanup['commands']))
    check('cleanup-reserve',cleanup['host_volume_after']['available_bytes']>=4*1024**3)
    return checks


def selftest(records):
    results = []
    assert all(c['pass'] for c in evaluate(records))
    variants = [
        ('false-fixed-close', lambda r: r['fixed/a/evidence/fault-ownerfs.json']['operations'][3].update(outcome='success', errno=None)),
        ('no-physical-fault', lambda r: r['fixed/a/evidence/fault-observed.json']['activate'][-1].update(stdout='linear')),
        ('changed-authority', lambda r: r['fixed/ctl/evidence/identity.json']['identity'].update(pid=-1)),
        ('corrupt-cold-watermark', lambda r: r['fixed/a/evidence/verify-old.json']['dfs'].update(sha256='wrong')),
        ('wrong-live-binary', lambda r: r['fixed/b/evidence/binding-final.json']['identity'].update(sha256='wrong')),
        ('wrong-live-config', lambda r: r['fixed/b/evidence/binding-final.json'].update(argv=['node','--config','unrelated/node.toml'])),
        ('false-cold-resize-ack', lambda r: r['fixed/a/evidence/resize-cold-first.json'].update(length=8192)),
        ('hidden-failed-original', lambda r: r['original/a/evidence/fault-ownerfs.json']['operations'][3].update(outcome='error'))]
    for name, mutate in variants:
        candidate = copy.deepcopy(records); mutate(candidate)
        rejected = any(not c['pass'] for c in evaluate(candidate))
        assert rejected, name
        results.append({'case': name, 'rejected': rejected})
    return results


if __name__ == '__main__':
    assert platform.system() == 'Linux'
    parser = argparse.ArgumentParser(); parser.add_argument('root', type=pathlib.Path); parser.add_argument('--selftest', action='store_true')
    args = parser.parse_args(); records = collect(args.root)
    checks = evaluate(records)
    output = {'status': 'PASS' if all(c['pass'] for c in checks) else 'FAIL', 'checks': checks,
              'formal_acceptance': 'NOT_RUN', 'level': 'SCOPED_FAULT_INTEGRATION', 'unproved': ['forced readonly remount in fixed local fault','resize-only close EIO after data already journaled','full backend/transport/device matrix']}
    trace=(args.root/'fixed/a/evidence/resize-close.strace').read_text()
    checks.append({'check':'actual-Home-close-fdatasync','pass':any('fdatasync(' in line and '/remote-size.bin>' in line and '= 0' in line for line in trace.splitlines())})
    output['status']='PASS' if all(c['pass'] for c in checks) else 'FAIL'
    if args.selftest: output['negative_tests'] = selftest(records)
    print(json.dumps(output, indent=2))
    assert output['status'] == 'PASS'
