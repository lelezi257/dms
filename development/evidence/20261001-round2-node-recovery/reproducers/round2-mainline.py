#!/usr/bin/env python3
"""Bounded Linux workers for current whole-system fault development, not acceptance."""
import argparse
import errno
import hashlib
import json
import os
import pathlib
import platform
import signal
import subprocess
import time
import urllib.request

NODE = 'd28d17faa8bb71eb8c0a28241e8556396e440b204cf0242d2af8133700f6a494'
META = '64d85fc3933637ccd7fdf4eafded5e5c8cb2354e73b4c8d1a27b2b8d43f4c7e7'
VOLUMES = {'ctl':'/mnt/lima-afsctlstate','a':'/mnt/lima-afsadata','b':'/mnt/lima-afsbdata','c':'/mnt/lima-afscdata'}
CHUNKS = ('blake3-4e94e6f582581a0f3855f3ce504b153e951e65036fe9e2f010b7e25473c54f98-4194304','blake3-63c31766464b0c4931ff8b7406a2c1d8140d08b94328ccd7cf3b431d94cc690f-17')
ROOT_ID = 'root-776f726b73706163652d726f756e64312d763737'
SIZE = 4194321
PAYLOAD_SHA = '7fa1366968edf9a8490fffac3eab7e0b1619480a152d8e75c7471f753a8c1231'

def sha(p):
    h=hashlib.sha256()
    with pathlib.Path(p).open('rb') as f:
        for part in iter(lambda:f.read(1024*1024),b''):h.update(part)
    return h.hexdigest()

def identity(run,which):
    role='meta' if which=='ctl' else 'node'
    pid=int((run/'run'/f'{role}.pid').read_text().strip())
    proc=pathlib.Path('/proc')/str(pid)
    raw=(proc/'stat').read_text()
    exe=(proc/'exe').resolve()
    expected=run/'prefix/bin'/f'afs-{role}'
    assert exe==expected.resolve(),(exe,expected)
    digest=sha(proc/'exe'); assert digest==(META if which=='ctl' else NODE),digest
    return {'pid':pid,'role':role,'exe':str(exe),'sha256':digest,'start_ticks':raw[raw.rindex(')')+2:].split()[19],'boot_id':pathlib.Path('/proc/sys/kernel/random/boot_id').read_text().strip()}

def web(path):
    opener=urllib.request.build_opener(urllib.request.ProxyHandler({}))
    with opener.open('http://192.168.109.11:19981'+path,timeout=10) as response:return json.load(response)

def mounts(run):
    result=[]
    for kind in ('ownerfs','dfs'):
        raw=subprocess.check_output(['findmnt','-J','-M',str(run/'mount'/kind),'-o','TARGET,SOURCE,FSTYPE'],text=True,timeout=5)
        m=json.loads(raw)['filesystems'][0]
        assert m['source']=='afs-'+kind and m['fstype'].startswith('fuse'),m
        result.append(m)
    return result

def main(args):
    assert platform.system()=='Linux' and platform.machine()=='aarch64'
    run=pathlib.Path(VOLUMES[args.which])/'afs-delivery/round1-mainline-v77-archive-async'
    record={'which':args.which,'action':args.action,'utc':time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime()),'runtime':str(run),'formal_acceptance':'NOT_RUN','environment':'PREPARING'}
    if args.action in ('identity','kill'):
        record['identity']=identity(run,args.which)
        if args.action=='kill':
            assert args.which!='ctl','Memory authority must remain running'
            assert args.expected_pid==record['identity']['pid'] and args.expected_ticks==record['identity']['start_ticks'],'Fault target identity changed'
            pidfd=os.pidfd_open(args.expected_pid)
            try:
                assert identity(run,args.which)==record['identity']
                signal.pidfd_send_signal(pidfd,signal.SIGKILL)
            finally:os.close(pidfd)
            for _ in range(100):
                proc=pathlib.Path('/proc')/str(args.expected_pid)
                if not proc.exists() or (proc/'stat').read_text().split(') ')[1].startswith('Z '):break
                time.sleep(.05)
            else:raise AssertionError('Killed process still alive')
            record['signal']='SIGKILL'; record['dead']=True
    elif args.action=='snapshot':
        record['home']=web('/v1/roots/'+ROOT_ID)
        record['replication']=[web('/v1/dfs/chunks/'+chunk+'/replication') for chunk in CHUNKS]
    elif args.action=='physical':
        record['identity']=identity(run,args.which)
        p=run/'state/node/dfs/chunks'
        record['chunks']=[{'name':c,'bytes':(p/c).stat().st_size,'sha256':sha(p/c)} for c in CHUNKS if (p/c).is_file()]
        if args.which=='a':
            f=run/'state/node/ownerfs'/f'{ROOT_ID}-e1/data.bin'
            record['home_file']={'sha256':sha(f),'bytes':f.stat().st_size}
    elif args.action=='read':
        record['identity']=identity(run,args.which);record['mounts']=mounts(run)
        assert args.kind in ('ownerfs','dfs')
        target=run/'mount'/args.kind
        if args.kind=='ownerfs':target/='workspace-round1-v77'
        target/='data.bin';record['path']=str(target)
        started=time.monotonic(); value=bytearray(); record['operation']='open'
        try:
            fd=os.open(target,os.O_RDONLY)
            try:
                record['operation']='read'
                while len(value)<=SIZE:
                    part=os.read(fd,min(1024*1024,SIZE+1-len(value)))
                    if not part:break
                    value.extend(part)
            finally:
                record['operation']='close';os.close(fd)
            record.update(outcome='success',bytes=len(value),sha256=hashlib.sha256(value).hexdigest())
            assert len(value)==SIZE and record['sha256']==PAYLOAD_SHA,'Incorrect successful read'
        except OSError as e:
            record.update(outcome='error',errno=e.errno,errno_name=errno.errorcode.get(e.errno,'UNKNOWN'),error=str(e),bytes_before_error=len(value))
        record['elapsed_seconds']=time.monotonic()-started
        assert record['outcome']==args.expect,(args.expect,record)
    else:raise ValueError(args.action)
    record['status']='PASS'
    return record

if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('which',choices=VOLUMES);p.add_argument('action',choices=('identity','kill','snapshot','physical','read'));p.add_argument('--kind',choices=('ownerfs','dfs'));p.add_argument('--expect',choices=('success','error'),default='success');p.add_argument('--expected-pid',type=int);p.add_argument('--expected-ticks');args=p.parse_args()
    print(json.dumps(main(args),indent=2),flush=True)
