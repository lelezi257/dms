#!/usr/bin/env python3
"""Preflight and bound one identified Linux release build; never repair the VM."""
import hashlib
import json
import os
import pathlib
import platform
import shutil
import signal
import subprocess
import time

if not __debug__:
    raise RuntimeError('validation requires normal Python')
r = pathlib.Path('/var/tmp/afs-e2e-release-20261006-r1')
source = r / 'source'
out = r / 'results'
out.mkdir(exist_ok=False)
inputs = json.loads((r / 'inputs.json').read_text())
env = dict(os.environ, PATH='/home/lzc.linux/.cargo/bin:' + os.environ['PATH'],
           CARGO_TARGET_DIR=str(r / 'target'))
receipt = {'source_commit': inputs['source_commit'], 'profile': 'release',
           'run_root': str(r), 'checks': {}}

def record():
    (out / 'preflight.json').write_text(json.dumps(receipt, indent=2) + '\n')

def check(name, value, passed):
    receipt['checks'][name] = {'value': value, 'status': 'PASS' if passed else 'BLOCKED'}
    record()
    if not passed:
        receipt['status'] = 'BLOCKED'
        record()
        raise RuntimeError(name)

def run(command, *, timeout=60):
    return subprocess.run(command, cwd=source, env=env, text=True,
                          capture_output=True, timeout=timeout)

check('platform', [platform.system(), platform.machine()],
      platform.system() == 'Linux' and platform.machine() == 'aarch64')
tools = {name: shutil.which(name, path=env['PATH']) for name in
         ('cargo', 'rustc', 'cc', 'protoc', 'pkg-config', 'findmnt', 'ldd')}
check('dependencies', tools, all(tools.values()))
for name in ('cargo', 'rustc'):
    result = run([name, '--version'])
    check(name, result.stdout.strip(), result.returncode == 0 and '1.95.0' in result.stdout)
check('profile-overrides', {k: env.get(k) for k in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS',
      'CARGO_PROFILE_RELEASE_OPT_LEVEL', 'CARGO_PROFILE_RELEASE_DEBUG', 'CARGO_PROFILE_RELEASE_LTO')},
      not any(env.get(k) for k in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS',
      'CARGO_PROFILE_RELEASE_OPT_LEVEL', 'CARGO_PROFILE_RELEASE_DEBUG', 'CARGO_PROFILE_RELEASE_LTO')))
actual = {k: hashlib.sha256((source / k).read_bytes()).hexdigest() for k in inputs['files']}
check('154-frozen-inputs', {'count': len(actual), 'map_sha256': inputs['map_sha256']},
      len(actual) == 154 and actual == inputs['files'])
fs = run(['findmnt', '-J', '-T', str(r)])
check('guest-ext4', json.loads(fs.stdout), fs.returncode == 0 and
      json.loads(fs.stdout)['filesystems'][0]['fstype'] == 'ext4')
free_start = shutil.disk_usage(r).free
check('capacity', {'free_bytes': free_start, 'estimated_increment_bytes': 3 * 2**30,
      'reserve_bytes': int(1.5 * 2**30), 'estimate_not_measured_peak': True},
      free_start >= int(4.5 * 2**30))
ib = run(['pkg-config', '--modversion', 'libibverbs'])
check('rdma-build-dependency', ib.stdout.strip(), ib.returncode == 0)
metadata = run(['cargo', 'metadata', '--format-version', '1', '--locked', '--offline',
                '--all-features', '--filter-platform', 'aarch64-unknown-linux-gnu'])
(out / 'metadata.stderr').write_text(metadata.stderr)
check('linux-offline-metadata', {'returncode': metadata.returncode}, metadata.returncode == 0)
packages = json.loads(metadata.stdout)['packages']
missing = [p['manifest_path'] for p in packages if not pathlib.Path(p['manifest_path']).is_file()]
check('offline-source-cache', {'packages': len(packages), 'missing': missing}, not missing)
receipt['status'] = 'PASS'
record()
command = inputs['build_command']
(out / 'build.command.json').write_text(json.dumps({'command': command, 'cwd': str(source),
      'CARGO_TARGET_DIR': env['CARGO_TARGET_DIR'], 'profile': 'release', 'timeout_seconds': 1200}, indent=2)+'\n')
started = time.monotonic()
minimum_free = free_start
stop_reason = None
with (out / 'build.log').open('w') as log, (out / 'capacity.jsonl').open('w') as capacity:
    process = subprocess.Popen(command, cwd=source, env=env, stdout=log, stderr=subprocess.STDOUT,
                               start_new_session=True)
    while process.poll() is None:
        free = shutil.disk_usage(r).free
        elapsed = time.monotonic() - started
        minimum_free = min(minimum_free, free)
        capacity.write(json.dumps({'elapsed_seconds': elapsed, 'free_bytes': free})+'\n')
        capacity.flush()
        if free < int(1.5 * 2**30) or elapsed > 1200:
            stop_reason = 'capacity-reserve' if free < int(1.5 * 2**30) else 'timeout'
            os.killpg(process.pid, signal.SIGTERM)
            try:
                process.wait(timeout=30)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait()
            break
        time.sleep(2)
    code = process.wait()
(out / 'build.exit').write_text(str(code)+'\n')
after = {k: hashlib.sha256((source / k).read_bytes()).hexdigest() for k in inputs['files']}
summary = {'source_commit': inputs['source_commit'], 'profile': 'release', 'returncode': code,
           'elapsed_seconds': time.monotonic()-started, 'minimum_free_bytes': minimum_free,
           'observed_disk_increment_bytes': free_start-minimum_free,
           'stop_reason': stop_reason, 'inputs_unchanged': after == actual,
           'product_acceptance': 'NOT_RUN', 'binaries': {}}
if code == 0 and after == actual:
    for name in ('afs-meta', 'afs-node'):
        path = r / 'target/release' / name
        libraries = run(['ldd', str(path)])
        (out / (name+'.ldd')).write_text(libraries.stdout+libraries.stderr)
        if libraries.returncode != 0 or 'not found' in libraries.stdout:
            stop_reason = 'runtime-library-missing'
        summary['binaries'][name] = {'path': str(path), 'sha256': hashlib.sha256(path.read_bytes()).hexdigest(),
                                    'size_bytes': path.stat().st_size}
summary['status'] = 'PASS' if code == 0 and after == actual and stop_reason is None else (
    'BLOCKED' if stop_reason else 'FAIL')
summary['stop_reason'] = stop_reason
(out / 'build-proof.json').write_text(json.dumps(summary, indent=2)+'\n')
print(json.dumps(summary))
raise SystemExit(0 if summary['status'] == 'PASS' else 1)
