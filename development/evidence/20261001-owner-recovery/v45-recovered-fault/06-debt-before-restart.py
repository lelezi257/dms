import json,time,pathlib
p=pathlib.Path('/mnt/lima-afsbdata/afs-delivery/p2-memory-peer-v45'+'/logs/node.log')
with p.open('rb') as f:f.seek(648);text=f.read().decode()
print(json.dumps({'at_unix':time.time(),'node_log_delta':text}))
