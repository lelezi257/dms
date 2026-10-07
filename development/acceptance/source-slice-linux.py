#!/usr/bin/env python3
"""Validate a frozen native-workspace source slice on ARM64 Linux, preserving exits."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import signal
import stat
import subprocess
import time

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--source', type=Path, required=True)
parser.add_argument('--inputs', type=Path, required=True)
parser.add_argument('--target', type=Path, required=True)
parser.add_argument('--out', type=Path, required=True)
parser.add_argument('--labels', nargs='+', help='run only named gates; receipts remain scoped to this subset')
args = parser.parse_args()
args.out.mkdir(exist_ok=False)
env = dict(os.environ, PATH='/home/lzc.linux/.cargo/bin:' + os.environ['PATH'],
           CARGO_TARGET_DIR=str(args.target), CARGO_BUILD_JOBS='2', CARGO_INCREMENTAL='0')
checks = {}


def dump(name, value):
    (args.out / name).write_text(json.dumps(value, indent=2) + '\n')


def admit(name, passed, value):
    checks[name] = {'status': 'PASS' if passed else 'BLOCKED', 'value': value}
    dump('preflight.json', {'status': 'PASS' if all(x['status'] == 'PASS' for x in checks.values()) else 'BLOCKED', 'checks': checks})
    if not passed:
        raise RuntimeError(name)


def quick(command):
    z = subprocess.run(command, cwd=args.source, env=env, capture_output=True, text=True, timeout=60)
    if z.returncode:
        raise RuntimeError({'command': command, 'exit': z.returncode, 'stderr': z.stderr})
    return z.stdout


frozen = json.loads(args.inputs.read_text())
try:
    admit('platform', (platform.system(), platform.machine()) == ('Linux', 'aarch64'), [platform.system(), platform.machine()])
    dependencies = {x: shutil.which(x, path=env['PATH']) for x in ('cargo', 'rustc', 'cc', 'protoc', 'pkg-config', 'findmnt', 'sudo', 'timeout', 'ldd', 'python3')}
    admit('dependencies', all(dependencies.values()), dependencies)
    for tool in ('cargo', 'rustc'):
        version = quick([tool, '--version']).strip()
        admit(tool, '1.95.0' in version, version)
    overrides = {k: env.get(k) for k in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS', 'CARGO_PROFILE_RELEASE_OPT_LEVEL', 'CARGO_PROFILE_RELEASE_DEBUG')}
    admit('profile-overrides', not any(overrides.values()), overrides)
    actual = {p: hashlib.sha256((args.source / p).read_bytes()).hexdigest() for p in frozen['files']}
    admit('inputs', actual == frozen['files'], {'count': len(actual), 'map_sha256': frozen['map_sha256']})
    filesystem = json.loads(quick(['findmnt', '-J', '-T', str(args.target)]))
    admit('ext4', filesystem['filesystems'][0]['fstype'] == 'ext4', filesystem)
    free = shutil.disk_usage(args.target).free
    admit('capacity', free >= 3*2**30, {'free_bytes': free, 'increment_estimate_bytes': int(1.5*2**30), 'reserve_bytes': int(1.5*2**30)})
    admit('fuse-device', stat.S_ISCHR(os.stat('/dev/fuse').st_mode), '/dev/fuse')
    admit('root-physical-tests', subprocess.run(['sudo', '-n', 'true']).returncode == 0, 'sudo -n true')
    admit('ibverbs', bool(quick(['pkg-config', '--modversion', 'libibverbs']).strip()), 'pkg-config')
    metadata = json.loads(quick(['cargo', 'metadata', '--locked', '--offline', '--all-features', '--format-version', '1', '--filter-platform', 'aarch64-unknown-linux-gnu']))
    missing = [p['manifest_path'] for p in metadata['packages'] if not Path(p['manifest_path']).is_file()]
    admit('offline-packages', not missing, {'count': len(metadata['packages']), 'missing': missing})
    admit('cargo-idle', subprocess.run(['pgrep', '-x', 'cargo'], stdout=subprocess.DEVNULL).returncode == 1, 'no concurrent cargo before gates')
except Exception as error:
    dump('preflight.json', {'status': 'BLOCKED', 'checks': checks, 'error': str(error)})
    raise

base = ['cargo', 'test', '--release', '--locked', '--offline', '--all-features', '-p', 'afs']
commands = [
    ('fmt', ['cargo', 'fmt', '--all', '--', '--check'], 60),
    ('control-client', ['python3', '-m', 'unittest', 'discover', '-s', 'scripts/ownerfs', '-p', 'test_native_workspace_control.py', '-v'], 60),
    ('native-control', base + ['--lib', 'native_workspace', '--', '--nocapture'], 900),
    ('native-home', base + ['--lib', 'native_home', '--', '--nocapture'], 600),
    ('contracts', base + ['--test', 'config_contract', '--test', 'vfs_contract', '--test', 'fuse_contract', '--test', 'node_health_contract', '--test', 'ownerfs_peer_contract', '--', '--nocapture'], 600),
    ('library', base + ['--lib', '--', '--nocapture'], 900),
    ('physical-build', base + ['--lib', '--no-run', '--message-format=json'], 300),
    ('physical-native', None, 180),
    ('fuse-build', base + ['--test', 'fuse_contract', '--no-run', '--message-format=json'], 300),
    ('physical-fuse', None, 180),
    ('owner-only', ['cargo', 'check', '--release', '--locked', '--offline', '--no-default-features', '--features', 'ownerfs'], 600),
    ('dfs-only', ['cargo', 'check', '--release', '--locked', '--offline', '--no-default-features', '--features', 'dfs'], 600),
    ('clippy', ['cargo', 'clippy', '--release', '--locked', '--offline', '--workspace', '--all-targets', '--all-features', '--', '-D', 'warnings'], 900),
    ('build', ['cargo', 'build', '--release', '--locked', '--offline', '--all-features', '--bins'], 900),
    ('helper-idle', ['python3', '-c', "import subprocess,sys,time,json; p=subprocess.Popen([sys.argv[1],'idle'],stdin=subprocess.DEVNULL,stdout=subprocess.PIPE,stderr=subprocess.PIPE); time.sleep(0.2); assert p.poll() is None,'idle init exited prematurely'; p.terminate(); out,err=p.communicate(timeout=3); print(json.dumps({'returncode':p.returncode,'stdout':out.decode(),'stderr':err.decode()})); assert p.returncode==0,'TERM failed'", str(args.target / 'release' / 'afs-workspace-probe')], 10),
]
if args.labels:
    unknown = set(args.labels) - {label for label, _, _ in commands}
    if unknown:
        parser.error('unknown gate labels: ' + ', '.join(sorted(unknown)))
    commands = [row for row in commands if row[0] in args.labels]
summary = {'source_base': frozen['source_base'], 'map_sha256': frozen['map_sha256'], 'status': 'RUNNING', 'selected_gates': [row[0] for row in commands], 'commands': []}
for label, command, timeout in commands:
    if command is None:
        native = label == 'physical-native'
        rows = [json.loads(line) for line in (args.out / ('physical-build.log' if native else 'fuse-build.log')).read_text().splitlines() if line.startswith('{')]
        paths = {r['executable'] for r in rows if r.get('reason') == 'compiler-artifact' and r.get('executable') and r['profile']['test'] and r['target']['name'] == ('afs' if native else 'fuse_contract')}
        if len(paths) != 1:
            raise RuntimeError({'artifacts': sorted(paths)})
        command = ['sudo', '-n', 'timeout', str(timeout), paths.pop()]
        if native:
            command += ['node::native_workspace::']
        command += ['--ignored', '--test-threads=1', '--nocapture']
    dump(label + '.command.json', {'argv': command, 'cwd': str(args.source), 'target': str(args.target), 'timeout_seconds': timeout})
    start = time.monotonic()
    minimum = shutil.disk_usage(args.target).free
    reason = None
    with (args.out / (label + '.log')).open('w') as log:
        child = subprocess.Popen(command, cwd=args.source, env=env, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
        while child.poll() is None:
            free = shutil.disk_usage(args.target).free
            minimum = min(minimum, free)
            if free < int(1.5*2**30) or time.monotonic()-start > timeout:
                reason = 'capacity-reserve' if free < int(1.5*2**30) else 'timeout'
                os.killpg(child.pid, signal.SIGTERM)
                try:
                    child.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    os.killpg(child.pid, signal.SIGKILL)
                break
            time.sleep(1)
        code = child.wait()
    (args.out / (label + '.exit')).write_text(str(code) + '\n')
    row = {'label': label, 'returncode': code, 'elapsed_seconds': time.monotonic()-start, 'minimum_free_bytes': minimum, 'stop_reason': reason}
    summary['commands'].append(row)
    summary['status'] = 'BLOCKED' if reason else ('FAIL' if code else 'RUNNING')
    dump('source-proof.json', summary)
    print(json.dumps(row), flush=True)
    if code or reason:
        raise SystemExit(1)
actual = {p: hashlib.sha256((args.source / p).read_bytes()).hexdigest() for p in frozen['files']}
summary['inputs_unchanged'] = actual == frozen['files']
summary['status'] = 'PASS' if summary['inputs_unchanged'] else 'FAIL'
summary['binaries'] = {}
for name in ('afs-meta', 'afs-node', 'afs-workspace-probe'):
    path = args.target / 'release' / name
    libraries = quick(['ldd', str(path)])
    (args.out / (name + '.ldd')).write_text(libraries)
    if 'not found' in libraries:
        raise RuntimeError(name + ': missing dynamic library')
    summary['binaries'][name] = {'sha256': hashlib.sha256(path.read_bytes()).hexdigest(), 'bytes': path.stat().st_size}
dump('source-proof.json', summary)
print(json.dumps(summary), flush=True)
