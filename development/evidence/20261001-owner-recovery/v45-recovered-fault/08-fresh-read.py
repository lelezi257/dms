import json,subprocess,pathlib,time
args=['python3','/var/tmp/afs-v45-owner-restart.py','check','--target','/mnt/lima-afsbdata/afs-delivery/p2-memory-peer-v45'+'/mount-dfs/'+'owner-restart-v45-64m.bin','--ready-file','/var/tmp/afs-v45-owner-fault'+'/ready.json','--result-file','/var/tmp/afs-v45-owner-fault'+'/check-result.json','--open-timeout','30']
start=time.monotonic();r=subprocess.run(args,capture_output=True,text=True,timeout=30)
result=json.loads(pathlib.Path('/var/tmp/afs-v45-owner-fault'+'/check-result.json').read_text()) if pathlib.Path('/var/tmp/afs-v45-owner-fault'+'/check-result.json').exists() else {}
print(json.dumps({'argv':args,'exit':r.returncode,'stdout':r.stdout,'stderr':r.stderr,'result':result,'elapsed_ms':(time.monotonic()-start)*1000}))
