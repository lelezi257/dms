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

run='/mnt/lima-afsadata/afs-delivery/p2-memory-lane-v45';node=identity(706396,run+'/bin/afs-node','d8937569feaee6026fdf14941e0b4f742c935a17798aeefd223f26e578fd49e8');meta=identity(706366,run+'/bin/afs-meta','8bcfbcbd2f84be5029ab848c214aaff258ba8147684a141b17b262d0b6cc8d10')
os.kill(node['pid'],9);deadline=time.monotonic()+2
while pathlib.Path('/proc/706396/stat').exists() and time.monotonic()<deadline:
    text=pathlib.Path('/proc/706396/stat').read_text()
    if text[text.rfind(')')+2:].split()[0]=='Z':break
    time.sleep(.05)
dead=not pathlib.Path('/proc/706396/stat').exists() or pathlib.Path('/proc/706396/stat').read_text().split(') ')[1].split()[0]=='Z'
assert dead
print(json.dumps({'signal':'SIGKILL','node_before':node,'node_dead':dead,'meta_after':identity(706366,run+'/bin/afs-meta','8bcfbcbd2f84be5029ab848c214aaff258ba8147684a141b17b262d0b6cc8d10'),'at_unix':time.time(),'root_after':root_status(run)}))
