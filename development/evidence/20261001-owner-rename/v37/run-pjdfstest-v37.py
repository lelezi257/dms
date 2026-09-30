import hashlib, importlib.util, json, os, platform, subprocess, sys
from pathlib import Path
assert platform.system() == 'Linux' and os.geteuid() == 0
backend=sys.argv[1]
assert backend in ('ownerfs','dfs')
spec=importlib.util.spec_from_file_location('consistency', '/var/tmp/afs-consistency-v37.py')
probe=importlib.util.module_from_spec(spec); spec.loader.exec_module(probe)
runtime=Path('/mnt/lima-afsadata/afs-delivery/p2-memory-lane-v37')
mount=runtime/('mount-'+backend)
expected=[('node',576329,'143294f76870794aab05f7f314287622367914f98b329c98b5fccd3a60a88faf'),('meta',576299,'ffe7268031438318de65ee999e4872846974784141fba4d85ad451865a958d01')]
run=Path('/mnt/lima-afsadata/afs-delivery/evidence/p2-posix-v37')/backend
run.mkdir(parents=True,exist_ok=False)
def save(name,obj): (run/name).write_text(json.dumps(obj,indent=2,sort_keys=True)+'\n')
def collect():
 identity=probe._worker_identity(mount,expected)
 assert identity['all_expected_processes_ok'] and identity['boot_id'] and identity['machine_id'],identity
 fs=json.loads(identity['mount']['stdout'])['filesystems']
 assert len(fs)==1 and fs[0]['source']=='afs-'+backend and fs[0]['fstype'].startswith('fuse') and fs[0]['target']==str(mount),fs
 return identity
pre=collect(); save('target-before.json',pre)
configs={name:hashlib.sha256((runtime/(name+'.toml')).read_bytes()).hexdigest() for name in ('node','meta')}
save('config-sha256.json',configs)
base=mount/'std01-v37'
if backend=='ownerfs': base=mount/'owner-memory-lane-20260930T181239Z'/'std01-v37'
base.mkdir(mode=0o755)
driver=Path('/mnt/lima-afsadata/afs-delivery/evidence/p2-posix-owner-v36/driver-current/experiments/afs-acceptance/drivers/standard.py')
argv=['python3',str(driver),'--profile','full','--timeout','1800','--suite-root','/mnt/lima-afsadata/afs-acceptance/suites-reference/src/pjdfstest','--backend',backend,'--meta','memory','--mount',str(mount),'--base-dir',str(base),'--run-dir',str(run/'driver-result'),'--process-pid','576329','--meta-process-pid','576299']
save('driver-input.json',{'argv':argv,'driver_sha256':hashlib.sha256(driver.read_bytes()).hexdigest(),'wrapper_sha256':hashlib.sha256(Path(__file__).read_bytes()).hexdigest()})
with (run/'driver.stdout.json').open('w') as out, (run/'driver.stderr.log').open('w') as err:
 result=subprocess.run(argv,stdout=out,stderr=err,timeout=1860,check=False)
post=collect(); save('target-after.json',post)
keys=('pid','start_ticks','sha256','exe_path','exe_dev','exe_inode')
checks={'boot_id':pre['boot_id']==post['boot_id'],'machine_id':pre['machine_id']==post['machine_id']}
for role,_,_ in expected:
 checks[role]=all(pre['expected_processes'][role][k]==post['expected_processes'][role][k] for k in keys)
checks['config']=configs=={name:hashlib.sha256((runtime/(name+'.toml')).read_bytes()).hexdigest() for name in ('node','meta')}
checks['mount']=json.loads(pre['mount']['stdout'])==json.loads(post['mount']['stdout'])
proof=json.loads((run/'driver-result/artifacts/std-01-pjdfstest/proof.json').read_text())
save('qualification.json',{'status':'PASS' if all(checks.values()) and result.returncode==0 and proof['status']=='PASS' else 'FAIL','identity_checks':checks,'driver_returncode':result.returncode,'driver_status':proof['status'],'accounting':proof['accounting']})
print(json.dumps({'backend':backend,'run':str(run),'returncode':result.returncode,'identity_checks':checks,'status':proof['status'],'accounting':proof['accounting']},sort_keys=True))
assert all(checks.values()),checks
sys.exit(result.returncode)
