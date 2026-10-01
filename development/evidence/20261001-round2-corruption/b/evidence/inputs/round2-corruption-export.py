import hashlib,json,os,pathlib,subprocess,sys,tarfile,urllib.request
assert sys.platform=='linux' and os.geteuid()==0
which=sys.argv[1];vol=pathlib.Path('/mnt/lima-afs'+('ctlstate' if which=='ctl' else which+'data'));r=vol/'afs-delivery/round2-corruption-v81';e=r/'evidence'
inputs=e/'inputs';inputs.mkdir(exist_ok=False)
for name in ('round2-corruption.py','round2-capacity.py','owner-eio-bind.py','eio-v80-r2-binaries.json','round2-corruption-export.py'):
 p=pathlib.Path('/home/lzc.guest')/name;(inputs/name).write_bytes(p.read_bytes())
p=pathlib.Path('/home/lzc.guest/round2-corruption-original.py')
if p.exists():(inputs/p.name).write_bytes(p.read_bytes())
role='meta' if which=='ctl' else 'node';pid=int((r/'run'/f'{role}.pid').read_text());assert not pathlib.Path('/proc',str(pid)).exists()
for name in ('ownerfs','dfs'):
 assert subprocess.run(['findmnt','-M',str(r/'mount'/name)],stdout=subprocess.DEVNULL).returncode!=0
old=vol/'afs-delivery/round1-mainline-v77-archive-async';opid=int((old/'run'/f'{role}.pid').read_text());proc=pathlib.Path('/proc',str(opid))
oldidentity={'pid':opid,'start_ticks':(proc/'stat').read_text().rsplit(') ',1)[1].split()[19],'sha256':hashlib.sha256((proc/'exe').read_bytes()).hexdigest(),'argv':[v.decode() for v in (proc/'cmdline').read_bytes().split(b'\0') if v]}
port={'ctl':19981,'a':19983,'b':19985}[which];ip={'ctl':'192.168.109.11','a':'192.168.109.12','b':'192.168.109.13'}[which]
with urllib.request.build_opener(urllib.request.ProxyHandler({})).open(f'http://{ip}:{port}/health',timeout=3) as f:oldidentity['health']=json.loads(f.read())
assert oldidentity['health']['status']=='ready'
s=os.statvfs(vol);reserve=s.f_bavail*s.f_frsize;assert reserve>=4*1024**3,reserve
(e/'cleanup.json').write_text(json.dumps({'status':'PASS','stopped_pid':pid,'mounts_gone':True,'data_available_bytes':reserve,'old_cohort':oldidentity},indent=2)+'\n')
output=pathlib.Path('/home/lzc.guest')/('round2-corruption-v81-'+which+'.tar.gz')
assert not output.exists()
with tarfile.open(output,'w:gz') as t:
 for name in ('evidence','etc','log'):t.add(r/name,arcname=name)
os.chown(output,501,20)
print(json.dumps({'archive':str(output),'sha256':hashlib.sha256(output.read_bytes()).hexdigest(),'files':sum(1 for n in e.rglob('*') if n.is_file()),'reserve':reserve}))
