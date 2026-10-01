#!/usr/bin/env python3
"""Two pre-opened product OwnerFs handles through a separate Linux Node."""
import importlib.util
import json
import os
import pathlib
import time
spec = importlib.util.spec_from_file_location('capacity', pathlib.Path(__file__).with_name('round2-capacity.py'))
c = importlib.util.module_from_spec(spec)
spec.loader.exec_module(c)

run = pathlib.Path('/mnt/lima-afsbdata/afs-delivery/owner-eio-v80-r2-b')
result = {'started_utc': time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime()),
          'formal_acceptance': 'NOT_RUN', 'environment': 'PREPARING'}
handles = []
resize_fd = None
try:
    path = run/'mount/ownerfs/workspace-eio-v80/remote.bin'
    for mode, flags in [('plain', os.O_RDWR), ('dsync', os.O_RDWR|os.O_DSYNC)]:
        handles.append((mode, os.open(path, flags)))
    plain_fd = handles[0][1]
    os.lseek(plain_fd, 8192, os.SEEK_SET)
    result['plain_write_before_fault'] = c.io_call('write', lambda: os.write(plain_fd, b'\x81'*8192))
    assert result['plain_write_before_fault'].get('value') == 8192
    resize_fd = os.open(run/'mount/ownerfs/workspace-eio-v80/remote-size.bin', os.O_RDWR)
    result['resize_before_fault'] = c.io_call('ftruncate', lambda: os.ftruncate(resize_fd, 16384))
    assert result['resize_before_fault']['outcome'] == 'success'
    (run/'evidence/remote-ready').write_text('two-handles-open\n')
    deadline = time.monotonic()+60
    while not (run/'evidence/remote-go').exists():
        assert time.monotonic() < deadline, 'controller did not arm fault'
        time.sleep(.05)
    result['triggered_utc'] = time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime())
    for mode, fd in reversed(handles):
        os.lseek(fd, 8192, os.SEEK_SET)
        ops = [result['plain_write_before_fault'] if mode == 'plain' else c.io_call('write', lambda: os.write(fd, b'\x81'*8192)),
               c.io_call('fdatasync', lambda: os.fdatasync(fd)),
               c.io_call('fsync', lambda: os.fsync(fd)),
               c.io_call('close', lambda: os.close(fd))]
        result[mode] = ops
        (run/'evidence'/f'remote-{mode}.json').write_text(json.dumps(ops, indent=2)+'\n')
        assert all(op.get('errno') == 5 for op in ops[1:]), ops
        if mode == 'dsync': assert ops[0].get('errno') == 5, ops
    handles.clear()
    result['resize_close'] = c.io_call('close', lambda: os.close(resize_fd))
    resize_fd = None
    assert result['resize_close'].get('errno') == 5, result['resize_close']
    result['status'] = 'PASS'
except BaseException as error:
    result.update(status='FAIL', error=repr(error)); raise
finally:
    if resize_fd is not None:
        try: os.close(resize_fd)
        except OSError: pass
    for _, fd in handles:
        try: os.close(fd)
        except OSError: pass
    (run/'evidence/remote-result.json').write_text(json.dumps(result, indent=2)+'\n')
    print(json.dumps(result, indent=2), flush=True)
