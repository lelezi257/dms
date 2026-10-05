#!/usr/bin/env python3
"""Offline observer. Retains semantic failures; never declares release PASS."""
import argparse
import copy
import hashlib
import json
from pathlib import Path
import tarfile


def sha(data): return hashlib.sha256(data).hexdigest()


def raw_inputs(directory,result):
    raw={}
    for role in ('a','b','ctl'):
        file=directory/(role+'-raw.tar.gz')
        assert sha(file.read_bytes())==result['raw_sha256'][role],role
        with tarfile.open(file) as tar:
            files={member.name.removeprefix('./'):tar.extractfile(member).read()
                   for member in tar.getmembers() if member.isfile()}
        for line in files['evidence-files.sha256'].decode().splitlines():
            digest,name=line.split('  ',1)
            assert sha(files[name])==digest,(role,name)
        assert json.loads(files['node-exit.json'])['exit']==0
        cfg=json.loads(files['role.json'])
        for name,digest in cfg['inputs'].items():
            if name in files: assert sha(files[name])==digest,(role,name)
            assert result['input_sha256'][name]==digest
        raw[role]=files
    return raw


def architecture(result,raw,trace):
    assert result['passed'] and result['case_profile']=='architecture-closeout'
    assert not result['inflight_before_mount']['reply_exists']
    assert not result['inflight_after_mount']['reply_exists']
    assert result['inflight_reply']['ok'] and result['inflight_reply']['result']==15
    assert result['activated']['state']=='NativeActive'
    assert result['prepared']['source']==result['activated']['source']
    assert result['inflight_trace_stop']['delayed']
    assert '"transition-data", 15, 0) = 15 (DELAYED)' in result['inflight_trace_stop']['trace']
    for role in ('a','b'):
        matrix=result['syscall_matrix'][role]
        assert matrix==json.loads(raw[role]['syscall-results.json'])
        for lane,operations in matrix['operations'].items():
            assert operations['private']['ok'] and operations['private']['file_bytes']=='AAAA'
            assert operations['mode_xattr']['ok'] and operations['mode_xattr']['mode']==0o640
            assert operations['symlink']['ok']
            if lane=='native': assert operations['shared']['ok'] and operations['shared']['file_bytes']=='MMAP'
            else: assert not operations['shared']['ok'] and operations['shared']['errno']==19
    watches=result['syscall_matrix']['a']['watch']
    assert any(row['name']=='fuse-watch-control' for row in watches['positive_control']['oldfuse'])
    assert any(row['name']=='native-watch-control' for row in watches['native_mutation']['native'])
    assert not watches['native_event_on_oldfuse']
    assert result['remote_watch']==json.loads(raw['b']['watch-result.json'])
    assert any(row['name']=='remote-watch-positive' for row in result['remote_watch']['control'])
    assert not result['remote_watch']['native_event_delivered']
    outputs=[json.loads(row['stdout']) for row in trace if row['exit']==0 and row['stdout'].strip().startswith('{')]
    for key in ('inflight_before_mount','inflight_after_mount','inflight_reply','inflight_trace_stop','remote_watch'):
        assert result[key] in outputs,key
    for role in ('a','b'): assert result['syscall_matrix'][role] in outputs
    commands={}
    for row in trace:
        if row['exit']!=0 or not row.get('input') or not row['command'][-1].endswith(' actor-command'): continue
        request=json.loads(row['input']);reply=json.loads(row['stdout'])
        role=next(role for role,ip in result['hosts'].items() if row['command'][-2]=='lzc@'+ip)
        actor=request['actor']
        assert json.loads(raw[role][f"actor-{actor}/reply-{request['id']}.json"])==reply
        commands.setdefault((role,actor),[]).append((request,reply))
    for key in (('a','native'),('a','oldfuse'),('b','remote')):
        entries=commands[key]
        reads=[index for index,(request,reply) in enumerate(entries)
               if request['operation']=='read' and request.get('handle')=='fresh' and reply.get('result')=='transition-data']
        assert len(reads)==1,key
        index=reads[0];assert 0<index<len(entries)-1
        before,after=entries[index-1][0],entries[index+1][0]
        assert before['operation']=='open' and before['name']=='data' and before['handle']=='fresh'
        assert after['operation']=='close' and after['handle']=='fresh'
        assert all(entries[number][1]['ok'] for number in (index-1,index,index+1))
    return dict(pending_across_bind=True,mapping_watch_positive_negative=True,semantic_acceptance=False)


def performance(result,raw,trace):
    assert result['passed'] and result['case_profile']=='performance-closeout'
    p=result['closeout_performance']
    assert p['profile']=='bounded-five-lane-v1' and p['metadata_files']==1000
    assert p['formal_10000_matrix'].startswith('NOT_RUN')
    assert p['sampling_completed'] and p['phases_completed']==['metadata','bulk']
    assert p['rounds']==6 and p['warmup_rounds']==1 and p['measured_rounds']==5
    assert 'unqualified' in p['durability'] and 'uncontrolled' in p['cache_limit']
    lanes={'ext4','native','mfs-local','dms-remote','mfs-remote'}
    assert set(p['setup']['a']['lanes'])=={'ext4','native','mfs-local'}
    assert set(p['setup']['b']['lanes'])=={'dms-remote','mfs-remote'}
    for role in ('a','b'):
        state=json.loads(raw[role]['closeout-performance-state.json'])
        assert state['completed'] and all(row['stopped'] and row['deleted'] for row in state['cleanup'])
        for target,name in (('/benchmark','benchmark'),('/io','io-closeout'),('/container-probe','container-probe')):
            assert state['rootfs_files'][target]['sha256']==result['input_sha256'][name]
        for lane,record in state['lanes'].items():
            assert record['source_path']==p['setup'][role]['lanes'][lane]['source_path']
            assert record['spec']['root']['readonly'] and record['spec']['process']['noNewPrivileges']
            assert all(not values for values in record['spec']['process']['capabilities'].values())
            rows=[line for line in record['mountinfo'].splitlines() if line.split()[4]=='/ownerfs/agent1']
            assert len(rows)==1
            if lane in ('ext4','native'):
                assert record['root_object']==result['activated']['source'] and ' - ext4 ' in rows[0]
            else: assert ' - fuse' in rows[0]
    definition=raw['a']['moosefs-class-definition.txt'].decode()
    assert 'name: native-bind-one' in definition and '\tkeep_labels: *\n' in definition
    runtime=[]
    for role in ('a','b'):
        runtime += [json.loads(line) for line in raw[role]['closeout-runtime.jsonl'].decode().splitlines()]
    replies=[json.loads(row['stdout']) for row in runtime if row['exit']==0 and row['stdout'].strip().startswith('{')]
    expected={(form,c,lane,r) for form in ('absolute','relative') for c in (1,8) for lane in lanes for r in range(6)}
    actual=set()
    for sample in p['metadata_samples']:
        key=(sample['form'],sample['concurrency'],sample['lane'],sample['round'])
        assert key not in actual;actual.add(key)
        m=sample['result'];assert m in replies
        assert sample['warmup']==(sample['round']==0)
        assert m['files']==1000 and m['file_bytes']==4096 and m['concurrency']==sample['concurrency'] and m['path_form']==sample['form']
        assert [phase['name'] for phase in m['phases']]==['create_write_close','stat','read_close','readdir','rename','unlink']
        assert all(phase['wall_ns']>0 and phase['client_cpu_ns']>=0 for phase in m['phases'])
    assert actual==expected
    # Match the fixed diagnostic matrix; do not claim the original full gate.
    from ownerfs_native_closeout_controller import cases
    assert p['case_matrix']==cases()
    expected={(case,lane,r) for case in range(8) for lane in lanes for r in range(6)}
    actual=set();objects={}
    for sample in p['samples']:
        key=sample['case'],sample['lane'],sample['round']
        assert key not in actual;actual.add(key)
        case=p['case_matrix'][sample['case']];m=sample['result'];assert m in replies
        assert sample['warmup']==(sample['round']==0)
        assert all(m[field]==case[field] for field in ('operation','file_bytes','io_bytes','block_bytes','concurrency','barrier'))
        assert m['content_ok'] and m['seed']==257 and m['operations']==case['io_bytes']//case['block_bytes']
        assert m['wall_ns']>0 and 0<=m['barrier_ns']<=m['wall_ns'] and m['client_cpu_ns']>=0
        assert 0<m['p50_ns']<=m['p95_ns']<=m['p99_ns']
        if sample['lane'] in ('ext4','native'):
            assert m['residency_observed'] and m['cache_requested']==case['cache']
            if case['cache']=='guest-cold': assert m['resident_before_bytes']==0
            if case['cache']=='hot': assert m['resident_before_bytes']==case['file_bytes']
            objectkey=case['dataset']
            if objectkey in objects: assert objects[objectkey]==m['file_object']
            objects[objectkey]=m['file_object']
        else:
            assert not m['residency_observed'] and m['cache_requested']=='unobserved'
        if case['cache']=='guest-cold': assert len(sample['cache_preparation'])==2
    assert actual==expected
    assert len(p['dataset_setup'])==6 and all(row['result']['content_ok'] for row in p['dataset_setup'])
    assert all(row['result']['completed'] for row in p['cleanup'])
    return dict(metadata_samples=120,io_samples=240,content_checked=True,cache_and_durability_limits_retained=True,
                performance_acceptance=False)


def main():
    parser=argparse.ArgumentParser()
    parser.add_argument('directory',type=Path)
    parser.add_argument('--self-test',action='store_true')
    args=parser.parse_args()
    result=json.loads((args.directory/'result.json').read_text())
    trace=[json.loads(line) for line in (args.directory/'transcript.jsonl').read_text().splitlines()]
    raw=raw_inputs(args.directory,result)
    verifier=architecture if result['case_profile']=='architecture-closeout' else performance
    observation=verifier(result,raw,trace)
    if args.self_test:
        mutations=[]
        bad=copy.deepcopy(result);bad['passed']=False;mutations.append(bad)
        if verifier==architecture:
            bad=copy.deepcopy(result);bad['inflight_after_mount']['reply_exists']=True;mutations.append(bad)
            bad=copy.deepcopy(result);bad['syscall_matrix']['b']['operations']['remote']['shared']['ok']=True;mutations.append(bad)
            bad=copy.deepcopy(result);bad['remote_watch']['native_event_delivered']=True;mutations.append(bad)
        else:
            bad=copy.deepcopy(result);bad['closeout_performance']['samples'].pop();mutations.append(bad)
            bad=copy.deepcopy(result);bad['closeout_performance']['samples'][0]['result']['wall_ns']=0;mutations.append(bad)
            bad=copy.deepcopy(result);bad['closeout_performance']['durability']='qualified';mutations.append(bad)
        for bad in mutations:
            try: verifier(bad,raw,trace)
            except AssertionError: pass
            else: raise AssertionError('observer accepted false evidence')
        observation['negative_observer_controls']=len(mutations)
    print(json.dumps(dict(checks_ok=True,scope='qualified investigation evidence only',**observation)))


if __name__=='__main__': main()
