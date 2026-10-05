#!/usr/bin/env python3
"""Five real OCI lanes, paired fixed workloads; no tuning or gate assertion."""
import json
from pathlib import Path
import time


def cases():
    # Fixed finite five-way diagnostics. Original 30-shape/8GiB gate is unchanged.
    seq,hot=64*1024**2,16*1024**2
    shapes=[('seq-read',1048576,1,'guest-cold','close','seq',seq),
            ('seq-read',1048576,8,'hot','close','seq',seq),
            ('seq-write',1048576,1,'guest-cold','close','seq',seq),
            ('seq-write',1048576,8,'guest-cold','fsync','seq',seq),
            ('random-read',4096,1,'hot','close','hot',hot),
            ('random-read',65536,8,'guest-cold','close','hot',hot),
            ('random-write',4096,1,'hot','fdatasync','write',hot),
            ('random-write',65536,8,'hot','fsync','write',hot)]
    return [dict(operation=op,block_bytes=block,concurrency=c,cache=cache,barrier=barrier,
                 dataset=dataset,file_bytes=size,io_bytes=size) for op,block,c,cache,barrier,dataset,size in shapes]


def run(out,result,guest,args):
    assert args.moosefs_mount is not None
    mfs=str(args.moosefs_mount)
    remote=mfs.replace('/mnt/afsdata/','/mnt/afsdata/',1)
    performance=dict(run_id=result['run_id'],scope='finite actual OCI local three-way and remote two-way diagnostics',
        profile='bounded-five-lane-v1',metadata_files=1000,formal_10000_matrix='NOT_RUN: original failed remote warmup retained',rounds=6,warmup_rounds=1,measured_rounds=5,samples=[],metadata_samples=[],setup={},cleanup=[],
        cache_limit='guest-only cold, uncontrolled Hyper-V host cache; FUSE residency absent and userspace MooseFS cache not qualified empty',
        durability='stock MooseFS B001 unqualified; timing matching syscall replies is not proof of equal physical durability',
        phases_completed=[])
    def save(): (out/'closeout-performance.json').write_text(json.dumps(performance,indent=2)+'\n')
    active=[]
    def command(role,action,**fields):
        return guest(role,'closeout-performance',command=dict(action=action,**fields))
    try:
        for role,path in (('a',mfs),('b',remote)):
            performance['setup'][role]=command(role,'setup',source=result['activated']['source'],moosefs_mount=path)
            active.append(role);save()
        lanes=[('a','ext4'),('a','native'),('a','mfs-local'),('b','dms-remote'),('b','mfs-remote')]
        for form in ('absolute','relative'):
            for concurrency in (1,8):
                for round_number in range(6):
                    for role,lane in (lanes if round_number%2==0 else list(reversed(lanes))):
                        measurement=command(role,'metadata',lane=lane,form=form,concurrency=concurrency,round=round_number)
                        performance['metadata_samples'].append(dict(role=role,lane=lane,form=form,concurrency=concurrency,
                            round=round_number,warmup=round_number==0,result=measurement))
                        save()
                print(f"{result['run_id']}: metadata {form} c{concurrency} completed",flush=True)
        performance['phases_completed'].append('metadata');save()
        performance['case_matrix']=cases()
        large,hot=64*1024**2,16*1024**2
        for lane in ('ext4','mfs-local'):
            for dataset,size in (('seq',large),('hot',hot),('write',hot)):
                initialized=command('a','dataset',lane=lane,dataset=dataset,bytes=size)
                performance.setdefault('dataset_setup',[]).append(dict(lane=lane,dataset=dataset,result=initialized))
                save()
        patterns={(group,dataset):90 for group in ('backing','moosefs') for dataset in ('seq','hot','write')}
        for case_number,case in enumerate(performance['case_matrix']):
            for round_number in range(6):
                for role,lane in (lanes if round_number%2==0 else list(reversed(lanes))):
                    group='moosefs' if lane.startswith('mfs') else 'backing'
                    byte=patterns[group,case['dataset']]
                    if case['operation']=='seq-write': byte=32+case_number*6+round_number
                    if case['operation']=='random-write': byte=161+round_number
                    command(role,'prepare',lane=lane,case=case,byte=byte)
                    cache=[]
                    if case['cache']=='guest-cold':
                        for cache_role in ('a','b'):
                            cache.append(guest(cache_role,'closeout',command=dict(action='cache-reset')))
                    started=time.monotonic_ns()
                    measurement=command(role,'measure',lane=lane,case=case,byte=byte)
                    if case['operation'].endswith('write'): patterns[group,case['dataset']]=byte
                    performance['samples'].append(dict(case=case_number,role=role,lane=lane,round=round_number,
                        warmup=round_number==0,cache_preparation=cache,controller_started_ns=started,
                        controller_finished_ns=time.monotonic_ns(),**measurement))
                    save()
            print(f"{result['run_id']}: IO case {case_number+1}/8 {case['operation']} {case['block_bytes']} c{case['concurrency']} {case['cache']} {case['barrier']} completed",flush=True)
        performance['phases_completed'].append('bulk');save()
    finally:
        for role in reversed(active):
            try: performance['cleanup'].append(dict(role=role,result=command(role,'finish')))
            except Exception as error: performance['cleanup'].append(dict(role=role,error=repr(error)))
            save()
    assert performance['phases_completed']==['metadata','bulk']
    assert all(row['result']['completed'] for row in performance['cleanup'])
    performance['sampling_completed']=True;save()
    return performance
