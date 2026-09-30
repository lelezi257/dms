import json,time,pathlib
state=pathlib.Path('/var/tmp/afs-v44-owner-fault');(state/'trigger').write_text('close after verified SIGKILL
')
deadline=time.monotonic()+30
while not (state/'writer-result.json').exists() and time.monotonic()<deadline:time.sleep(.1)
assert (state/'writer-result.json').exists(), 'close result absent at deadline'
r=json.loads((state/'writer-result.json').read_text());assert r['close']['completed'] and r['close']['duration_ms']<30000,r
print(json.dumps(r))
