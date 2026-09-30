import json,subprocess,pathlib,time
args=['python3','/var/tmp/afs-v44-owner-restart.py','check','--target','/mnt/lima-afsbdata/afs-delivery/p2-memory-peer-v44/mount-dfs/owner-restart-v44-64m.bin','--ready-file','/var/tmp/afs-v44-owner-fault/ready.json','--result-file','/var/tmp/afs-v44-owner-fault/diagnostic-check-result.json','--open-timeout','30']
start=time.monotonic();r=subprocess.run(args,capture_output=True,text=True,timeout=30)
path=pathlib.Path('/var/tmp/afs-v44-owner-fault/diagnostic-check-result.json')
print(json.dumps({'argv':args,'exit':r.returncode,'stdout':r.stdout,'stderr':r.stderr,'result':json.loads(path.read_text()) if path.exists() else {},'elapsed_ms':(time.monotonic()-start)*1000}))
