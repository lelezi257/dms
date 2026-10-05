#!/usr/bin/env python3
"""Finite paired OCI lanes for investigation, never a release-gate bypass."""
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import time


def runtime(base, guest, argv):
    current=guest.node(base)
    executable=shutil.which('runc') or '/mnt/afsdata/ownerfs-native-tools/runc-1.3.4'
    assert guest.digest(Path(executable))=='bdce4d45b2dd217491db8a98c8484b161e225ce49c15ebc1ba42077fb7c07d50'
    command=['nsenter','--target',str(current['pid']),'--mount','--',executable,'--root',str(base/'closeout-oci-state'),*argv]
    if argv[0]=='run':
        with open(base/(argv[-1]+'.stdout'),'w') as output,open(base/(argv[-1]+'.stderr'),'w') as errors:
            result=subprocess.run(command,stdin=subprocess.DEVNULL,stdout=output,stderr=errors,text=True,timeout=30)
        assert result.returncode==0,(base/(argv[-1]+'.stderr')).read_text()
        return ''
    result=subprocess.run(command,capture_output=True,text=True,timeout=1800)
    with open(base/'closeout-runtime.jsonl','a') as log:
        log.write(json.dumps(dict(argv=command,exit=result.returncode,stdout=result.stdout,stderr=result.stderr))+'\n')
    assert result.returncode==0,result.stderr
    return result.stdout


def records(base):
    return json.loads((base/'closeout-performance-state.json').read_text())


def check(base,guest,lane):
    record=records(base)['lanes'][lane]
    guest.verify(record['process'])
    state=json.loads(runtime(base,guest,['state',record['id']]))
    assert state['status']=='running' and state['pid']==record['process']['pid']
    return record


def setup(base,command,guest):
    cfg=guest.role_config(base)
    role=cfg['role']
    assert role in ('a','b')
    for name in ('ownerfs_native_closeout_performance.py','io-closeout','benchmark','container-probe'):
        assert guest.digest(base/name)==cfg['inputs'][name]
    mfsroot=Path(command['moosefs_mount'])
    assert str(mfsroot).startswith('/mnt/afsdata/ownerfs-native-moosefs/') and mfsroot.name=='mount'
    observed=json.loads(guest.in_namespace(base,['findmnt','-T',str(mfsroot),'--json']).stdout)['filesystems'][0]
    assert observed['source']=='mfs#10.77.30.11:19421' and observed['target']==str(mfsroot)
    mfs=mfsroot/(base.name+'-closeout')
    if role=='a':
        assert not mfs.exists()
        mfs.mkdir()
        install=mfsroot.parent/'moosefs-install/bin'
        subprocess.run([str(install/'mfssetsclass'),'-r','native-bind-one',str(mfs)],check=True,capture_output=True,text=True)
        goal=subprocess.check_output([str(install/'mfsgetsclass'),str(mfs)],text=True)
        assert goal.strip().endswith(': native-bind-one')
        definition=subprocess.check_output([str(install/'mfsscadmin'),'list','-M',str(mfsroot),'-l','native-bind-one'],text=True)
        assert definition.strip()
        (base/'moosefs-class-definition.txt').write_text(definition)
        from ownerfs_native_closeout_guest import source
        backing=source(base,command['source'])
        assert os.statvfs(backing).f_bavail*os.statvfs(backing).f_frsize>22*1024**3
        lanes={'ext4':backing,'native':base/'mount/agent1','mfs-local':mfs}
    else:
        assert mfs.is_dir()
        lanes={'dms-remote':base/'mount/agent1','mfs-remote':mfs}
        goal=None
    rootfs=base/'closeout-rootfs'
    rootfs.mkdir()
    for path in ('proc','ownerfs/agent1'): (rootfs/path).mkdir(parents=True,exist_ok=True)
    manifest={}
    for name,destination in (('io-closeout','io'),('benchmark','benchmark'),('container-probe','container-probe')):
        targets=[(base/name,'/'+destination)]
        deps=subprocess.check_output(['ldd',str(base/name)],text=True)
        targets += [(Path(path),path) for path in re.findall(r'(/[^\s]+)',deps)]
        for file,target in targets:
            final=rootfs/target.lstrip('/')
            final.parent.mkdir(parents=True,exist_ok=True)
            shutil.copyfile(file,final);final.chmod(0o755)
            manifest[target]=dict(source=str(file),sha256=guest.digest(final))
    (base/'closeout-oci-state').mkdir()
    current=guest.node(base)
    result=dict(role=role,node=current,source=command.get('source'),rootfs_files=manifest,
                lanes={},goal=goal,moosefs_mount=observed,completed=False)
    guest.save(base/'closeout-performance-state.json',result)
    for lane,source in lanes.items():
        bundle=base/('closeout-bundle-'+lane);bundle.mkdir()
        identifier=base.name+'-'+lane
        spec=dict(ociVersion='1.0.2',root=dict(path=str(rootfs),readonly=True),
            process=dict(terminal=False,user=dict(uid=0,gid=0),args=['/container-probe','--idle'],cwd='/',
                env=['PATH=/bin'],noNewPrivileges=True,
                capabilities={key:[] for key in ('bounding','effective','inheritable','permitted','ambient')}),
            mounts=[dict(destination='/proc',type='proc',source='proc',options=['nosuid','noexec','nodev']),
                    dict(destination='/ownerfs/agent1',type='bind',source=str(source),options=['bind','rw','rprivate','nosuid','nodev'])],
            linux=dict(rootfsPropagation='private',cgroupsPath='/dms-native-closeout/'+base.name+'/'+lane,
                namespaces=[dict(type=kind) for kind in ('mount','pid','network','ipc','uts','cgroup')]))
        guest.save(bundle/'config.json',spec)
        runtime(base,guest,['run','--detach','--bundle',str(bundle),identifier])
        state=json.loads(runtime(base,guest,['state',identifier]))
        process=guest.identity(state['pid'])
        mountinfo=Path(f"/proc/{state['pid']}/mountinfo").read_text()
        mount=[line for line in mountinfo.splitlines() if line.split()[4]=='/ownerfs/agent1']
        assert len(mount)==1
        assert (' - ext4 ' in mount[0]) if lane in ('ext4','native') else (' - fuse' in mount[0])
        final=os.stat(f"/proc/{state['pid']}/root/ownerfs/agent1")
        root_object=dict(device=final.st_dev,inode=final.st_ino)
        if lane in ('ext4','native'): assert root_object==command['source']
        status=Path(f"/proc/{state['pid']}/status").read_text()
        assert all('Cap'+key+':\t0000000000000000' in status for key in ('Inh','Prm','Eff','Bnd','Amb'))
        assert 'NoNewPrivs:\t1' in status
        result['lanes'][lane]=dict(id=identifier,process=process,source_path=str(source),root_object=root_object,
            mountinfo=mountinfo,status=status,spec=spec)
        guest.save(base/'closeout-performance-state.json',result)
    return result


def io(base,guest,lane,case,byte,creation='existing',preparation=False):
    record=check(base,guest,lane)
    observed=lane in ('ext4','native')
    cache=('unchecked' if preparation else case['cache']) if observed else 'unobserved'
    path='/ownerfs/agent1/io-'+case['dataset']+'-data'
    argv=['exec',record['id'],'/io',path,case['operation'],str(case['file_bytes']),str(case['block_bytes']),
        str(case['concurrency']),case['barrier'],str(case['io_bytes']),str(byte),creation,cache]
    return json.loads(runtime(base,guest,argv))


def run(base,command,guest):
    action=command['action']
    if action=='setup': return setup(base,command,guest)
    if action=='dataset':
        lane=command['lane']
        case=dict(dataset=command['dataset'],operation='seq-write',file_bytes=command['bytes'],
            io_bytes=command['bytes'],block_bytes=1024**2,concurrency=1,barrier='fsync',cache='unchecked')
        return io(base,guest,lane,case,90,'create',True)
    if action=='prepare':
        case=command['case']; lane=command['lane']; byte=command['byte']
        # All lanes reset the same source to the same state outside the timer.
        prep=dict(case,operation='seq-write',block_bytes=1024**2,concurrency=1,barrier='fsync',io_bytes=case['file_bytes'])
        out=io(base,guest,lane,prep,0,preparation=True) if case['operation']=='random-write' else dict(reset=False)
        if case['cache'] in ('hot','repeat'):
            warm=dict(prep,operation='seq-read',barrier='close')
            out=io(base,guest,lane,warm,0 if case['operation']=='random-write' else byte,preparation=True)
        return out
    if action=='measure':
        before=dict(diskstats=Path('/proc/diskstats').read_text(),meminfo=Path('/proc/meminfo').read_text())
        result=io(base,guest,command['lane'],command['case'],command['byte'])
        return dict(result=result,resources_before=before,resources_after=dict(
            diskstats=Path('/proc/diskstats').read_text(),meminfo=Path('/proc/meminfo').read_text()))
    if action=='metadata':
        record=check(base,guest,command['lane'])
        suffix={'ext4':'ext4','native':'natv','mfs-local':'mfsl','dms-remote':'dmsr','mfs-remote':'mfsr'}[command['lane']]
        path='/ownerfs/agent1/perf-'+command['form']+'-'+str(command['concurrency'])+'-'+str(command['round']).zfill(2)+'-'+suffix
        return json.loads(runtime(base,guest,['exec',record['id'],'/benchmark',path,command['form'],str(command['concurrency'])]))
    if action=='finish':
        result=records(base);cleanup=[]
        for lane in reversed(result['lanes']):
            record=check(base,guest,lane)
            runtime(base,guest,['kill',record['id'],'TERM'])
            deadline=time.monotonic()+10
            while time.monotonic()<deadline:
                state=json.loads(runtime(base,guest,['state',record['id']]))
                if state['status']=='stopped': break
                time.sleep(.05)
            assert state['status']=='stopped'
            runtime(base,guest,['delete',record['id']])
            cleanup.append(dict(lane=lane,stopped=True,deleted=True))
        result['cleanup']=cleanup;result['completed']=True
        guest.save(base/'closeout-performance-state.json',result)
        return result
    raise ValueError(action)
