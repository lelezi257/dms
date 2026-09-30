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
assert not pathlib.Path('/proc/705537/exe').exists()
meta=identity(705507,run+'/bin/afs-meta','8f879feb0c8d58d59a41fa2aa9df06d071d7ac3078b19fcfe60196181d3bf351')
assert hashlib.sha256(pathlib.Path(run+'/bin/afs-node').read_bytes()).hexdigest()=='eb1540ba7b95b9b19aade368d2e93227aba5427581f832a888c10af548a9c865'
for kind in ('dfs','ownerfs'):
    target=run+'/mount-'+kind
    r=subprocess.run(['findmnt','-J','-M',target,'-o','SOURCE,FSTYPE,TARGET'],text=True,capture_output=True)
    if r.returncode==0:
        m=json.loads(r.stdout)['filesystems'][0];assert m['target']==target and m['fstype'].startswith('fuse'),m
        subprocess.run(['fusermount3','-u',target],check=True,timeout=3)
with pathlib.Path(run+'/logs/node-restart-v44.log').open('wb') as output:
    p=subprocess.Popen([run+'/bin/afs-node','--config',run+'/node.toml'],stdin=subprocess.DEVNULL,stdout=output,stderr=subprocess.STDOUT,start_new_session=True)
pathlib.Path(run+'/run/node.pid').write_text(str(p.pid))
start=time.monotonic();deadline=start+50
while time.monotonic()<deadline:
    assert p.poll() is None, 'restarted Node exited'
    try:
        h=json.load(urllib.request.urlopen('http://127.0.0.1:17883/health',timeout=.5))
        if h.get('status')=='ready':
            ms=mounts(run);break
    except Exception:pass
    time.sleep(.1)
else:raise RuntimeError('Node not ready within restart deadline')
print(json.dumps({'node':identity(p.pid,run+'/bin/afs-node','eb1540ba7b95b9b19aade368d2e93227aba5427581f832a888c10af548a9c865'),'meta':identity(705507,run+'/bin/afs-meta','8f879feb0c8d58d59a41fa2aa9df06d071d7ac3078b19fcfe60196181d3bf351'),'mounts':ms,'health':h,'root':root_status(run),'restart_ready_ms':(time.monotonic()-start)*1000,'at_unix':time.time(),'config_sha256':hashlib.sha256(pathlib.Path(run+'/node.toml').read_bytes()).hexdigest()}))
