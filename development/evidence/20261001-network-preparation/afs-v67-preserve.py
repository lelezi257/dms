import pathlib,json,hashlib,os,subprocess,time
rows=[]
for p in pathlib.Path('/proc').glob('[0-9]*'):
 try:
  exe=str((p/'exe').resolve(strict=True))
  if pathlib.Path(exe).name not in ('afs-node','afs-meta'): continue
  st=(p/'stat').read_text().rsplit(') ',1)[1].split()
  argv=(p/'cmdline').read_bytes().split(b'\0')
  configs=[]
  for v in argv:
   path=pathlib.Path(os.fsdecode(v))
   if path.suffix=='.toml' and path.is_file():configs.append({'path':str(path),'sha256':hashlib.sha256(path.read_bytes()).hexdigest()})
  rows.append({'pid':int(p.name),'start_ticks':st[19],'exe':exe,'exe_sha256':hashlib.sha256(pathlib.Path(exe).read_bytes()).hexdigest(),'configs':configs})
 except (OSError,ValueError):continue
print(json.dumps({'observed_at_unix':time.time(),'hostname':os.uname().nodename,'boot_id':pathlib.Path('/proc/sys/kernel/random/boot_id').read_text().strip(),'machine_id':pathlib.Path('/etc/machine-id').read_text().strip(),'processes':rows,'mountinfo':pathlib.Path('/proc/self/mountinfo').read_text(),'iptables':subprocess.run(['iptables-save'],text=True,capture_output=True).stdout},indent=2))
