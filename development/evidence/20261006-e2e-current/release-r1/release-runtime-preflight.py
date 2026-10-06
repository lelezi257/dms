#!/usr/bin/env python3
"""Admit the release runtime before starting workloads; no environment repair."""
import hashlib
import json
import os
import pathlib
import platform
import shutil
import socket
import stat
import subprocess
import sys
import tomllib

if not __debug__:
    raise RuntimeError('validation requires normal Python')
r = pathlib.Path('/var/tmp/afs-e2e-release-20261006-r1')
old = pathlib.Path('/var/tmp/afs-e2e-20261006-r1')
expected = {'afs-meta': '9cfdf8f01def1a45383baea834598a0a6ff572f3a2a0b4aa9af6a1523963bea9',
            'afs-node': '926ada2800093b5b7682ed37a978549eaf2704a25b5ebfc7cc346244cbd689a7',
            'posix-workload': 'a2a56e4cddecf4129f32d449562e7455a050fdd0d018f19f716ced7bd06d6054'}
checks = {}
def check(name, passed, value):
    checks[name] = {'status': 'PASS' if passed else 'BLOCKED', 'value': value}
    if not passed:
        raise RuntimeError(name)
def command(args):
    result = subprocess.run(args, text=True, capture_output=True, timeout=15)
    if result.returncode:
        raise RuntimeError({'command': args, 'exit': result.returncode, 'stderr': result.stderr})
    return result.stdout
phase = sys.argv[1]
try:
    check('linux-root', platform.system() == 'Linux' and platform.machine() == 'aarch64'
          and os.geteuid() == 0, [platform.system(), platform.machine(), os.geteuid()])
    if phase == 'before':
        needed = ('python3', 'bash', 'timeout', 'openssl', 'findmnt', 'fusermount3',
                  'ldd', 'git', 'prove', 'df', 'perl', 'sh')
        deps = {name: shutil.which(name) for name in needed}
        check('dependencies', all(deps.values()), deps)
        check('fuse-device', stat.S_ISCHR(os.stat('/dev/fuse').st_mode), '/dev/fuse')
        for name, sha in expected.items():
            path = r / ('tools' if name == 'posix-workload' else 'prefix/bin') / name
            value = hashlib.sha256(path.read_bytes()).hexdigest()
            check(name+'-sha256', value == sha, value)
            libraries = command(['ldd', str(path)])
            check(name+'-libraries', 'not found' not in libraries, libraries)
        for port in (21400, 21401, 21500, 21501):
            with socket.socket() as sock:
                sock.bind(('127.0.0.1', port))
        check('ports-free', True, [21400, 21401, 21500, 21501])
        for name in ('meta', 'node'):
            path = r / f'etc/{name}.toml'
            config = tomllib.loads(path.read_text())
            check(name+'-dedicated-path', config['data_dir'].startswith(str(r)), config['data_dir'])
            check(name+'-grpc', name == 'meta' or config['data_mode'] == 'grpc', config.get('data_mode'))
            if name == 'meta':
                check('local-file-meta', config['meta_store'] == 'local-file', config['meta_store'])
            cert = config['tls_identity_certificate']
            check(name+'-certificate', ': OK' in command(['openssl', 'verify', '-CAfile',
                  config['tls_ca_certificate'], cert]), cert)
            (r / f'results/{name}.toml').write_bytes(path.read_bytes())
        fs = json.loads(command(['findmnt', '-J', '-T', str(r)]))
        check('ext4', fs['filesystems'][0]['fstype'] == 'ext4', fs)
        check('capacity', shutil.disk_usage(r).free >= 3*2**30,
              {'free_bytes': shutil.disk_usage(r).free, 'minimum_bytes': 3*2**30})
        suite = old / 'suites/pjdfstest'
        revision = command(['git', '-c', 'safe.directory='+str(suite), '-C', str(suite),
                            'rev-parse', 'HEAD']).strip()
        check('pjdfstest-pin', revision == 'd25636a227606f8960e5179741d8f4ad7030ef41', revision)
        suite_sha = hashlib.sha256((suite/'pjdfstest').read_bytes()).hexdigest()
        check('pjdfstest-binary', suite_sha == '83f27ae21a4de5c2dabc447238c83f21c1a17558e61762ec52fe7ffbcf61f780', suite_sha)
    elif phase == 'mounted':
        for name in ('meta', 'node'):
            pid = int((r / f'run/{name}.pid').read_text())
            elf_sha = hashlib.sha256(pathlib.Path(f'/proc/{pid}/exe').read_bytes()).hexdigest()
            starttick = int(pathlib.Path(f'/proc/{pid}/stat').read_text().rsplit(')',1)[1].split()[19])
            check(name+'-running-identity', elf_sha == expected['afs-'+name],
                  {'pid': pid, 'elf_sha256': elf_sha, 'starttick': starttick})
        mount = json.loads(command(['findmnt', '-J', '--mountpoint', str(r/'mount/ownerfs')]))
        check('owner-exact-mount', mount['filesystems'][0]['source'] == 'afs-ownerfs', mount)
    else:
        raise RuntimeError('unknown phase')
    proof = {'status': 'PASS', 'source_commit': 'e925c5bcf0408851ebfa08a59df29953374da9e9',
             'profile': 'release', 'phase': phase, 'checks': checks}
except Exception as exc:
    proof = {'status': 'BLOCKED', 'phase': phase, 'checks': checks, 'error': str(exc)}
(r / f'results/runtime-preflight-{phase}.json').write_text(json.dumps(proof, indent=2)+'\n')
print(json.dumps({'status': proof['status'], 'phase': phase, 'checks': len(checks), 'error': proof.get('error')}))
raise SystemExit(0 if proof['status'] == 'PASS' else 1)
