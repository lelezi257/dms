#!/usr/bin/env python3
"""Scoped product TCP outage on Linux B; immutable evidence, automatic rollback.

This interrupts only B -> the owned A business gRPC port. It does not claim
RoCE link interruption, formal acceptance, or persistent Meta recovery.
"""
import hashlib
import json
import os
import pathlib
import platform
import socket
import subprocess
import sys
import time

RUN = pathlib.Path('/mnt/lima-afsbdata/afs-delivery/round1-mainline-v77-archive-async')
OUT = pathlib.Path('/mnt/lima-afsbdata/afs-delivery/round2-network-v79b')
WORKER = '/home/lzc.guest/round2-mainline-v78.py'
CHAIN = 'AFS_R2_V79B'
DEST = '192.168.109.12'
PORT = '19982'


def record(name, value):
    (OUT / name).write_text(json.dumps(value, indent=2) + '\n')


def command(name, argv, timeout=15, allow_failure=False):
    started = time.monotonic()
    result = subprocess.run(argv, text=True, capture_output=True, timeout=timeout)
    value = {'argv': argv, 'exit': result.returncode, 'stdout': result.stdout,
             'stderr': result.stderr, 'elapsed_seconds': time.monotonic()-started,
             'utc': time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime())}
    record(name, value)
    if not allow_failure:
        assert result.returncode == 0, value
    return value


def identity(name):
    result = command(name, ['python3', WORKER, 'b', 'identity'])
    return json.loads(result['stdout'])['identity']


def tcp(name, expected):
    started = time.monotonic()
    try:
        with socket.create_connection((DEST, int(PORT)), timeout=2):
            outcome, detail = 'connected', None
    except OSError as error:
        outcome, detail = 'error', str(error)
    record(name, {'destination': DEST, 'port': int(PORT), 'outcome': outcome,
                  'error': detail, 'elapsed_seconds': time.monotonic()-started})
    assert outcome == expected, (name, outcome)


def read(name, kind, expect):
    result = command(name, ['timeout', '--signal=TERM', '--kill-after=1', '29',
                            'python3', WORKER, 'b', 'read', '--kind', kind,
                            '--expect', expect], timeout=32)
    value = json.loads(result['stdout'])
    assert value['elapsed_seconds'] < 30, value
    return value


def controller(name, action):
    return command(name, [str(RUN/'prefix/bin/afs-processctl'), '--prefix', str(RUN/'prefix'),
                          '--config-dir', str(RUN/'etc'), '--run-dir', str(RUN/'run'),
                          '--log-dir', str(RUN/'log'), action, 'node'], timeout=60)


def main():
    assert platform.system() == 'Linux' and platform.machine() == 'aarch64'
    assert os.geteuid() == 0
    assert not OUT.exists(), 'Evidence output must be fresh'
    OUT.mkdir()
    record('scope.json', {'source_sha256': hashlib.sha256(pathlib.Path(__file__).read_bytes()).hexdigest(),
                         'runtime': str(RUN), 'chain': CHAIN, 'blocked': f'{DEST}:{PORT}/tcp',
                         'formal_acceptance': 'NOT_RUN', 'environment': 'PREPARING'})
    before = identity('identity-before.json')
    command('iptables-before.json', ['iptables-save'])
    absent = command('chain-guard.json', ['iptables', '-S', CHAIN], allow_failure=True)
    assert absent['exit'] != 0, 'Never mutate an existing chain'
    tcp('tcp-before.json', 'connected')
    controller('cold-restart.json', 'restart')
    cold = identity('identity-cold.json')
    assert cold['pid'] != before['pid'] and cold['boot_id'] == before['boot_id']

    # A detached, bounded Linux watchdog restores only the rule owned here even
    # if this process or its controlling host disappears. Unique chain guard
    # prevents accidentally cleaning a pre-existing user firewall rule.
    cleanup = ['iptables', '-D', 'OUTPUT', '-d', DEST, '-p', 'tcp', '--dport', PORT, '-j', CHAIN]
    watchdog_body = ('sleep 90; ' + ' '.join(cleanup) + ' 2>/dev/null; '
                     f'iptables -F {CHAIN} 2>/dev/null; iptables -X {CHAIN} 2>/dev/null')
    watchdog_log = (OUT/'watchdog.log').open('w')
    watchdog = subprocess.Popen(['bash', '-c', watchdog_body], start_new_session=True,
                                stdin=subprocess.DEVNULL, stdout=watchdog_log, stderr=subprocess.STDOUT)
    record('watchdog.json', {'pid': watchdog.pid, 'argv': ['bash', '-c', watchdog_body], 'ttl_seconds': 90})
    restored = False
    try:
        command('chain-create.json', ['iptables', '-N', CHAIN])
        command('chain-drop.json', ['iptables', '-A', CHAIN, '-j', 'DROP'])
        command('jump-install.json', ['iptables', '-I', 'OUTPUT', '1', '-d', DEST,
                                     '-p', 'tcp', '--dport', PORT, '-j', CHAIN])
        (OUT/'fault-active').write_text(str(time.time()))
        tcp('tcp-blocked.json', 'error')
        owner = read('owner-blocked.json', 'ownerfs', 'error')
        assert owner['bytes_before_error'] == 0
        read('dfs-during.json', 'dfs', 'success')
        command('iptables-during.json', ['iptables', '-L', CHAIN, '-nvx'])
        # Permit the host to check unaffected C->A Home reads during this fault.
        (OUT/'fault-reads-complete').write_text(str(time.time()))
        time.sleep(20)
    finally:
        command('jump-remove.json', cleanup, allow_failure=True)
        command('chain-flush.json', ['iptables', '-F', CHAIN], allow_failure=True)
        command('chain-remove.json', ['iptables', '-X', CHAIN], allow_failure=True)
        after = command('iptables-after.json', ['iptables-save'])
        restored = CHAIN not in after['stdout']
        record('rollback.json', {'restored': restored})
        watchdog_log.close()
    assert restored, 'Owned firewall rule remains'
    tcp('tcp-after.json', 'connected')
    recovered = read('owner-recovered.json', 'ownerfs', 'success')
    read('dfs-recovered.json', 'dfs', 'success')
    assert recovered['elapsed_seconds'] < 60
    assert identity('identity-final.json') == cold
    record('result.json', {'status': 'PASS', 'scope': 'B-to-A owned TCP control/data RPC port outage',
                           'formal_acceptance': 'NOT_RUN', 'environment': 'PREPARING',
                           'rdma_link_fault': False, 'firewall_restored': restored})
    print(json.dumps({'status': 'PASS', 'evidence': str(OUT)}), flush=True)


if __name__ == '__main__':
    try:
        main()
    except BaseException as error:
        if OUT.is_dir():
            record('failure.json', {'type': type(error).__name__, 'error': str(error)})
        raise
