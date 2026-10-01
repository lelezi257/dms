"""Capture preserved cohort recovery without changing files or deployment."""
import hashlib,json,os,pathlib,platform,subprocess,sys,tarfile,time,urllib.request
assert sys.platform=='linux' and platform.machine()=='aarch64' and os.geteuid()==0
which=sys.argv[1];vol=pathlib.Path('/mnt/lima-afs'+('ctlstate' if which=='ctl' else which+'data'))
r=vol/'afs-delivery/round1-mainline-v77-archive-async';role='meta' if which=='ctl' else 'node'
pid=int((r/'run'/f'{role}.pid').read_text());p=pathlib.Path('/proc',str(pid))
digest=lambda x:hashlib.sha256(pathlib.Path(x).read_bytes()).hexdigest()
expected={'meta':'64d85fc3933637ccd7fdf4eafded5e5c8cb2354e73b4c8d1a27b2b8d43f4c7e7','node':'d28d17faa8bb71eb8c0a28241e8556396e440b204cf0242d2af8133700f6a494'}
assert digest(p/'exe')==expected[role]
ip={'ctl':11,'a':12,'b':13,'c':14}[which];port={'ctl':19981,'a':19983,'b':19985,'c':19987}[which]
with urllib.request.build_opener(urllib.request.ProxyHandler({})).open(f'http://192.168.109.{ip}:{port}/health',timeout=3) as f:health=json.loads(f.read())
assert health['status']=='ready'
value={'status':'PASS','utc':time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime()),'pid':pid,'start_ticks':p.joinpath('stat').read_text().rsplit(') ',1)[1].split()[19],'sha256':expected[role],'boot_id':pathlib.Path('/proc/sys/kernel/random/boot_id').read_text().strip(),'health':health,'files':[],'rdma':subprocess.check_output(['rdma','link','show'],text=True),'data_available_bytes':os.statvfs(vol).f_bavail*os.statvfs(vol).f_frsize,'formal_acceptance':'NOT_RUN'}
assert value['data_available_bytes']>=4*1024**3
if which!='ctl':
 for path in (r/'mount/ownerfs/workspace-round1-v77/data.bin',r/'mount/dfs/data.bin'):
  fd=os.open(path,os.O_RDONLY);h=hashlib.sha256();length=0
  try:
   while block:=os.read(fd,262144):h.update(block);length+=len(block)
   assert os.read(fd,1)==b''
  finally:os.close(fd)
  assert length==4194321 and h.hexdigest()=='7fa1366968edf9a8490fffac3eab7e0b1619480a152d8e75c7471f753a8c1231'
  value['files'].append({'path':str(path),'length':length,'sha256':h.hexdigest(),'eof':True})
out=pathlib.Path('/home/lzc.guest/round2-v82-sleep-recovery');out.mkdir(exist_ok=True)
assert not (out/'after.json').exists();(out/'after.json').write_text(json.dumps(value,indent=2)+'\n')
if which=='ctl':assert pid==1418880
else:assert int((out/'pid-before').read_text())!=pid and (out/'start.exit').read_text().strip()=='0'
archive=pathlib.Path('/home/lzc.guest')/f'round2-v82-sleep-recovery-{which}.tar.gz'
with tarfile.open(archive,'w:gz') as t:t.add(out,arcname=which)
os.chown(archive,501,20);print(json.dumps(value))
