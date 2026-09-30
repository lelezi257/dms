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

run='/mnt/lima-afsadata/afs-delivery/p2-memory-lane-v44';node=identity(705537,run+'/bin/afs-node','eb1540ba7b95b9b19aade368d2e93227aba5427581f832a888c10af548a9c865');meta=identity(705507,run+'/bin/afs-meta','8f879feb0c8d58d59a41fa2aa9df06d071d7ac3078b19fcfe60196181d3bf351')
os.kill(node['pid'],9);deadline=time.monotonic()+2
while pathlib.Path('/proc/705537/stat').exists() and time.monotonic()<deadline:
    text=pathlib.Path('/proc/705537/stat').read_text()
    if text[text.rfind(')')+2:].split()[0]=='Z':break
    time.sleep(.05)
dead=not pathlib.Path('/proc/705537/stat').exists() or pathlib.Path('/proc/705537/stat').read_text().split(') ')[1].split()[0]=='Z'
assert dead
print(json.dumps({'signal':'SIGKILL','node_before':node,'node_dead':dead,'meta_after':identity(705507,run+'/bin/afs-meta','8f879feb0c8d58d59a41fa2aa9df06d071d7ac3078b19fcfe60196181d3bf351'),'at_unix':time.time(),'root_after':root_status(run)}))
