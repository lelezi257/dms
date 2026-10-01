#!/usr/bin/env python3
"""Decode actual /dev/fuse requests in a controlled strace, not timings."""
from collections import Counter
from pathlib import Path
import json
import re
import struct
import sys

OPCODES = {1: 'LOOKUP', 2: 'FORGET', 3: 'GETATTR', 4: 'SETATTR', 5: 'READLINK',
           9: 'MKDIR', 10: 'UNLINK', 12: 'RENAME', 14: 'OPEN', 15: 'READ',
           16: 'WRITE', 18: 'RELEASE', 20: 'FSYNC', 25: 'FLUSH', 26: 'INIT',
           27: 'OPENDIR', 28: 'READDIR', 29: 'RELEASEDIR', 34: 'ACCESS',
           35: 'CREATE', 42: 'BATCH_FORGET', 45: 'RENAME2'}
PHASES = ['absolute', 'native_cwd', 'native_dirfd', 'direct_backing_dirfd', 'old_fuse_dirfd']
DATA_OR_MUTATION = {4, 9, 10, 12, 14, 15, 16, 20, 25, 35, 45}


def decode(trace, actor):
    pending = {}
    active = None
    begun = []
    ended = []
    counts = {name: Counter() for name in PHASES}
    nodes = {name: Counter() for name in PHASES}
    names = {name: Counter() for name in PHASES}
    outside = Counter()
    total = 0
    for line in trace.splitlines():
        match = re.match(r'^\s*(?:\[pid\s+)?(\d+)\]?\s+(.*)$', line)
        if not match:
            continue
        thread, body = int(match[1]), match[2]
        if '<unfinished ...>' in body:
            assert thread not in pending, (thread, body)
            pending[thread] = body.split('<unfinished ...>', 1)[0]
            continue
        if body.startswith('<... ') and ' resumed>' in body:
            assert thread in pending, (thread, body)
            body = pending.pop(thread) + body.split(' resumed>', 1)[1]
        strings = re.findall(r'"((?:\\x[0-9a-fA-F]{2})+)"', body)
        if not strings:
            continue
        payload = bytes.fromhex(strings[0].replace('\\x', ''))
        if body.startswith('write(2') and thread == actor:
            marker = re.fullmatch(rb'DMS_PATH_PHASE_(BEGIN|END)\|([a-z_]+)\|([0-9]+)\n', payload)
            if marker:
                phase = marker[2].decode()
                assert int(marker[3]) == actor and phase in PHASES
                if marker[1] == b'BEGIN':
                    assert active is None and phase not in begun
                    active = phase
                    begun.append(phase)
                else:
                    assert active == phase and phase not in ended
                    ended.append(phase)
                    active = None
                continue
        descriptor = re.match(r'read\(\d+<(.+?)>,', body)
        if descriptor is None:
            continue
        descriptor_path = descriptor[1].split('<', 1)[0]
        # strace -xx hex-encodes descriptor annotations as well as strings;
        # -yy adds nested <char 10:229> before the outer closing bracket.
        # Check the decoded path so ignoring every FUSE request cannot appear
        # as zero native overhead; the FUSE READ/CREATE control must still pass.
        if re.fullmatch(r'(?:\\x[0-9a-fA-F]{2})+', descriptor_path):
            descriptor_path = bytes.fromhex(descriptor_path.replace('\\x', '')).decode()
        if descriptor_path != '/dev/fuse':
            continue
        result = re.search(r'\)\s+=\s+(\d+)(?:\s|$)', body)
        if not result or int(result[1]) == 0:
            continue
        assert len(payload) >= 40, 'truncated FUSE header'
        length, opcode, unique, node, uid, gid, caller, padding = struct.unpack('<IIQQIIII', payload[:40])
        assert length == int(result[1]) and length <= len(payload), 'truncated/malformed FUSE frame'
        total += 1
        if active is None or caller != actor:
            outside[OPCODES.get(opcode, str(opcode))] += 1
            continue
        counts[active][opcode] += 1
        nodes[active][(opcode, node)] += 1
        if opcode == 1:
            name = payload[40:].split(b'\0', 1)[0].decode('utf-8', errors='backslashreplace')
            names[active][name] += 1
    assert begun == ended == PHASES and active is None, (begun, ended, active)
    assert total > 0 and counts['old_fuse_dirfd'][15] >= 16, 'missing real FUSE READ positive control'
    assert counts['old_fuse_dirfd'][35] == 16, 'missing real FUSE CREATE positive control'
    for phase in PHASES[:-1]:
        assert not any(counts[phase][op] for op in DATA_OR_MUTATION), (phase, counts[phase])
    return {'scope': 'A1 actual request paths; in-process authoritative Home fixture; no Node/P2P/timing claim',
            'actor_pid': actor, 'total_fuse_frames': total,
            'phases': {phase: {'request_count': sum(counts[phase].values()),
                'opcodes': {OPCODES.get(op, str(op)): count for op, count in sorted(counts[phase].items())},
                'lookup_names': dict(names[phase]),
                'nodes': [{'opcode': OPCODES.get(op, str(op)), 'nodeid': node, 'count': count}
                          for (op, node), count in sorted(nodes[phase].items())]}
                for phase in PHASES},
            'requests_outside_actor_phase': dict(outside), 'native_data_requests_zero': True,
            'positive_control_verified': True}


if __name__ == '__main__':
    if len(sys.argv) != 3:
        raise SystemExit('usage: ownerfs_native_path_observer.py STRACE_FILE TEST_LOG')
    log = Path(sys.argv[2]).read_text()
    row = next(line.split('native_path_probe ', 1)[1] for line in log.splitlines()
               if 'native_path_probe ' in line)
    actor = json.loads(row)
    assert actor['phase_count'] == 5 and actor['iterations_per_phase'] == 16
    result = decode(Path(sys.argv[1]).read_text(), actor['actor_pid'])
    result['actor'] = actor
    print(json.dumps(result, indent=2) + '\n')
