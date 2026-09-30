import os, json, hashlib, subprocess, pathlib, urllib.request, time
def identity(pid, exe, expected):
    path = pathlib.Path('/proc')/str(pid)
    actual = os.readlink(path/'exe')
    digest = hashlib.sha256((path/'exe').read_bytes()).hexdigest()
    stat = (path/'stat').read_text(); fields=stat[stat.rfind(')')+2:].split()
    assert actual == exe and digest == expected and fields[0] != 'Z', (actual,digest)
    return {'pid':pid,'exe':actual,'sha256':digest,'start_ticks':fields[19],'state':fields[0]}
def mounts(run):
    rows=[]
    for kind in ('dfs','ownerfs'):
        target=run+'/mount-'+kind
        value=json.loads(subprocess.check_output(['findmnt','-J','-M',target,'-o','SOURCE,FSTYPE,TARGET,OPTIONS'],text=True))['filesystems'][0]
        assert value['target']==target and value['fstype'].startswith('fuse'), value
        rows.append(value)
    return rows
def root_status(run):
    root=pathlib.Path(run+'/logs/root-id.txt').read_text().strip()
    return json.load(urllib.request.urlopen('http://127.0.0.1:17881/v1/roots/'+root,timeout=2))

run='/mnt/lima-afsadata/afs-delivery/p2-memory-lane-v44'
report={'node':identity(705537,run+'/bin/afs-node','eb1540ba7b95b9b19aade368d2e93227aba5427581f832a888c10af548a9c865'),'meta':identity(705507,run+'/bin/afs-meta','8f879feb0c8d58d59a41fa2aa9df06d071d7ac3078b19fcfe60196181d3bf351'),'boot_id':pathlib.Path('/proc/sys/kernel/random/boot_id').read_text().strip(),'mounts':mounts(run),'root':root_status(run),'config_sha256':hashlib.sha256(pathlib.Path(run+'/node.toml').read_bytes()).hexdigest()}
target=run+'/mount-dfs/'+'owner-restart-v44-64m.bin'
fd=os.open(target,os.O_CREAT|os.O_EXCL|os.O_RDWR,0o600)
os.write(fd,b'created-by-owner-a');os.fsync(fd);os.close(fd)
report['target']=target
print(json.dumps(report))
