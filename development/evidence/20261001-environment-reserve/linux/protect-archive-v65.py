#!/usr/bin/env python3
"""Observe protected fault data/baseline binaries and live process identities."""
import hashlib
import json
import os
from pathlib import Path
import stat
import sys

base=Path('/mnt/lima-afsadata')
protected=[base/'afs-delivery'/name/suffix for name in ('corrupt-v62-a','rxe-repair-v62-a')
           for suffix in ('state/node/dfs','prefix/bin')]
protected += [base/name/'bin' for name in ('3fs-baseline','3fs-baseline-arm64-patched')]
files=[]
for root in protected:
    if not root.is_dir():
        raise RuntimeError(f'missing protected tree: {root}')
    for path in sorted(root.rglob('*')):
        s=path.lstat()
        row={'path':str(path),'mode':s.st_mode,'device':s.st_dev,'inode':s.st_ino,
             'uid':s.st_uid,'gid':s.st_gid,'mtime_ns':s.st_mtime_ns,'size':s.st_size}
        if stat.S_ISREG(s.st_mode):
            h=hashlib.sha256()
            with path.open('rb') as stream:
                for block in iter(lambda:stream.read(1024*1024),b''):
                    h.update(block)
            row['sha256']=h.hexdigest()
        elif stat.S_ISLNK(s.st_mode):
            row['target']=os.readlink(path)
        elif not stat.S_ISDIR(s.st_mode):
            raise RuntimeError(f'special protected entry: {path}')
        files.append(row)
processes=[]
for proc in Path('/proc').iterdir():
    if not proc.name.isdigit():continue
    try:
        exe=os.readlink(proc/'exe')
        if not exe.startswith(str(base)+'/'):continue
        s=(proc/'exe').stat()
        fields=(proc/'stat').read_text().rsplit(')',1)[1].split()
        processes.append({'pid':int(proc.name),'exe':exe,'start_ticks':fields[19],
                          'device':s.st_dev,'inode':s.st_ino,'size':s.st_size})
    except (FileNotFoundError,ProcessLookupError):continue
result={'files':files,'processes':sorted(processes,key=lambda x:x['pid']),
        'mountinfo':Path('/proc/self/mountinfo').read_text()}
Path(sys.argv[1]).write_text(json.dumps(result,indent=2)+'\n')
print(json.dumps({'files':len(files),'processes':len(processes),'output':sys.argv[1]}))
