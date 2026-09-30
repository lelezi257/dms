import json,time,pathlib
time.sleep(6)
p=pathlib.Path('/mnt/lima-afsbdata/afs-delivery/p2-memory-peer-v44'+'/logs/node.log')
with p.open('rb') as f:f.seek(13687);text=f.read().decode()
print(json.dumps({'at_unix':time.time(),'node_log_delta':text}))
