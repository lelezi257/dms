#!/usr/bin/env python3
"""Linux-only focused Node stop proof. Fresh ext4 runtime; never touches v45."""
import os,sys,time,json,hashlib,signal,subprocess,pathlib,urllib.request,platform
assert platform.system()=='Linux'
run=pathlib.Path(sys.argv[1]);node_sha=sys.argv[2];meta_sha=sys.argv[3]
assert not run.exists();run.mkdir()
for rel in ('logs','run','state','bin','mount-dfs','mount-ownerfs'): (run/rel).mkdir()
source=pathlib.Path('/mnt/lima-afsadata/afs-delivery/p2-memory-lane-v45')
for role,sha in (('node',node_sha),('meta',meta_sha)):
    binary=pathlib.Path('/var/tmp/afs-v46-'+role)
    assert hashlib.sha256(binary.read_bytes()).hexdigest()==sha
    subprocess.run(['cp',str(binary),str(run/'bin'/('afs-'+role))],check=True)
    config=(source/(role+'.toml')).read_text().replace(str(source),str(run))
    for old,new in (('17880','17980'),('17881','17981'),('17882','17982'),('17883','17983')): config=config.replace(old,new)
    (run/(role+'.toml')).write_text(config)
inventory=json.loads(subprocess.check_output(['findmnt','-J','-T',str(run),'-o','FSTYPE,SOURCE,TARGET'],text=True))['filesystems'][0]
assert inventory['fstype']=='ext4',inventory
def spawn(role,log):
    with (run/'logs'/log).open('wb') as out:
        return subprocess.Popen([str(run/'bin'/('afs-'+role)),'--config',str(run/(role+'.toml'))],stdin=subprocess.DEVNULL,stdout=out,stderr=subprocess.STDOUT,start_new_session=True)
def identity(p,role,sha):
    proc=pathlib.Path('/proc')/str(p.pid)
    assert p.poll() is None
    assert os.readlink(proc/'exe')==str(run/'bin'/('afs-'+role))
    assert hashlib.sha256((proc/'exe').read_bytes()).hexdigest()==sha
    return {'pid':p.pid,'sha256':sha,'start_ticks':(proc/'stat').read_text().split(') ')[1].split()[19]}
def ready(p,port):
    start=time.monotonic()
    while time.monotonic()-start<20:
        assert p.poll() is None,(p.returncode,port)
        try:
            if json.load(urllib.request.urlopen('http://127.0.0.1:'+str(port)+'/health',timeout=.3))['status']=='ready': return
        except Exception: pass
        time.sleep(.05)
    raise AssertionError('not ready '+str(port))
def close(fd):
    try: os.close(fd);return {'errno':None}
    except OSError as e:return {'errno':e.errno}
def stop(p):
    start=time.monotonic();p.send_signal(signal.SIGTERM)
    return {'status':p.wait(timeout=19),'elapsed_ms':(time.monotonic()-start)*1000}
def exactmounts():
    for kind in ('dfs','ownerfs'):
        m=json.loads(subprocess.check_output(['findmnt','-J','-M',str(run/('mount-'+kind)),'-o','TARGET,FSTYPE,SOURCE'],text=True))['filesystems'][0]
        assert m['target']==str(run/('mount-'+kind)) and m['source']=='afs-'+kind and m['fstype'].startswith('fuse'),m
meta=None;node=None;meta_paused=False;fds=[];events=[{'environment':{'platform':platform.platform(),'boot_id':pathlib.Path('/proc/sys/kernel/random/boot_id').read_text().strip(),'filesystem':inventory,'config_sha256':{role:hashlib.sha256((run/(role+'.toml')).read_bytes()).hexdigest() for role in ('node','meta')}}}]
try:
    meta=spawn('meta','meta.log');ready(meta,17981)
    node=spawn('node','node-normal.log');ready(node,17983);exactmounts()
    events.append({'meta':identity(meta,'meta',meta_sha),'node':identity(node,'node',node_sha)})
    path=run/'mount-dfs'/'normal-drain.bin'
    fd=os.open(path,os.O_CREAT|os.O_EXCL|os.O_RDWR,0o600);fds.append(fd)
    assert os.write(fd,b'accepted-dirty')==14
    normal=stop(node);normal['old_fd_close']=close(fd);fds.remove(fd)
    events.append({'normal_stop':normal});assert normal['status']==0 and normal['elapsed_ms']<19000,normal
    node=spawn('node','node-normal-restart.log');ready(node,17983);exactmounts()
    with path.open('rb') as f: data=f.read()
    assert data==b'accepted-dirty',data
    events.append({'normal_restart':identity(node,'node',node_sha),'dirty_drain_readback':data.hex()})
    # Establish an acknowledged watermark separately from the unknown write.
    path2=run/'mount-dfs'/'forced-unknown.bin'
    fd=os.open(path2,os.O_CREAT|os.O_EXCL|os.O_RDWR,0o600)
    assert os.write(fd,b'seed')==4;os.fsync(fd);os.close(fd)
    fd=os.open(path2,os.O_RDWR);fds.append(fd);assert os.write(fd,b'next')==4
    before=identity(meta,'meta',meta_sha)
    meta.send_signal(signal.SIGSTOP);meta_paused=True
    deadline=time.monotonic()+1
    while time.monotonic()<deadline:
        state=(pathlib.Path('/proc')/str(meta.pid)/'stat').read_text().split(') ')[1].split()[0]
        if state=='T':break
        time.sleep(.01)
    assert state=='T',state
    events.append({'fault':{'signal':'SIGSTOP','meta':before,'state':state,'node':identity(node,'node',node_sha)}})
    forced=stop(node);forced['old_fd_close']=close(fd);fds.remove(fd)
    events.append({'forced_stop':forced});assert forced['status']==124 and 14000<=forced['elapsed_ms']<19000,forced
    meta.send_signal(signal.SIGCONT);meta_paused=False;ready(meta,17981)
    # It is an unknown outcome: either the acknowledged head or the complete
    # later commit may be observed. Never require the latter or accept mixtures.
    for kind in ('dfs','ownerfs'):
        target=run/('mount-'+kind)
        m=subprocess.run(['findmnt','-M',str(target)],capture_output=True)
        if m.returncode==0: subprocess.run(['fusermount3','-u',str(target)],check=True,timeout=3)
    node=spawn('node','node-forced-restart.log');ready(node,17983);exactmounts()
    with path2.open('rb') as f: data=f.read()
    assert data in (b'seed',b'next'),data
    with path.open('rb') as f: prior=f.read()
    assert prior==b'accepted-dirty',prior
    events.append({'forced_restart':identity(node,'node',node_sha),'unknown_readback':data.hex(),'prior_watermark':prior.hex()})
    final=stop(node);events.append({'final_stop':final});assert final['status']==0,final
    ms=stop(meta);events.append({'meta_stop':ms});assert ms['status']==0,ms
    (run/'report.json').write_text(json.dumps({'status':'PASS','events':events},indent=2)+'\n')
    print(json.dumps({'status':'PASS','events':events},indent=2))
except BaseException:
    (run/'report.json').write_text(json.dumps({'status':'FAIL','events':events},indent=2)+'\n')
    raise
finally:
    if meta_paused: meta.send_signal(signal.SIGCONT)
    for fd in fds: close(fd)
    for p in (node,meta):
        if p and p.poll() is None:
            p.terminate()
            try:p.wait(timeout=19)
            except subprocess.TimeoutExpired:p.kill();p.wait(timeout=3)
