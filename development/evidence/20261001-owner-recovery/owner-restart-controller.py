#!/usr/bin/env python3
"""Host orchestration only; product I/O, faults and assertions run on Linux."""
import json
import subprocess
import time
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
EVIDENCE = ROOT / "evidence/afs-delivery/p2-owner-recovery-v45/runtime/owner-crash-64m"
A = "/mnt/lima-afsadata/afs-delivery/p2-memory-lane-v45"
B = "/mnt/lima-afsbdata/afs-delivery/p2-memory-peer-v45"
STATE = "/var/tmp/afs-v45-owner-fault"
NODE_SHA = "d8937569feaee6026fdf14941e0b4f742c935a17798aeefd223f26e578fd49e8"
META_SHA = "8bcfbcbd2f84be5029ab848c214aaff258ba8147684a141b17b262d0b6cc8d10"
FILE = "owner-restart-v45-64m.bin"
RESUME = '--resume-after-kill' in sys.argv
EVIDENCE.mkdir(parents=True, exist_ok=RESUME)

def guest(vm, name, code, timeout=35):
    if RESUME: name += '-resumed'
    started = time.monotonic()
    cmd = ["limactl", "shell", vm, "--", "sudo", "python3", "-"]
    (EVIDENCE / (name + ".py")).write_text(code)
    result = subprocess.run(cmd, input=code, text=True, capture_output=True, timeout=timeout)
    (EVIDENCE / (name + ".stdout")).write_text(result.stdout)
    (EVIDENCE / (name + ".stderr")).write_text(result.stderr)
    (EVIDENCE / (name + ".command.json")).write_text(json.dumps({"argv": cmd, "exit": result.returncode, "elapsed": time.monotonic()-started}, indent=2))
    if result.returncode:
        raise RuntimeError(f"{name} exit {result.returncode}: {result.stderr[-1500:]}")
    return json.loads(result.stdout)

IDENTITY = '''import os, json, hashlib, subprocess, pathlib, urllib.request, time
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
'''

def main():
    if RESUME:
        def saved(name):
            return json.loads((EVIDENCE / (name + '.stdout')).read_text())
        before_a=saved('01-preflight-a')
        before_b=saved('02-preflight-b')
        ready=saved('03-durable-watermark')
        fault=saved('04-kill-owner')
    else:
        before_a=guest('afs-accept-a','01-preflight-a',IDENTITY+f'''
run={A!r}
report={{'node':identity(706396,run+'/bin/afs-node',{NODE_SHA!r}),'meta':identity(706366,run+'/bin/afs-meta',{META_SHA!r}),'boot_id':pathlib.Path('/proc/sys/kernel/random/boot_id').read_text().strip(),'mounts':mounts(run),'root':root_status(run),'config_sha256':hashlib.sha256(pathlib.Path(run+'/node.toml').read_bytes()).hexdigest()}}
target=run+'/mount-dfs/'+{FILE!r}
fd=os.open(target,os.O_CREAT|os.O_EXCL|os.O_RDWR,0o600)
os.write(fd,b'created-by-owner-a');os.fsync(fd);os.close(fd)
report['target']=target
print(json.dumps(report))
''')
        before_b=guest('afs-accept-b','02-preflight-b',IDENTITY+f'''
run={B!r}
report={{'node':identity(67058,run+'/bin/afs-node',{NODE_SHA!r}),'boot_id':pathlib.Path('/proc/sys/kernel/random/boot_id').read_text().strip(),'mounts':mounts(run),'log_offset':pathlib.Path(run+'/logs/node.log').stat().st_size,'config_sha256':hashlib.sha256(pathlib.Path(run+'/node.toml').read_bytes()).hexdigest()}}
state=pathlib.Path({STATE!r});state.mkdir(exist_ok=False);assert not (state/'ready.json').exists()
args=['python3','/var/tmp/afs-v45-owner-restart.py','writer','--target',run+'/mount-dfs/'+{FILE!r},'--state-dir',str(state),'--ready-file',str(state/'ready.json'),'--trigger-file',str(state/'trigger'),'--result-file',str(state/'writer-result.json'),'--size','67108864','--trigger-timeout','120','--close-deadline','30']
with (state/'writer.log').open('wb') as out:
    p=subprocess.Popen(args,stdin=subprocess.DEVNULL,stdout=out,stderr=subprocess.STDOUT,start_new_session=True)
report['worker_pid']=p.pid;report['argv']=args
print(json.dumps(report))
''')
        assert before_a['boot_id'] != before_b['boot_id']
        ready=guest('afs-accept-b','03-durable-watermark',f'''import json,time,pathlib
path=pathlib.Path({STATE!r}+'/ready.json');deadline=time.monotonic()+29
while not path.exists() and time.monotonic()<deadline:time.sleep(.1)
assert path.exists(), 'READY not reached before 30s operation deadline'
ready=json.loads(path.read_text());assert ready['durable_watermark']['fsync_completed']
assert ready['durable_watermark']['length']==67108864
assert ready['durable_watermark']['write_fsync_elapsed_ms']<30000
assert ready['payload']['sha256']==ready['durable_watermark']['sha256']
print(json.dumps(ready))
''')
        print('READY 64 MiB persisted; injecting exact owner SIGKILL',flush=True)
        fault=guest('afs-accept-a','04-kill-owner',IDENTITY+f'''
run={A!r};node=identity(706396,run+'/bin/afs-node',{NODE_SHA!r});meta=identity(706366,run+'/bin/afs-meta',{META_SHA!r})
os.kill(node['pid'],9);deadline=time.monotonic()+2
while pathlib.Path('/proc/706396/stat').exists() and time.monotonic()<deadline:
    text=pathlib.Path('/proc/706396/stat').read_text()
    if text[text.rfind(')')+2:].split()[0]=='Z':break
    time.sleep(.05)
dead=not pathlib.Path('/proc/706396/stat').exists() or pathlib.Path('/proc/706396/stat').read_text().split(') ')[1].split()[0]=='Z'
assert dead
print(json.dumps({{'signal':'SIGKILL','node_before':node,'node_dead':dead,'meta_after':identity(706366,run+'/bin/afs-meta',{META_SHA!r}),'at_unix':time.time(),'root_after':root_status(run)}}))
''')
    close=guest('afs-accept-b','05-close-after-crash',f'''import json,time,pathlib
state=pathlib.Path({STATE!r});(state/'trigger').write_text('close after verified SIGKILL')
deadline=time.monotonic()+30
while not (state/'writer-result.json').exists() and time.monotonic()<deadline:time.sleep(.1)
assert (state/'writer-result.json').exists(), 'close result absent at deadline'
r=json.loads((state/'writer-result.json').read_text());assert r['close']['completed'] and r['close']['duration_ms']<30000,r
print(json.dumps(r))
''')
    debt=guest('afs-accept-b','06-debt-before-restart',f'''import json,time,pathlib
p=pathlib.Path({B!r}+'/logs/node.log')
with p.open('rb') as f:f.seek({before_b['log_offset']});text=f.read().decode()
print(json.dumps({{'at_unix':time.time(),'node_log_delta':text}}))
''')
    restarted=guest('afs-accept-a','07-restart-same-owner-disk',IDENTITY+f'''
run={A!r}
assert not pathlib.Path('/proc/706396/exe').exists()
meta=identity(706366,run+'/bin/afs-meta',{META_SHA!r})
assert hashlib.sha256(pathlib.Path(run+'/bin/afs-node').read_bytes()).hexdigest()=={NODE_SHA!r}
for kind in ('dfs','ownerfs'):
    target=run+'/mount-'+kind
    r=subprocess.run(['findmnt','-J','-M',target,'-o','SOURCE,FSTYPE,TARGET'],text=True,capture_output=True)
    if r.returncode==0:
        m=json.loads(r.stdout)['filesystems'][0];assert m['target']==target and m['fstype'].startswith('fuse'),m
        subprocess.run(['fusermount3','-u',target],check=True,timeout=3)
with pathlib.Path(run+'/logs/node-restart-v45.log').open('wb') as output:
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
print(json.dumps({{'node':identity(p.pid,run+'/bin/afs-node',{NODE_SHA!r}),'meta':identity(706366,run+'/bin/afs-meta',{META_SHA!r}),'mounts':ms,'health':h,'root':root_status(run),'restart_ready_ms':(time.monotonic()-start)*1000,'at_unix':time.time(),'config_sha256':hashlib.sha256(pathlib.Path(run+'/node.toml').read_bytes()).hexdigest()}}))
''',timeout=55)
    print('Owner restarted on same disk; validating first successful fresh read',flush=True)
    checked=guest('afs-accept-b','08-fresh-read',f'''import json,subprocess,pathlib,time
args=['python3','/var/tmp/afs-v45-owner-restart.py','check','--target',{B!r}+'/mount-dfs/'+{FILE!r},'--ready-file',{STATE!r}+'/ready.json','--result-file',{STATE!r}+'/check-result.json','--open-timeout','30']
start=time.monotonic();r=subprocess.run(args,capture_output=True,text=True,timeout=30)
result=json.loads(pathlib.Path({STATE!r}+'/check-result.json').read_text()) if pathlib.Path({STATE!r}+'/check-result.json').exists() else {{}}
print(json.dumps({{'argv':args,'exit':r.returncode,'stdout':r.stdout,'stderr':r.stderr,'result':result,'elapsed_ms':(time.monotonic()-start)*1000}}))
''')
    final_b=guest('afs-accept-b','09-retirement-after-restart',IDENTITY+f'''
time.sleep(6);run={B!r}
with pathlib.Path(run+'/logs/node.log').open('rb') as f:f.seek({before_b['log_offset']});text=f.read().decode()
print(json.dumps({{'node':identity(67058,run+'/bin/afs-node',{NODE_SHA!r}),'at_unix':time.time(),'log_delta':text}}))
''')
    result={'scope':'64MiB R1 memory Meta owner Node crash slice; not complete release cases','source_head_before_uncommitted_fix':'a5efb7b8a11764b0689666ecfb1b238637a90476','node_sha256':NODE_SHA,'meta_sha256':META_SHA,'source_manifest':json.loads((ROOT/'.local/v45-host-source-hashes.json').read_text()),'before_a':before_a,'before_b':before_b,'ready':ready,'fault':fault,'close':close,'debt':debt,'restart':restarted,'read':checked,'after_b':final_b}
    result['status']='PASS' if checked['exit']==0 and checked['result'].get('status')=='PASS' else 'FAIL'
    (EVIDENCE/'report.json').write_text(json.dumps(result,indent=2))
    print(result['status'],flush=True)
    return 0 if result['status']=='PASS' else 1

if __name__=='__main__':
    try:
        raise SystemExit(main())
    except Exception as exc:
        (EVIDENCE/'report-failure.json').write_text(json.dumps({'status':'FAIL','exception':type(exc).__name__,'message':str(exc)},indent=2))
        raise
