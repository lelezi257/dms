import copy
import importlib.util
import json
import pathlib
import shutil
import sys
import tempfile

assert sys.platform.startswith('linux')
spec = importlib.util.spec_from_file_location('checker', sys.argv[1])
checker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(checker)
original = pathlib.Path(sys.argv[2])
profile = sys.argv[3] if len(sys.argv) > 3 else 'diagnostic'
checks = []

def reject(name, mutate):
    with tempfile.TemporaryDirectory() as scratch:
        directory = pathlib.Path(scratch) / 'case'
        shutil.copytree(original, directory)
        mutate(directory)
        try:
            checker.audit(directory, 'rxe0', profile)
        except (AssertionError, KeyError, TypeError, ValueError):
            checks.append(name)
        else:
            raise AssertionError('unexpected PASS: ' + name)

def mutate_json(directory, filename, mutate):
    path = directory / filename
    value = json.loads(path.read_text())
    mutate(value)
    path.write_text(json.dumps(value))

def mutate_event(directory, mutate):
    path = directory / 'debugger.jsonl'
    value = [json.loads(line) for line in path.read_text().splitlines()]
    mutate(value)
    path.write_text('\n'.join(json.dumps(event) for event in value) + '\n')

assert checker.audit(original, 'rxe0', profile)['status'] == 'PASS'
checks.append('valid observed flat schema')
for key, value in [('operation', 2), ('bytes', 0), ('data_wr_id', 0), ('poisoned', True), ('qp', -1)]:
    reject('posted ' + key, lambda d, k=key, v=value: mutate_event(d, lambda events: events[0].update({k: v})))
reject('missing posted', lambda d: mutate_event(d, lambda events: events.pop(0)))
reject('failed inferior', lambda d: mutate_event(d, lambda events: events[-1].update({'code': 101})))
reject('reversed timing', lambda d: mutate_event(d, lambda events: events[1].update({'monotonic': 0})))
reject('different process', lambda d: mutate_json(d, 'drained-process.json', lambda p: p.update({'pid': -1})))
reject('missing posted thread', lambda d: mutate_json(d, 'closed-paused-process.json', lambda p: p.update({'tids': []})))
reject('lost paused MR', lambda d: (d / 'closed-paused-mr.json').write_text('[]'))
reject('wrong device', lambda d: mutate_json(d, 'closed-paused-qp.json', lambda rows: [row.update({'ifname': 'other'}) for row in rows]))
def retain_retired_mr(directory):
    row = copy.deepcopy(json.loads((directory / 'connected-mr.json').read_text())[0])
    row.pop('pid', None)
    (directory / 'drained-mr.json').write_text(json.dumps([row]))
reject('retained MR with no live owner', retain_retired_mr)
reject('changed resource identity', lambda d: mutate_json(d, 'closed-paused-cq.json', lambda rows: [row.update({'cqn': -1}) for row in rows]))
reject('nested unsupported JSON', lambda d: (d / 'connected-qp.json').write_text(json.dumps([{'qp': json.loads((d / 'closed-paused-qp.json').read_text())}])))
reject('missing completion', lambda d: (d / 'gdb.log').write_text((d / 'gdb.log').read_text().replace('AFS_RDMA_COMPLETE op=READ bytes=4096', 'removed')))
reject('duplicate completion', lambda d: (d / 'gdb.log').write_text((d / 'gdb.log').read_text() + '\nAFS_RDMA_COMPLETE op=READ bytes=4096\n'))
reject('wrong content claim', lambda d: (d / 'gdb.log').write_text((d / 'gdb.log').read_text().replace('content=EXACT', 'content=BAD')))
reject('replay claim', lambda d: (d / 'gdb.log').write_text((d / 'gdb.log').read_text().replace('replay=ABSENT', 'replay=PRESENT')))
if profile != 'diagnostic':
    reject('arbitrary cancellation is not a deadline', lambda d: mutate_json(d, 'caller-error.json', lambda p: p.update({'kind': 'Cancelled', 'code': 0x01030003, 'message': 'caller cancelled'})))
    reject('missing deadline', lambda d: (d / 'gdb.log').write_text((d / 'gdb.log').read_text().replace('caller=TIMEOUT', 'caller=SUCCESS')))
    if profile == 'owner-deadline':
        reject('client endpoint incorrectly retained', lambda d: shutil.copyfile(d / 'connected-mr.json', d / 'closed-paused-mr.json'))
    else:
        reject('source MR retired before drain', lambda d: mutate_json(d, 'closed-paused-mr.json', lambda rows: rows.pop(0)))
        reject('unexplained absent late write', lambda d: (d / 'gdb.log').write_text((d / 'gdb.log').read_text().replace('content=EXACT', 'content=ABSENT')))
        reject('fixture close must be explicit', lambda d: (d / 'gdb.log').write_text((d / 'gdb.log').read_text().replace('close=EXPLICIT', 'close=AUTO')))
        reject('source must remain alive', lambda d: (d / 'gdb.log').write_text((d / 'gdb.log').read_text().replace('client=RETAINED', 'client=RETIRED')))
    reject('server endpoint removed before drain', lambda d: (d / 'closed-paused-qp.json').write_text('[]'))
print(json.dumps({'status': 'PASS', 'count': len(checks), 'checks': checks}, indent=2))
