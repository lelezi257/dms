#!/usr/bin/env python3
"""Whole-system round-2 normal/recovery regression; current binaries, memory Meta, required RXE."""
import argparse,errno,hashlib,importlib.util,json,os,pathlib,platform,time,tomllib,urllib.parse,urllib.request
spec=importlib.util.spec_from_file_location('base',pathlib.Path(__file__).with_name('round2-capacity.py'));c=importlib.util.module_from_spec(spec);spec.loader.exec_module(c)
c.VOLUMES={'ctl':'/mnt/lima-afsctlstate','a':'/mnt/lima-afsadata','b':'/mnt/lima-afsbdata'}
c.SHA=json.loads(pathlib.Path('/home/lzc.guest/eio-v80-r2-binaries.json').read_text())
NAME='afs-delivery/round2-overall-v82'
PORTS={'ctl':20580,'a':20582,'b':20584}
IPS={'ctl':'192.168.109.11','a':'192.168.109.12','b':'192.168.109.13'}

def fetch(url):
 with urllib.request.build_opener(urllib.request.ProxyHandler({})).open(url,timeout=3) as f:return json.loads(f.read())

def metrics(run,which):
 import re
 url=f'http://{IPS[which]}:{PORTS[which]+1}/metrics'
 with urllib.request.build_opener(urllib.request.ProxyHandler({})).open(url,timeout=3) as f:raw=f.read().decode()
 rows={}
 for line in raw.splitlines():
  if line.startswith('afs_dfs_payload_bytes_total{'):
   labels,value=line.split('} ');d=dict(re.findall(r'(\w+)="([^"]+)"',labels));rows[d['transport']+'/'+d['direction']+'/'+d['operation']]=int(float(value))
 assert set(rows)=={'grpc/recv/replica','grpc/send/read','rdma/recv/replica','rdma/send/read'},rows
 assert rows['grpc/recv/replica']==0 and rows['grpc/send/read']==0,rows
 return {'identity':c.identity(run,'node'),'payload_counters':rows,'rdma_links':c.run_cmd(['rdma','link','show'])['stdout'],'health':fetch(f'http://{IPS[which]}:{PORTS[which]+1}/health')}

def entry(run,which,action,file):
 role='meta' if which=='ctl' else 'node'
 if action=='prepare':
  assert not run.exists();before=c.usage(c.VOLUMES[which]);assert before['available_bytes']>=4*1024**3,before
  candidate=pathlib.Path('/var/lib/afs-acceptance/candidates/eio-v80-r2/prefix')
  assert all(c.digest(candidate/'bin'/f'afs-{r}')==h for r,h in c.SHA.items())
  old=pathlib.Path(c.VOLUMES[which])/c.BASELINE
  cfg=old/'etc'/f'{role}.toml';text=cfg.read_text()
  remove={'grpc_listen','rest_listen','data_dir','uds_path','meta_endpoint','advertise_endpoint','ownerfs_mount','dfs_mount','data_mode','rdma_device'}
  text='\n'.join(l for l in text.splitlines() if not any(l.startswith(k+' =') for k in remove))+'\n'
  text+=f'grpc_listen = "0.0.0.0:{PORTS[which]}"\nrest_listen = "{IPS[which]}:{PORTS[which]+1}"\ndata_dir = "{run}/state/{role}"\n'
  if role=='node':
   text+=f'meta_endpoint = "https://{IPS["ctl"]}:{PORTS["ctl"]}"\nadvertise_endpoint = "https://{IPS[which]}:{PORTS[which]}"\nuds_path = "{run}/run/node.sock"\nownerfs_mount = "{run}/mount/ownerfs"\ndfs_mount = "{run}/mount/dfs"\ndata_mode = "rdma"\nrdma_device = "rxe0"\n'
   assert 'state ACTIVE' in c.run_cmd(['rdma','link','show'])['stdout']
  parsed=tomllib.loads(text);assert parsed['dfs_desired_copies']==2 and parsed['dfs_sync_required_copies']==1
  assert all(':'+str(port)+' ' not in c.run_cmd(['ss','-ltnH'])['stdout'] for port in (PORTS[which],PORTS[which]+1))
  run.mkdir()
  for name in ('etc','run','log','state','mount','evidence'):(run/name).mkdir()
  (run/'prefix').symlink_to(candidate,target_is_directory=True);(run/'etc'/f'{role}.toml').write_text(text)
  return {'configuration':parsed,'config_sha256':c.digest(run/'etc'/f'{role}.toml'),'before':before,'after':c.usage(c.VOLUMES[which]),'candidate_hashes':c.SHA}
 if action in ('start','stop','restart'):
  v={'controller':c.controller(run,action,role)}
  if action!='stop':v['identity']=c.identity(run,role)
  return v
 if action=='identity':return {'identity':c.identity(run,role),'configuration':tomllib.loads((run/'etc'/f'{role}.toml').read_text()),'config_sha256':c.digest(run/'etc'/f'{role}.toml')}
 if action=='metrics':return metrics(run,which)
 if action=='replication':
  chunk=file;polls=[];deadline=time.monotonic()+45
  while True:
   value=fetch(f'http://{IPS["ctl"]}:{PORTS["ctl"]+1}/v1/dfs/chunks/{urllib.parse.quote(chunk,safe="")}/replication');polls.append({'utc':time.time(),'value':value})
   available=value.get('available_copies');states=[x.get('state') for x in value.get('tasks',[]) if x.get('chunk_id')==chunk]
   if (args.expect=='healthy' and available==2 and 'Completed' in states) or (args.expect=='blocked' and available==0 and 'BlockedNoSource' in states):return {'polls':polls,'identity':c.identity(run,role)}
   assert time.monotonic()<deadline,polls;time.sleep(1)
 if action=='workspace':
  assert which=='a'
  workspace='workspace-round2-v82'
  os.mkdir(run/'mount/ownerfs'/workspace)
  value=fetch(f'http://{IPS['ctl']}:{PORTS['ctl']+1}/v1/roots/root-'+workspace.encode().hex())
  assert value['home_node_id']=='round1-a' and value['home_serving'],value
  return {'workspace':workspace,'home':value,'identity':c.identity(run,'node')}
 data=run/'state/node/dfs';info_path=run/'evidence'/('file-'+file+'.json')
 if action=='create':
  payload=bytes((i*193+hashlib.sha256(file.encode()).digest()[(i//251)%32])%251 for i in range(65536))
  before=set((data/'chunks').iterdir()) if (data/'chunks').exists() else set()
  path=run/'mount/dfs'/file;fd=os.open(path,os.O_CREAT|os.O_EXCL|os.O_RDWR,0o600)
  try:assert os.write(fd,payload)==len(payload);os.fdatasync(fd);os.fsync(fd)
  finally:os.close(fd)
  check=c.check_read(path,payload);new=set((data/'chunks').iterdir())-before;assert len(new)==1,new;chunk=new.pop()
  result={'file':file,'chunk':chunk.name,'sha256':check['sha256'],'length':65536,'read':check,'identity':c.identity(run,'node')}
  assert not info_path.exists();info_path.write_text(json.dumps(result,indent=2)+'\n');return result
 info=json.loads(info_path.read_text());chunk=data/'chunks'/info['chunk']
 if action=='inject':
  pidfile=run/'run/node.pid';assert not pidfile.exists() or not (pathlib.Path('/proc')/pidfile.read_text().strip()).exists()
  raw=chunk.read_bytes();assert len(raw)==info['length'] and hashlib.sha256(raw).hexdigest()==info['sha256'];ino=chunk.stat().st_ino
  backup=run/'evidence'/('backup-'+file+'.chunk');assert not backup.exists()
  with backup.open('xb') as f:f.write(raw);f.flush();os.fsync(f.fileno())
  with chunk.open('r+b') as f:f.seek(8192);f.write(bytes([raw[8192]^255]));f.flush();os.fsync(f.fileno())
  assert chunk.stat().st_ino==ino;return {'before_sha256':info['sha256'],'after_sha256':c.digest(chunk),'physical_inode':ino,'chunk':info['chunk'],'length':chunk.stat().st_size,'backup':str(backup)}
 if action=='read':
  try:
   fd=os.open(run/'mount/dfs'/file,os.O_RDONLY)
   try:raw=os.read(fd,65537);tail=os.read(fd,1)
   finally:os.close(fd)
   result={'outcome':'success','bytes':len(raw),'sha256':hashlib.sha256(raw).hexdigest(),'eof':tail==b''}
  except OSError as e:result={'outcome':'error','errno':e.errno,'returned_bytes':0,'error':str(e)}
  return {'read':result,'identity':c.identity(run,'node')}
 if action=='physical':
  txns=[json.loads(l) for l in (data/'catalog.wal').read_text().splitlines() if l.strip()]
  records=[dict(r,txn_revision=t['revision']) for t in txns for r in t['records'] if r['chunk']['id']==info['chunk']]
  return {'chunk':info['chunk'],'sha256':c.digest(chunk),'physical_inode':chunk.stat().st_ino,'records':records,'identity':c.identity(run,'node')}
 raise AssertionError(action)

p=argparse.ArgumentParser();p.add_argument('which',choices=tuple(c.VOLUMES));p.add_argument('action');p.add_argument('--file',default='');p.add_argument('--label',required=True);p.add_argument('--expect',choices=('healthy','blocked'),default='healthy');args=p.parse_args()
assert platform.system()=='Linux' and platform.machine()=='aarch64' and os.geteuid()==0
r=pathlib.Path(c.VOLUMES[args.which])/NAME
try:
 v=entry(r,args.which,args.action,args.file);result={'status':'PASS','which':args.which,'action':args.action,'utc':time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime()),'formal_acceptance':'NOT_RUN','result':v}
except BaseException as e:
 result={'status':'FAIL','which':args.which,'action':args.action,'error':repr(e)}
 if (r/'evidence').exists():c.save(r,args.label,result)
 raise
c.save(r,args.label,result);print(json.dumps(result))
