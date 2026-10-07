#!/usr/bin/env python3
"""One real Node controller-startup rejection; not full workspace bind acceptance."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import socket
import stat
import subprocess
import time


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('source', 'inputs', 'build-proof', 'target', 'root', 'out'):
        parser.add_argument('--' + name, type=Path, required=True)
    args = parser.parse_args()
    args.out.mkdir(exist_ok=False)
    checks, commands = {}, []
    owned = {}

    def save(name, value):
        (args.out / name).write_text(json.dumps(value, indent=2) + '\n')

    def check(name, ok, value):
        checks[name] = {'pass': bool(ok), 'value': value}
        save('checks.json', checks)
        if not ok:
            raise RuntimeError(name)

    def command(argv):
        argv = [str(x) for x in argv]
        result = subprocess.run(argv, capture_output=True, text=True, timeout=30)
        record = {'argv': argv, 'returncode': result.returncode,
                  'stdout': result.stdout, 'stderr': result.stderr}
        commands.append(record)
        save('commands.json', commands)
        check('command-' + str(len(commands)), result.returncode == 0, record)
        return result.stdout

    def processes():
        found = {}
        for proc in Path('/proc').iterdir():
            if not proc.name.isdigit():
                continue
            try:
                executable = Path(os.readlink(proc / 'exe'))
                if executable.name in ('afs-node', 'afs-meta'):
                    found[proc.name] = {'exe': str(executable),
                        'starttick': (proc / 'stat').read_text().rsplit(')', 1)[1].split()[19]}
            except (FileNotFoundError, ProcessLookupError, PermissionError):
                pass
        return found

    status = 'BLOCKED'
    try:
        check('Linux-arm64-root', platform.system() == 'Linux'
              and platform.machine() == 'aarch64' and os.geteuid() == 0,
              [platform.system(), platform.machine(), os.geteuid()])
        check('fresh-owned-root', args.root.parent == Path('/opt')
              and args.root.name.startswith('afs-bind-node-startup-') and not args.root.exists(), str(args.root))
        deps = {n: shutil.which(n) for n in ('bash', 'openssl', 'findmnt', 'fusermount3', 'curl')}
        check('dependencies', all(deps.values()), deps)
        check('FUSE-device', stat.S_ISCHR(os.stat('/dev/fuse').st_mode), '/dev/fuse')
        check('ext4', json.loads(command(['findmnt', '-J', '-T', '/opt']))['filesystems'][0]['fstype'] == 'ext4', '/opt')
        check('capacity', shutil.disk_usage('/opt').free >= 2**30, shutil.disk_usage('/opt').free)
        for port in (24800, 24801, 24900, 24901):
            with socket.socket() as sock:
                sock.bind(('127.0.0.1', port))
        check('ports-free', True, [24800, 24801, 24900, 24901])
        frozen = json.loads(args.inputs.read_text())
        check('source-inputs', all(sha(args.source / p) == h for p, h in frozen['files'].items()), frozen['map_sha256'])
        proof = json.loads(args.build_proof.read_text())
        check('fresh-release-build', proof['status'] == 'PASS' and 'build' in proof['selected_gates']
              and proof['map_sha256'] == frozen['map_sha256'], proof)
        binaries = {name: args.target / 'release' / ('afs-' + name) for name in ('meta', 'node')}
        for name, binary in binaries.items():
            check(name + '-ELF', sha(binary) == proof['binaries']['afs-' + name]['sha256'], sha(binary))
            check(name + '-libraries', 'not found' not in command(['ldd', binary]), str(binary))
        before = {'mountinfo': Path('/proc/self/mountinfo').read_text().splitlines(), 'processes': processes()}
        save('before.json', before)
        save('tool-inputs.json', {str(p): sha(p) for p in (Path(__file__), args.source / 'scripts/deploy/afs-trial-config')})
        status = 'FAIL'
        args.root.mkdir(mode=0o755)
        (args.root / 'control').mkdir(mode=0o700)
        (args.root / 'rootfs').mkdir(mode=0o755)
        command(['bash', args.source / 'scripts/deploy/afs-trial-config', 'single', '--backend', 'memory',
                 '--config-dir', args.root / 'etc', '--state-dir', args.root / 'state',
                 '--run-dir', args.root / 'run', '--mount-root', args.root / 'mount',
                 '--meta-grpc-port', '24800', '--meta-rest-port', '24801',
                 '--node-grpc-port', '24900', '--node-rest-port', '24901'])
        (args.root / 'mount/ownerfs').mkdir(mode=0o755)
        for name in ('meta', 'node'):
            config = args.root / 'etc' / (name + '.toml')
            text = '\n'.join(line for line in config.read_text().replace('fs = "all"', 'fs = "ownerfs"').splitlines()
                             if not line.startswith('dfs_mount =')) + '\n'
            if name == 'node':
                text = 'experimental_native_workspace = true\n' + text + '\n[native_workspace]\n' + (
                    f'control_dir = "{args.root}/control"\nruntime = "{args.root}/intentionally-absent-runc"\n'
                    f'rootfs = "{args.root}/rootfs"\nworkload_uid = 501\nworkload_gid = 501\n')
            config.write_text(text)
            (args.out / (name + '.toml')).write_text(text)
        resolved = json.loads(command([binaries['node'], '--config', args.root / 'etc/node.toml', '--print-config']))
        save('node-print-config.json', resolved)
        check('configuration-valid-fault', resolved['experimental_native_workspace'] is True
              and not Path(resolved['native_workspace']['runtime']).exists(), resolved['native_workspace'])
        for name in ('meta', 'node'):
            argv = [str(binaries[name]), '--config', str(args.root / 'etc' / (name + '.toml'))]
            with (args.out / (name + '.stdout')).open('w') as stdout, (args.out / (name + '.stderr')).open('w') as stderr:
                proc = subprocess.Popen(argv, stdout=stdout, stderr=stderr)
            owned[name] = proc
            save(name + '-process.json', {'argv': argv, 'pid': proc.pid, 'exe_sha256': sha(binaries[name]),
                'starttick': Path(f'/proc/{proc.pid}/stat').read_text().rsplit(')', 1)[1].split()[19]})
            if name == 'meta':
                until = time.monotonic() + 10
                while True:
                    response = subprocess.run(['curl', '-fsS', '--max-time', '1', 'http://127.0.0.1:24801/health'],
                                              capture_output=True, text=True)
                    if response.returncode == 0:
                        save('meta-health.json', {'returncode': 0, 'stdout': response.stdout})
                        break
                    if proc.poll() is not None or time.monotonic() >= until:
                        raise RuntimeError('Meta did not become healthy within admission deadline')
                    time.sleep(0.05)
            else:
                code = proc.wait(timeout=25)
                save('node-wait.json', {'pid': proc.pid, 'returncode': code, 'actual_wait': True})
                stdout = (args.out / 'node.stdout').read_text()
                stderr = (args.out / 'node.stderr').read_text()
                output = stdout + stderr
                terminal = re.sub(r'\x1b\[[0-9;]*m', '', stderr)
                check('Node-original-admission-failure', code == 1 and re.search(
                    r'^Error: Custom \{ kind: Other, error: "No such file or directory \(os error 2\)" \}$',
                    terminal, re.MULTILINE) is not None, terminal)
                check('no-ready', 'node.ready' not in output, 'no node.ready event')
                event_stream = next((s for s in (stdout, stderr)
                                     if 'services.stopped' in s and 'node.shutdown_failed' in s), '')
                event_lines = event_stream.splitlines()
                stopped = [i for i, line in enumerate(event_lines) if 'services.stopped' in line]
                failed = [i for i, line in enumerate(event_lines) if 'node.shutdown_failed' in line]
                check('observed-shutdown-chain', bool(stopped) and len(failed) == 1
                      and stopped[-1] < failed[0] and 'No such file or directory' in event_lines[failed[0]]
                      and 'os error 2' in event_lines[failed[0]], event_lines)
                uds = Path(resolved['uds_path'])
                check('Local-API-socket-removed', not uds.exists(), str(uds))
                check('controller-not-created', not (args.root / 'control/control.sock').exists()
                      and not (args.root / 'control/controller.lock').exists(), 'admission before controller creation')
        owned['meta'].terminate()
        code = owned['meta'].wait(timeout=10)
        save('meta-wait.json', {'pid': owned['meta'].pid, 'returncode': code, 'actual_wait': True})
        check('Meta-normal-exit', code == 0, code)
        after = {'mountinfo': Path('/proc/self/mountinfo').read_text().splitlines(), 'processes': processes()}
        save('after.json', after)
        check('mounts-and-protected-processes-unchanged', after == before, after)
        check('source-still-fixed', all(sha(args.source / p) == h for p, h in frozen['files'].items()), frozen['map_sha256'])
        allocated = sum(p.stat().st_blocks * 512 for p in args.root.rglob('*') if p.is_file())
        check('owned-budget-and-free-floor', allocated <= 256*2**20 and shutil.disk_usage('/opt').free >= 2**30,
              {'allocated_bytes': allocated, 'budget_bytes': 256*2**20, 'free_bytes': shutil.disk_usage('/opt').free})
        status = 'PASS'
    except Exception as error:
        save('failure.json', {'error': repr(error), 'owned_pids': {k: p.pid for k, p in owned.items()}})
        raise
    finally:
        for name, proc in owned.items():
            if proc.poll() is None:
                proc.terminate()
                try:
                    code = proc.wait(timeout=10)
                    save(name + '-failure-stop.json', {'pid': proc.pid, 'actual_wait': True, 'returncode': code})
                except subprocess.TimeoutExpired:
                    save(name + '-unresolved.json', {'pid': proc.pid, 'status': 'BLOCKED', 'reason': 'owned process did not stop; no forced cleanup'})
        save('result.json', {'status': status, 'scope': 'one real Node startup rejection, no runtime/bind creation',
                             'checks': len(checks), 'root': str(args.root)})


if __name__ == '__main__':
    main()
