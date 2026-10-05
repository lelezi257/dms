#!/usr/bin/env python3
"""Finite investigation only. No production admission, recovery or tuning."""
import ctypes
import errno
import json
import mmap
import os
from pathlib import Path
import select
import signal
import struct
import subprocess
import sys
import time


def source(base, expected):
    found = [Path(d) for d, _, _ in os.walk(base/'data')
             if dict(device=os.stat(d).st_dev, inode=os.stat(d).st_ino) == expected]
    assert len(found) == 1
    return found[0]


def run(base, command, guest):
    assert guest.digest(base/'ownerfs_native_closeout_guest.py') == guest.role_config(base)['inputs']['ownerfs_native_closeout_guest.py']
    action = command['action']
    if action == 'trace-start':
        current = guest.node(base)
        # Delay the return of actual Home data syscalls. No simulated FUSE reply.
        log = open(base/'delayed-write.strace', 'w')
        child = subprocess.Popen(['strace','-f','-ttt','-s','128','-p',str(current['pid']),
            '-e','trace=pwrite64','-e','inject=pwrite64:delay_exit=8s:when=1'],
            stdin=subprocess.DEVNULL,stdout=subprocess.DEVNULL,stderr=log,start_new_session=True)
        time.sleep(.3)
        assert child.poll() is None
        record = guest.identity(child.pid)
        guest.save(base/'trace-process.json',record)
        return dict(trace=record,node=current,delay_seconds=8)
    if action == 'trace-stop':
        record = json.loads((base/'trace-process.json').read_text())
        guest.verify(record)
        os.kill(record['pid'],signal.SIGINT)
        deadline = time.monotonic()+5
        while time.monotonic()<deadline:
            status = Path(f"/proc/{record['pid']}/status")
            if not status.exists() or 'State:\tZ' in status.read_text(): break
            time.sleep(.05)
        text = (base/'delayed-write.strace').read_text()
        guest.save(base/'trace-result.json',dict(delayed='DELAYED' in text,trace=text))
        return dict(delayed='DELAYED' in text,trace=text)
    if action == 'physical-check':
        backing = source(base,command['source'])
        path = backing/'data'
        return dict(matches=path.read_bytes()==command['data'].encode(),bytes=path.stat().st_size)
    if action == 'pending':
        directory = base/('actor-'+command['actor'])
        actor = json.loads((directory/'ready.json').read_text())
        guest.verify(actor['process'])
        return dict(reply_exists=(directory/f"reply-{command['id']}.json").exists(),
                    actor=actor,syscall=Path(f"/proc/{actor['process']['pid']}/syscall").read_text())
    if action == 'syscalls':
        role = guest.role_config(base)['role']
        roots = {'remote':str(base/'mount/agent1')} if role=='b' else {'native':str(base/'mount/agent1')}
        if role=='a':
            actor=json.loads((base/'actor-oldfuse/ready.json').read_text())
            guest.verify(actor['process'])
            matches=[]
            for fd in Path(f"/proc/{actor['process']['pid']}/fd").iterdir():
                try:
                    stat=os.stat(fd)
                    if dict(device=stat.st_dev,inode=stat.st_ino)==actor['root']: matches.append(fd)
                except FileNotFoundError: pass
            assert len(matches)==1
            roots['oldfuse']=str(matches[0])
        cfg=dict(roots=roots,base=str(base),mutation_root=str(base/'mount/agent1'),role=role)
        if role=='a': cfg['physical']=str(source(base,command['source']))
        result=json.loads(guest.in_namespace(base,[sys.executable,str(base/'ownerfs_native_closeout_guest.py'),
                           '--syscalls',json.dumps(cfg)]).stdout)
        guest.save(base/'syscall-results.json',result)
        return result
    if action == 'watch-start':
        cfg=dict(base=str(base),root=str(base/'mount/agent1'))
        with open(base/'watch.stdout','w') as output,open(base/'watch.stderr','w') as errors:
            current=guest.node(base)
            child=subprocess.Popen(['nsenter','--target',str(current['pid']),'--mount','--',
                sys.executable,str(base/'ownerfs_native_closeout_guest.py'),'--watch',json.dumps(cfg)],
                stdin=subprocess.DEVNULL,stdout=output,stderr=errors,start_new_session=True)
        ready=guest.wait_file(base/'watch-ready.json',10)
        guest.verify(ready['process'])
        return ready
    if action == 'watch-finish':
        (base/'watch-finish').write_text('finish')
        result=guest.wait_file(base/'watch-result.json',10)
        return result
    if action == 'watch-mutate':
        return guest.in_namespace(base,[sys.executable,'-c',
            "from pathlib import Path; import sys; p=Path(sys.argv[1]); (p/'native-watch-marker').write_text('native')",
            str(base/'mount/agent1')]).stdout
    if action == 'cache-reset':
        os.sync()
        Path('/proc/sys/vm/drop_caches').write_text('3\n')
        return dict(guest_drop_caches=3,scope='guest only; userspace client caches/Hyper-V host cache not proven empty')
    raise ValueError(action)


def events(fd):
    rows=[]
    while select.select([fd],[],[],0)[0]:
        data=os.read(fd,65536)
        offset=0
        while offset<len(data):
            wd,mask,cookie,length=struct.unpack_from('iIII',data,offset)
            name=data[offset+16:offset+16+length].split(b'\0',1)[0].decode()
            rows.append(dict(wd=wd,mask=mask,cookie=cookie,name=name))
            offset+=16+length
    return rows


def watch(root):
    libc=ctypes.CDLL(None,use_errno=True)
    fd=libc.inotify_init1(os.O_NONBLOCK|os.O_CLOEXEC)
    assert fd>=0
    wd=libc.inotify_add_watch(fd,os.fsencode(root),0x00000100|0x00000002|0x00000008)
    if wd<0: raise OSError(ctypes.get_errno(),'inotify_add_watch')
    return fd,wd


def syscall_matrix(cfg):
    roots={name:Path(root) for name,root in cfg['roots'].items()}
    result=dict(role=cfg['role'],roots=cfg['roots'],operations={},watch={})
    for lane,root in roots.items():
        file=root/('mapping-'+lane)
        fd=os.open(file,os.O_CREAT|os.O_EXCL|os.O_RDWR,0o600)
        try:
            assert os.write(fd,b'A'*4096)==4096
            os.fsync(fd)
            rows={}
            for flag,name in ((mmap.MAP_SHARED,'shared'),(mmap.MAP_PRIVATE,'private')):
                try:
                    mapping=mmap.mmap(fd,4096,flags=flag,prot=mmap.PROT_READ|mmap.PROT_WRITE)
                    assert mapping[:4]==b'AAAA'
                    mapping[:4]=b'MMAP'
                    mapping.flush()
                    os.fsync(fd)
                    mapping.close()
                    actual=os.pread(fd,4,0)
                    assert actual==(b'MMAP' if flag==mmap.MAP_SHARED else b'AAAA')
                    rows[name]=dict(ok=True,file_bytes=actual.decode(),msync_fsync_ok=True)
                    assert os.pwrite(fd,b'AAAA',0)==4
                except OSError as error:
                    rows[name]=dict(ok=False,errno=error.errno,error=str(error))
            os.fchmod(fd,0o640)
            os.setxattr(fd,b'user.native_probe',b'xattr-proof')
            assert os.getxattr(fd,b'user.native_probe')==b'xattr-proof'
            assert os.fstat(fd).st_mode&0o777==0o640
            rows['mode_xattr']=dict(ok=True,mode=0o640)
            link=root/('symlink-'+lane)
            os.symlink(file.name,link)
            assert link.read_bytes()==file.read_bytes()
            os.unlink(link)
            rows['symlink']=dict(ok=True)
            result['operations'][lane]=rows
        finally:
            os.close(fd)
            file.unlink()
    if cfg['role']=='a':
        watchers={lane:watch(root) for lane,root in roots.items()}
        try:
            # A positive FUSE notification control precedes the native mutation.
            (roots['oldfuse']/'fuse-watch-control').write_text('fuse-control')
            time.sleep(.2)
            control={lane:events(fd) for lane,(fd,_) in watchers.items()}
            assert any(row['name']=='fuse-watch-control' for row in control['oldfuse'])
            (roots['native']/'native-watch-control').write_text('native-control')
            time.sleep(.2)
            mutation={lane:events(fd) for lane,(fd,_) in watchers.items()}
            assert any(row['name']=='native-watch-control' for row in mutation['native'])
            result['watch']=dict(positive_control=control,native_mutation=mutation,
                native_event_on_oldfuse=any(row['name']=='native-watch-control' for row in mutation['oldfuse']))
        finally:
            for fd,_ in watchers.values(): os.close(fd)
    return result


def watch_process(cfg):
    # Import the source-frozen helper only in the deployed run directory.
    import guest
    base=Path(cfg['base'])
    fd,wd=watch(cfg['root'])
    root=Path(cfg['root'])
    (root/'remote-watch-positive').write_text('remote-positive')
    time.sleep(.2)
    control=events(fd)
    assert any(row['name']=='remote-watch-positive' for row in control)
    guest.save(base/'watch-ready.json',dict(process=guest.identity(os.getpid()),control=control,wd=wd))
    until=time.monotonic()+30
    while not (base/'watch-finish').exists():
        assert time.monotonic()<until
        time.sleep(.05)
    time.sleep(.2)
    observed=events(fd)
    os.close(fd)
    guest.save(base/'watch-result.json',dict(control=control,after_native=observed,
        native_event_delivered=any(row['name']=='native-watch-marker' for row in observed)))


if __name__=='__main__':
    if sys.argv[1]=='--syscalls': print(json.dumps(syscall_matrix(json.loads(sys.argv[2]))))
    elif sys.argv[1]=='--watch': watch_process(json.loads(sys.argv[2]))
    else: raise ValueError(sys.argv[1])
