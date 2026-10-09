#!/usr/bin/env python3
"""Read-only admission of an archived stopped R3 fixture; no service control."""
import argparse
import json
import os
from pathlib import Path
import socket

from probes.dfs_r3_fixture import Fixture, require, safe


def fields(path):
    return dict(line.split('=', 1) for line in Path(path).read_text().splitlines() if '=' in line)


def verify_closed(value, *, role, root, binary_sha, boot_id, records, gone):
    name = 'meta' if role == 'ctl' else 'node'
    frozen = value.get('frozen', {})
    wait = value.get('actual_wait', {})
    child = wait.get('child', {})
    lifecycle = Path(wait.get('path', '/invalid'))
    require(value.get('status') == 'PASS' and value.get('mode') == 'closed'
            and value.get('role') == role and value.get('frozen', {}).get('root') == str(root)
            and frozen.get('executable') == str(root / 'prefix/bin' / ('afs-' + name))
            and frozen.get('executable_sha256') == binary_sha
            and lifecycle.parent == root / 'run' and lifecycle.name.startswith(name + '.lifecycle.')
            and child.get('lifecycle') == str(lifecycle)
            and child.get('exe') == frozen.get('executable')
            and child.get('config') == str(root / 'etc' / (name + '.toml'))
            and child.get('boot_id') == boot_id
            and all(child.get(key, '').isdigit() and int(child[key]) > 0
                    for key in ('pid', 'supervisor_pid', 'start_ticks'))
            and child['pid'] != child['supervisor_pid']
            and wait.get('ready') == {'supervisor_pid': child['supervisor_pid']}
            and wait.get('exit') == dict(child, exit_code='0')
            and records == wait and gone is True,
            'prior exact wait0/identity or stopped incarnation missing')


def verify_restart(before, after, first_closed, nodes_before, nodes_after):
    require(before.get('status') == after.get('status') == first_closed.get('status') == 'PASS'
            and before.get('mode') == after.get('mode') == 'capture'
            and first_closed.get('mode') == 'closed'
            and first_closed['actual_wait']['child'] == before['child']
            and first_closed['actual_wait']['exit'] == dict(before['child'], exit_code='0')
            and before['frozen'] == after['frozen']
            and before['initial_sha256'] == after['initial_sha256']
            and before['child']['boot_id'] == after['child']['boot_id']
            and before['child']['pid'] != after['child']['pid']
            and before['child']['supervisor_pid'] != after['child']['supervisor_pid']
            and int(before['child']['start_ticks']) < int(after['child']['start_ticks'])
            and before['lifecycle'] != after['lifecycle']
            and set(nodes_before) == {'a', 'b', 'c'} and nodes_before == nodes_after,
            'Meta successor or stable three Node/mount identities not proved')


class RetainedFixture(Fixture):
    def __init__(self, *args, closed, **kwargs):
        super().__init__(*args, **kwargs)
        self.closed = closed

    def patch(self):
        raise RuntimeError('retained recovery admission is read-only; policy patch forbidden')

    def fresh(self):
        # Called by inherited preflight. This lane never calls patch or writes config.
        value = json.loads(self.closed.read_text())
        lifecycle = Path(value['actual_wait']['path'])
        safe(lifecycle, self.root)
        records = {'path': str(lifecycle), **{key: fields(safe(lifecycle / key, self.root))
                                            for key in ('child', 'ready', 'exit')}}
        gone = all(not (Path('/proc') / value['actual_wait']['child'][key]).exists()
                   for key in ('pid', 'supervisor_pid'))
        verify_closed(value, role=self.role, root=self.root, binary_sha=self.sha[self.name],
            boot_id=Path('/proc/sys/kernel/random/boot_id').read_text().strip(), records=records, gone=gone)
        require(any(safe(self.root / 'state' / self.name, self.root).iterdir()), 'retained state missing')
        for port in self.ports():
            with socket.socket() as sock:
                sock.bind(('0.0.0.0', port))
        require(not any(str(self.root) in row for row in Path('/proc/self/mountinfo').read_text().splitlines()),
                'retained fixture has a live mount')
        require(not os.path.lexists(self.root / 'run/node.sock'), 'retained fixture UDS remains')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--role', choices=('ctl', 'a', 'b', 'c'), required=True)
    parser.add_argument('--fixture-name', required=True)
    parser.add_argument('--meta-sha256', required=True)
    parser.add_argument('--node-sha256', required=True)
    parser.add_argument('--ceiling-bytes', type=int, required=True)
    parser.add_argument('--floor-bytes', type=int, required=True)
    parser.add_argument('--closed', type=Path, required=True)
    args = parser.parse_args()
    fixture = RetainedFixture(args.role, args.fixture_name,
        {'meta': args.meta_sha256, 'node': args.node_sha256}, closed=args.closed,
        ceiling_bytes=args.ceiling_bytes, floor_bytes=args.floor_bytes)
    try:
        fixture.guest()
        result = fixture.preflight()
        result['status'] = 'PASS_RETAINED_ADMISSION_ONLY'
        code = 0
    except Exception as error:
        result, code = {'status': 'BLOCKED', 'error': str(error)}, 1
    print(json.dumps(dict(result, role=args.role, root=str(fixture.root),
                          commands=fixture.commands), indent=2))
    return code


if __name__ == '__main__':
    raise SystemExit(main())
