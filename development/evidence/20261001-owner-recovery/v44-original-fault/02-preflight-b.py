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

run='/mnt/lima-afsbdata/afs-delivery/p2-memory-peer-v44'
report={'node':identity(66574,run+'/bin/afs-node','eb1540ba7b95b9b19aade368d2e93227aba5427581f832a888c10af548a9c865'),'boot_id':pathlib.Path('/proc/sys/kernel/random/boot_id').read_text().strip(),'mounts':mounts(run),'log_offset':pathlib.Path(run+'/logs/node.log').stat().st_size,'config_sha256':hashlib.sha256(pathlib.Path(run+'/node.toml').read_bytes()).hexdigest()}
state=pathlib.Path('/var/tmp/afs-v44-owner-fault');assert not (state/'ready.json').exists()
args=['python3','/var/tmp/afs-v44-owner-restart.py','writer','--target',run+'/mount-dfs/'+'owner-restart-v44-64m.bin','--state-dir',str(state),'--ready-file',str(state/'ready.json'),'--trigger-file',str(state/'trigger'),'--result-file',str(state/'writer-result.json'),'--size','67108864','--trigger-timeout','120','--close-deadline','30']
with (state/'writer.log').open('wb') as out:
    p=subprocess.Popen(args,stdin=subprocess.DEVNULL,stdout=out,stderr=subprocess.STDOUT,start_new_session=True)
report['worker_pid']=p.pid;report['argv']=args
print(json.dumps(report))
