import json,time,pathlib
path=pathlib.Path('/var/tmp/afs-v44-owner-fault'+'/ready.json');deadline=time.monotonic()+29
while not path.exists() and time.monotonic()<deadline:time.sleep(.1)
assert path.exists(), 'READY not reached before 30s operation deadline'
ready=json.loads(path.read_text());assert ready['durable_watermark']['fsync_completed']
assert ready['durable_watermark']['length']==67108864
assert ready['durable_watermark']['write_fsync_elapsed_ms']<30000
assert ready['payload']['sha256']==ready['durable_watermark']['sha256']
print(json.dumps(ready))
