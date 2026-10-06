#!/usr/bin/env python3
"""Bounded Linux-only current-candidate local baseline; immutable result dirs."""
import hashlib
import json
import os
import pathlib
import platform
import statistics
import subprocess
import time

if not __debug__:
    raise RuntimeError('optimized Python disables validation; run without -O/PYTHONOPTIMIZE')

ROOT = pathlib.Path('/var/tmp/afs-e2e-20261006-r1')
OUT = ROOT / 'results/owner-small-r1'
TOOL = ROOT / 'tools/posix-workload'
OWNER = ROOT / 'mount/ownerfs/g2-e2e-owner/perf-small-r1'
EXT4 = ROOT / 'perf-ext4-small-r1'
SHA = {'node': '04b68193d7cdd04dea8c861f07e47336be05281de11a91e9ea8f3890123d1900',
       'meta': '38f0e76a5b4c4afc3efdde3ee7bfa96b0e7e01a0403d556343ec385a86b20d45'}

def save(path, obj):
    path.write_text(json.dumps(obj, indent=2) + '\n')

def digest(path):
    with open(path, 'rb') as stream:
        h = hashlib.sha256()
        for chunk in iter(lambda: stream.read(1024 * 1024), b''):
            h.update(chunk)
    return h.hexdigest()

def cmd(args, timeout=120):
    return subprocess.run(args, text=True, capture_output=True, timeout=timeout, check=True).stdout

def preflight():
    assert platform.system() == 'Linux' and platform.machine() == 'aarch64'
    assert os.geteuid() == 0
    assert digest(TOOL) == 'a2a56e4cddecf4129f32d449562e7455a050fdd0d018f19f716ced7bd06d6054'
    assert 'not found' not in cmd(['ldd', str(TOOL)])
    identities = {}
    for name in ('node', 'meta'):
        pid = int((ROOT / f'run/{name}.pid').read_text())
        identities[name] = {'pid': pid, 'elf_sha256': digest(f'/proc/{pid}/exe')}
        assert identities[name]['elf_sha256'] == SHA[name]
    owner_mount = json.loads(cmd(['findmnt', '-J', '-T', str(OWNER.parent)]))['filesystems'][0]
    disk = json.loads(cmd(['findmnt', '-J', '-T', str(ROOT / 'state/node')]))['filesystems'][0]
    assert owner_mount['source'] == 'afs-ownerfs' and owner_mount['fstype'].startswith('fuse')
    assert disk['fstype'] == 'ext4'
    capacity = os.statvfs(ROOT)
    available = capacity.f_bavail * capacity.f_frsize
    assert available >= 3 * 1024 ** 3, f'need 3GiB free, observed {available}'
    assert not OWNER.exists() and not EXT4.exists(), 'do not overwrite previous data'
    save(OUT / 'preflight.json', {'status': 'PASS', 'kernel': platform.release(),
         'identities': identities, 'owner_mount': owner_mount, 'data_disk': disk,
         'free_bytes': available, 'tool_sha256': digest(TOOL),
         'script_sha256': digest(__file__), 'workload_help': cmd([str(TOOL), '--help'])})
    OWNER.mkdir()
    EXT4.mkdir()
    assert os.stat(EXT4).st_dev == os.stat(ROOT / 'state/node').st_dev

def workload(action, family, target, root, log):
    args = [str(TOOL), action, '--root', str(root), '--family', family,
            '--total-bytes', '67108864', '--block-bytes', '1048576',
            '--concurrency', '1', '--generation', '0', '--trace', str(log.with_suffix('.jsonl'))]
    if action == 'run':
        args += ['--target', target, '--barrier', 'read-close' if family == 'seq-read' else 'fdatasync']
    result = subprocess.run(args, text=True, capture_output=True, timeout=120)
    save(log.with_suffix('.command.json'), {'argv': args, 'returncode': result.returncode})
    log.with_suffix('.stdout').write_text(result.stdout)
    log.with_suffix('.stderr').write_text(result.stderr)
    assert result.returncode == 0, f'{log}: rc={result.returncode}'
    summary = json.loads(result.stdout)
    assert summary['status'] == 'PASS'
    if action == 'run':
        assert summary['total_bytes'] == 67108864 and summary['wall_ns'] > 0
        assert summary['family'] == family and summary['target'] == target
        assert summary['barrier'] == ('read-close' if family == 'seq-read' else 'fdatasync')
        assert summary['operations'] == 64 and len(summary['workers']) == 1
        worker = summary['workers'][0]
        assert worker['io_count'] == 64
        assert worker['io_bytes'] == worker['content_checked_bytes'] == 67108864
        assert worker['file_size_before'] == (67108864 if family == 'seq-read' else 0)
        assert worker['file_size_after'] == 67108864
        assert all(worker[k] == 0 for k in ('open_rc', 'eof_rc', 'barrier_rc', 'close_rc'))
    return summary

def pair_io(family):
    rows = []
    for pair in range(5):
        order = ('ext4', 'owner') if pair % 2 == 0 else ('owner', 'ext4')
        for target in order:
            root = (OWNER if target == 'owner' else EXT4) / f'{family}-{pair}'
            prefix = OUT / f'{family}-{pair}-{target}'
            workload('prepare', family, target, root, prefix.with_name(prefix.name + '-prepare'))
            sample = workload('run', family, target, root, prefix.with_name(prefix.name + '-run'))
            workload('verify', family, target, root, prefix.with_name(prefix.name + '-verify'))
            rows.append({'pair': pair, 'target': target, 'wall_ns': sample['wall_ns'],
                         'mib_per_second': 64 * 1e9 / sample['wall_ns']})
    rates = {t: statistics.median(x['mib_per_second'] for x in rows if x['target'] == t)
             for t in ('owner', 'ext4')}
    ratio = rates['owner'] / rates['ext4']
    result = {'case_id': 'G2.09' if family == 'seq-read' else 'G2.10',
              'correctness': 'PASS', 'performance': 'PASS' if ratio >= .9 else 'FAIL',
              'target_ratio': .9, 'observed_ratio': ratio, 'median_mib_per_second': rates,
              'samples': rows, 'scope': '64MiB C1 buffered; read includes content validation; write includes open/write/fdatasync/close; directory fsync and fresh-open verify outside timer'}
    save(OUT / f'{family}.json', result)
    return result

def pair_delete():
    rows = []
    for pair in range(5):
        for target in (('ext4', 'owner') if pair % 2 == 0 else ('owner', 'ext4')):
            root = (OWNER if target == 'owner' else EXT4) / f'delete-{pair}'
            root.mkdir()
            directory = os.open(root, os.O_RDONLY | os.O_DIRECTORY)
            try:
                for i in range(100):
                    with open(root / f'f{i:03}', 'xb') as f:
                        written = f.write(bytes([i]) * 4096)
                        assert written == 4096
                        f.flush()
                        os.fdatasync(f.fileno())
                os.fsync(directory)
                start = time.monotonic_ns()
                for i in range(100):
                    os.unlink(root / f'f{i:03}')
                elapsed = time.monotonic_ns() - start
                os.fsync(directory)
                assert not list(root.iterdir())
                row = {'pair': pair, 'target': target, 'operations': 100,
                       'wall_ns': elapsed, 'ops_per_second': 100 * 1e9 / elapsed,
                       'mean_unlink_ns': elapsed / 100, 'correctness': 'PASS'}
                save(OUT / f'delete-{pair}-{target}.json', row)
                rows.append(row)
            finally:
                os.close(directory)
    medians = {t: statistics.median(x['ops_per_second'] for x in rows if x['target'] == t)
               for t in ('owner', 'ext4')}
    result = {'case_id': 'G2.11', 'status': 'PASS', 'correctness': 'PASS',
              'performance': 'REPORTED_NO_RATIO_GATE', 'median_ops_per_second': medians,
              'observed_ratio': medians['owner'] / medians['ext4'], 'samples': rows,
              'scope': '100x4KiB files per sample; unlink loop timed; prepare/file sync/directory sync/final empty check outside timer; no new throughput gate'}
    save(OUT / 'delete.json', result)
    return result

def main():
    OUT.mkdir()
    save(OUT / 'frozen-plan.json', {'source_commit': 'e925c5bcf0408851ebfa08a59df29953374da9e9',
         'candidate': 'existing published target/debug ELF, no product rebuild', 'bind': 'OFF',
         'samples_per_target_per_case': 5, 'order': 'alternating ext4/owner by pair',
         'read_write_total_bytes': 67108864, 'block_bytes': 1048576, 'concurrency': 1,
         'read_write_ratio_gate': .9, 'noise_tolerance': 0, 'reruns': 0,
         'cache': 'buffered after preparation, no cold/hot qualification',
         'delete': '100x4KiB files, report only; no new ratio gate',
         'per_action_timeout_seconds': 120, 'external_whole_run_timeout_seconds': 300})
    try:
        preflight()
    except Exception as exc:
        save(OUT / 'blocked.json', {'status': 'BLOCKED', 'stage': 'preflight', 'error': str(exc)})
        raise
    results = []
    for name, action in [('seq-read', lambda: pair_io('seq-read')),
                         ('seq-write', lambda: pair_io('seq-write')), ('delete', pair_delete)]:
        try:
            result = action()
        except Exception as exc:
            result = {'status': 'FAIL', 'case': name, 'error': str(exc)}
            save(OUT / f'{name}-failure.json', result)
        results.append(result)
        print(json.dumps({k: v for k, v in result.items() if k != 'samples'}), flush=True)
    save(OUT / 'summary.json', results)
    before = json.loads((OUT / 'preflight.json').read_text())
    for name, identity in before['identities'].items():
        assert int((ROOT / f'run/{name}.pid').read_text()) == identity['pid']
        assert digest(f'/proc/{identity["pid"]}/exe') == identity['elf_sha256']
    after_mount = json.loads(cmd(['findmnt', '-J', '-T', str(OWNER)]))['filesystems'][0]
    assert after_mount == before['owner_mount']
    failed = any(x.get('status') == 'FAIL' or x.get('performance') == 'FAIL' for x in results)
    save(OUT / 'run-status.json', {'status': 'FAIL' if failed else 'PASS',
                                 'identity_after': 'UNCHANGED', 'case_results': results})
    return 3 if failed else 0

if __name__ == '__main__':
    raise SystemExit(main())
