#!/usr/bin/env python3
"""Linux-only admission and direct closure observations for one orderly restart."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import platform
import re
import shutil
import socket
import stat
import struct
import subprocess
import tarfile
import tempfile
from types import SimpleNamespace

ROOT = Path('/opt/afs-managed-orderly-recovery-20261007-r1')
GUEST = Path('/var/tmp/afs-managed-orderly-recovery-20261007-r1')
CEILING, FLOOR = 256 * 2**20, 2**30
RAM_MARGIN = 512 * 2**20
PORTS = (24400, 24401, 24500, 24501)
TOOLS = ('development/acceptance/native-workspace-linux.py',
         'development/acceptance/installed-smoke-linux.py',
         'development/acceptance/probes/native_source_rejection.py',
         'development/acceptance/probes/native_orderly_recovery.py',
         'development/acceptance/probes/test_native_orderly_recovery.py',
         'scripts/ownerfs/native-workspace-control.py')
ROOTFS = ('afs-workspace-probe', 'bin/busybox', 'bin/sh',
          'lib/ld-linux-aarch64.so.1', 'lib/aarch64-linux-gnu/libgcc_s.so.1',
          'lib/aarch64-linux-gnu/libc.so.6')
TARGETS = ('afs-', 'runc', 'mfsmaster', 'mfschunkserver', 'mfsmount',
           'mgmtd', 'metad', 'storaged', 'fuse_main', 'fdbserver')


def require(ok, reason):
    if not ok:
        raise ValueError(reason)


def sha(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def read_json(path):
    return json.loads(Path(path).read_text())


def publish(path, value):
    """Publish complete evidence once; never replace an earlier receipt."""
    path = Path(path)
    with tempfile.NamedTemporaryFile(mode='w', dir=path.parent, delete=False) as stream:
        temporary = Path(stream.name)
        try:
            json.dump(value, stream, indent=2)
            stream.write('\n')
            stream.flush()
            os.fsync(stream.fileno())
            os.link(temporary, path)
        finally:
            temporary.unlink()


def allocations(roots):
    seen, allocated, apparent = set(), 0, 0
    for root in roots:
        if not os.path.lexists(root):
            continue
        for path in [root, *Path(root).rglob('*')]:
            st = path.lstat()
            key = st.st_dev, st.st_ino
            if key not in seen:
                seen.add(key)
                allocated += st.st_blocks * 512
                apparent += st.st_size
    return dict(allocated_bytes=allocated, apparent_bytes=apparent, distinct_inodes=len(seen))


def verify_budget(allocated, free, initial=False):
    require(type(allocated) is int and 0 <= allocated <= CEILING, 'allocated ceiling exceeded')
    require(type(free) is int and free >= FLOOR + (CEILING - allocated if initial else 0),
            'free capacity floor/reservation missing')


def verify_ram(available, cgroups):
    require(type(available) is int and available >= RAM_MARGIN, 'MemAvailable below 512MiB')
    for item in cgroups:
        maximum, current = item['maximum'], item['current']
        require(type(current) is int and current >= 0, 'invalid cgroup current memory')
        require(maximum == 'max' or (type(maximum) is int and maximum - current >= RAM_MARGIN),
                'finite cgroup memory headroom below 512MiB')


def ram_observation():
    meminfo = Path('/proc/meminfo').read_text()
    available = int(re.search(r'^MemAvailable:\s+(\d+) kB$', meminfo, re.M)[1]) * 1024
    membership = Path('/proc/self/cgroup').read_text()
    require(membership.startswith('0::/'), 'requires observable cgroup-v2 memory limits')
    relative = membership.strip().split('::', 1)[1].lstrip('/')
    base = Path('/sys/fs/cgroup')
    current, values = base / relative, []
    for directory in [current, *current.parents]:
        if directory == base.parent:
            break
        maximum = directory / 'memory.max'
        if maximum.is_file():
            value = maximum.read_text().strip()
            values.append({'path': str(directory), 'maximum': 'max' if value == 'max' else int(value),
                           'current': int((directory / 'memory.current').read_text())})
    require(bool(values), 'no observable cgroup memory limit')
    verify_ram(available, values)
    return {'meminfo': meminfo, 'cgroup': membership, 'available_bytes': available,
            'ancestor_limits': values, 'minimum_margin_bytes': RAM_MARGIN}


def package_inputs(path, expected):
    require(sha(path) == expected['package_sha256'], 'wrong package SHA')
    with tarfile.open(path, 'r:gz') as archive:
        members = archive.getmembers()
        require(all(not Path(m.name).is_absolute() and '..' not in Path(m.name).parts
                    and (m.isfile() or m.isdir()) for m in members), 'unsafe package member')
        names = [m.name for m in members]
        require(len(names) == len(set(names)), 'duplicate package member')
        require(len({Path(name).parts[0] for name in names}) == 1, 'multiple package roots')
        manifests = [m for m in members if m.name.endswith('/manifest.json')]
        require(len(manifests) == 1, 'missing unique manifest')
        manifest = json.load(archive.extractfile(manifests[0]))
        require(manifest.get('source_commit') == expected['source_commit'], 'wrong package source')
        prefix = manifests[0].name.removesuffix('manifest.json')
        blobs = {}
        for role in ('meta', 'node'):
            member = archive.getmember(prefix + 'bin/afs-' + role)
            require(member.isfile(), 'nonregular ELF')
            blob = archive.extractfile(member).read()
            require(hashlib.sha256(blob).hexdigest() == expected['afs_' + role + '_sha256'],
                    'wrong package ELF: ' + role)
            require(len(blob) >= 20 and blob[:6] == b'\x7fELF\x02\x01'
                    and struct.unpack('<H', blob[18:20])[0] == 183, 'not Linux ARM64 ELF')
            blobs[role] = blob
        return manifest, blobs


def ancestors():
    result, pid = set(), os.getpid()
    while pid > 1 and pid not in result:
        result.add(pid)
        try:
            pid = int(Path(f'/proc/{pid}/stat').read_text().rsplit(')', 1)[1].split()[1])
        except FileNotFoundError:
            break
    return result


def process(pid):
    root = Path('/proc') / str(pid)
    return {'pid': pid, 'exe': os.readlink(root / 'exe'),
            'starttick': int((root / 'stat').read_text().rsplit(')', 1)[1].split()[19]),
            'argv': (root / 'cmdline').read_bytes().replace(b'\0', b' ').decode(errors='replace'),
            'exe_sha256': sha(root / 'exe')}


def inventory(owned=None):
    found, skip = [], ancestors()
    for directory in Path('/proc').iterdir():
        if not directory.name.isdigit() or int(directory.name) in skip:
            continue
        try:
            exe = os.readlink(directory / 'exe')
            argv = (directory / 'cmdline').read_bytes().replace(b'\0', b' ').decode(errors='replace')
        except (FileNotFoundError, ProcessLookupError):
            continue  # Discovery of an unrelated short-lived process is not a blocker.
        selected = str(owned) in exe or str(owned) in argv if owned else any(
            target in exe or target in argv for target in TARGETS)
        if selected:
            found.append(process(int(directory.name)))  # An identified service must remain observable.
    return {'boot_id': Path('/proc/sys/kernel/random/boot_id').read_text().strip(),
            'processes': found, 'mountinfo': Path('/proc/self/mountinfo').read_text().splitlines()}


def verify_protected(before, after):
    require(before['boot_id'] == after['boot_id'], 'VM boot identity changed')
    actual = {p['pid']: p for p in after['processes']}
    require(all(actual.get(p['pid']) == p for p in before['processes']),
            'protected process incarnation changed')
    require(set(before['mountinfo']) <= set(after['mountinfo']), 'protected mount changed')


def verify_driver(value, expected, driver_sha):
    require(value.get('status') == 'PASS' and value.get('cleanup') == 'PASS'
            and value.get('source_commit') == expected['source_commit']
            and value.get('driver_sha256') == driver_sha
            and value.get('orderly_recovery_selected') is True
            and all(value.get(k) is False for k in ('basic_payload_selected',
                    'source_rejection_selected', 'control_capacity_selected'))
            and value.get('semantic_groups') == [], 'wrong driver result/mode/identity')


def verify_waits(root, oracle, saved_final, probe, phase1_archive,
                 exists=lambda pid: Path('/proc').joinpath(str(pid)).exists()):
    expected = [(oracle['phase1']['process'], oracle['phase1']['actual_waits']),
                (oracle['phase2']['process'], saved_final)]
    observed, pids = [], set()
    for phase, (services, saved) in enumerate(expected):
        directories = set()
        receipt_root = phase1_archive if phase == 0 else root / 'run'
        require(set(saved) == {'meta', 'node'}, 'missing actual wait')
        for role in ('meta', 'node'):
            identity, entry = services[role], saved[role]
            path = Path(entry['path'])
            require(path.parent == root / 'run' and path.name.startswith(role + '.lifecycle.'),
                    'foreign lifecycle path')
            actual_path = receipt_root / path.name
            child, receipt, ready = [probe.fields(actual_path / name) for name in ('child', 'exit', 'ready')]
            require(entry == {'path': str(path), 'child': child, 'exit': receipt, 'ready': ready,
                              'both_gone': True}, 'saved receipt differs from actual files')
            for key in ('pid', 'supervisor_pid'):
                require(re.fullmatch(r'[1-9][0-9]*', child.get(key, '')) is not None, 'invalid receipt PID')
            ids = {int(child['pid']), int(child['supervisor_pid'])}
            require(len(ids) == 2 and not ids & pids, 'reused child/supervisor identity')
            gone = all(not exists(pid) for pid in ids)
            probe.verify_receipt(receipt, child, ready, identity, gone)
            require(child['lifecycle'] == str(path), 'lifecycle receipt path mismatch')
            pids.update(ids)
            directories.add(actual_path)
            observed.append({**entry, 'observed_receipt_directory': str(actual_path)})
        require(set(receipt_root.glob('*.lifecycle.*')) == directories,
                'missing/extra lifecycle in phase receipt directory')
    require(len(pids) == 8, 'missing/extra service PID')
    return {'receipts': observed, 'eight_pids_gone': sorted(pids)}


class Checks:
    def __init__(self, args):
        self.args = args
        self.report = {'status': 'BLOCKED' if args.mode == 'admit' else 'FAIL',
                       'checker_sha256': sha(__file__), 'checks': {}, 'commands': []}

    def check(self, name, function):
        try:
            value = function()
            self.report['checks'][name] = {'status': 'PASS', 'actual': value}
            return value
        except Exception as exc:
            self.report['checks'][name] = {'status': self.report['status'], 'error': repr(exc)}
            raise

    def command(self, argv, allowed=(0,)):
        completed = subprocess.run(list(map(str, argv)), capture_output=True, timeout=25)
        record = {'argv': list(map(str, argv)), 'rc': completed.returncode,
                  'stdout': completed.stdout.decode(errors='replace'),
                  'stderr': completed.stderr.decode(errors='replace')}
        self.report['commands'].append(record)
        require(completed.returncode in allowed, 'command failed: ' + str(argv))
        return record

    def identities(self):
        args = self.args
        expected = {k: getattr(args, k) for k in ('source_commit', 'package_sha256',
                       'afs_meta_sha256', 'afs_node_sha256', 'runtime_sha256')}
        require(re.fullmatch(r'[0-9a-f]{40}', expected['source_commit']) is not None, 'invalid commit')
        require(all(re.fullmatch(r'[0-9a-f]{64}', v) for k, v in expected.items() if k != 'source_commit'),
                'invalid SHA')
        require(args.root == ROOT and args.out == GUEST / 'results-r1'
                and args.tools_root == GUEST / 'tools', 'outside frozen fixture paths')
        require(sha(args.tool_inputs) == args.tool_inputs_sha256, 'wrong tool map SHA')
        tools = read_json(args.tool_inputs)['files']
        require(set(tools) == set(TOOLS), 'wrong r4 tool map')
        for relative, item in tools.items():
            path = args.tools_root / relative
            require(path.is_file() and not path.is_symlink() and sha(path) == item['sha256']
                    and path.stat().st_size == item['bytes'], 'wrong tool: ' + relative)
        require(sha(args.rootfs_inputs) == args.rootfs_inputs_sha256, 'wrong rootfs map SHA')
        require(sha(args.runtime) == expected['runtime_sha256'], 'wrong runtime SHA')
        manifest, blobs = package_inputs(args.package, expected)
        self.report['identity'] = {'expected': expected, 'manifest': manifest, 'tools': tools,
                                  'root': str(args.root), 'out': str(args.out),
                                  'tool_inputs_sha256': args.tool_inputs_sha256,
                                  'rootfs_inputs_sha256': args.rootfs_inputs_sha256}
        return blobs

    def admit(self):
        args = self.args
        require(args.report == args.snapshot, 'admission snapshot must be its published report')
        require(not os.path.lexists(args.root) and not os.path.lexists(args.out), 'fixture/result not fresh')
        dependencies = {name: shutil.which(name) for name in ('bash', 'python3', 'tar', 'ldd', 'openssl',
                        'findmnt', 'fusermount3', 'curl', 'flock', 'ss', 'sed', 'awk', 'mountpoint',
                        'setsid', 'nsenter', 'timeout', 'sha256sum', 'install', 'readlink', 'grep')}
        require(all(dependencies.values()), 'missing dependencies: ' + repr(dependencies))
        self.report['dependencies'] = dependencies
        require(stat.S_ISCHR(os.stat('/dev/fuse').st_mode), 'missing FUSE device')
        for directory in ('/opt', str(GUEST)):
            mounted = json.loads(self.command(['findmnt', '-J', '-T', directory])['stdout'])
            require(mounted['filesystems'][0]['fstype'] == 'ext4', 'not ext4: ' + directory)
        for port in PORTS:
            with socket.socket() as sock:
                sock.bind(('0.0.0.0', port))
        self.report['ports'] = list(PORTS)
        self.report['ram'] = self.check('ram-margin', ram_observation)
        self.report['protected_before'] = inventory()
        blobs = self.identities()
        # Qualification of these exact package ELFs, without borrowing an older installed binary.
        with tempfile.TemporaryDirectory(prefix='admission-elf-', dir=GUEST) as directory:
            for role, blob in blobs.items():
                path = Path(directory) / ('afs-' + role)
                path.write_bytes(blob)
                path.chmod(0o700)
                libraries = self.command(['ldd', path])
                require('not found' not in libraries['stdout'] + libraries['stderr'], 'missing ELF library')
        inputs = read_json(args.rootfs_inputs)
        require(set(inputs) == set(ROOTFS), 'wrong six rootfs inputs')
        for relative, item in inputs.items():
            path = args.template_rootfs / relative
            st = path.lstat()
            require(stat.S_ISREG(st.st_mode) and st.st_uid == 0 and stat.S_IMODE(st.st_mode) == 0o755
                    and sha(path) == item['sha256'], 'wrong template input: ' + relative)
        libraries = self.command(['ldd', args.template_rootfs / 'afs-workspace-probe'])
        require('not found' not in libraries['stdout'] + libraries['stderr'], 'missing probe library')
        self.command([args.runtime, '--version'])
        runtime_ldd = self.command(['ldd', args.runtime], allowed=(0, 1))
        require('not found' not in runtime_ldd['stdout'] + runtime_ldd['stderr'], 'missing runtime library')
        amount = allocations([args.root, GUEST])
        free = shutil.disk_usage('/opt').free
        verify_budget(amount['allocated_bytes'], free, initial=True)
        self.report['capacity_before'] = {**amount, 'free_bytes': free, 'ceiling': CEILING, 'floor': FLOOR}

    def postcheck(self):
        args = self.args
        before = read_json(args.snapshot)
        require(before.get('status') == 'PASS', 'admission not qualified')
        self.identities()
        admitted_checker = args.admitted_checker_sha256 or sha(__file__)
        require(re.fullmatch(r'[0-9a-f]{64}', admitted_checker) is not None
                and before['checker_sha256'] == admitted_checker, 'wrong admitted checker identity')
        self.report['admitted_checker_sha256'] = admitted_checker
        require(before['identity'] == self.report['identity'], 'admitted identities changed')
        spec = importlib.util.spec_from_file_location('orderly_probe', args.tools_root / TOOLS[3])
        probe = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(probe)
        identity = probe.load_identity()
        driver = read_json(args.out / 'result.json')
        verify_driver(driver, before['identity']['expected'], before['identity']['tools'][TOOLS[0]]['sha256'])
        oracle = read_json(args.out / 'orderly-recovery-result.json')
        require(oracle.get('status') == 'PASS' and oracle.get('inputs_unchanged') is True, 'recovery oracle failed')
        read_commands = []
        for phase in ('phase1', 'phase2'):
            entry = oracle[phase]
            identity.verify_unchanged(entry['identity'], entry['identity'])
            require(entry['process']['meta']['sha256'] == args.afs_meta_sha256
                    and entry['process']['node']['sha256'] == args.afs_node_sha256, 'observed wrong service ELF')
            for role in ('meta', 'node'):
                service = entry['process'][role]
                executable = args.root / 'prefix/bin' / ('afs-' + role)
                st = executable.stat()
                require(type(service['pid']) is int and service['pid'] > 0
                        and type(service['starttick']) is int and service['starttick'] > 0
                        and service['boot_id'] == before['protected_before']['boot_id']
                        and service['installed'] == {'path': str(executable), 'device': st.st_dev, 'inode': st.st_ino}
                        and sha(executable) == getattr(args, 'afs_' + role + '_sha256'),
                        'service installed/incarnation identity mismatch')
            for value in entry['content'].values():
                probe.verify_content(value)
            require(set(entry['content']) == {'container', 'fuse'}, 'missing content evidence')
            read = read_json(args.out / ('orderly-' + phase + '-read-content.json'))
            command = Path(read['command'])
            read_commands.append(command)
            require(command.parent == args.root / 'control' and command.name.startswith('command-')
                    and command.name.endswith('.command.json')
                    and sha(command) == read['command_sha256'], 'wrong actual read command')
            command_value = read_json(command)
            require(command_value.get('argv', [])[-3:] == ['/bin/sh', '-ec', probe.read_shell()],
                    'actual command is not complete container read')
            exit_path = command.with_name(command.name.replace('.command.json', '.exit.json'))
            exit_value = read_json(exit_path)
            require(exit_value == {'code': 0, 'reason': None, 'success': True}
                    and read['exit'] == exit_value, 'actual read exit failed')
            actual_content = probe.parse_container_content(command.with_name(
                command.name.replace('.command.json', '.stdout')).read_text())
            require(read['content'] == actual_content == entry['content']['container'],
                    'actual read content differs from oracle')
            private = read_json(args.out / ('orderly-' + phase + '-private-root.json'))
            config = Path(private['runtime']['bundle']) / 'config.json'
            require(private['runtime'] == entry['identity']['runtime'] and sha(config) == private['config_sha256']
                    and read_json(config) == private['oci_config'], 'private OCI config changed')
            probe.verify_private_root(private['runtime'], private['oci_config'], args.root / 'rootfs')
            require(not Path('/proc/' + str(entry['identity']['process']['pid'])).exists(), 'container still exists')
        require(len(set(read_commands)) == 2, 'phase2 reused phase1 read command')
        probe.verify_restart(oracle['phase1']['process'], oracle['phase2']['process'],
                             oracle['phase1']['identity'], oracle['phase2']['identity'])
        self.report['actual_waits'] = verify_waits(args.root, oracle,
            read_json(args.out / 'orderly-final-actual-waits.json'), probe, args.out / 'phase1/run')
        probe.verify_inputs(read_json(args.out / 'orderly-inputs-before-first-start.json'),
                            probe.frozen_inputs(SimpleNamespace(root=args.root)))
        checks = read_json(args.out / 'checks.json')
        for key in ('orderly-phase1-stopped', 'orderly-phase1-idle', 'orderly-phase1-runtime-empty',
                    'stop', 'idle-after-stop', 'runtime-empty', 'mount-removed', 'controller-artifacts-removed'):
            require(checks.get(key, {}).get('status') == 'PASS', 'missing public closure: ' + key)
        commands = read_json(args.out / 'commands.json')
        for ident, field, expected in (('orderly-phase1-stop', 'status', 'Stopped'),
                                       ('orderly-phase1-idle', 'state', 'Idle'),
                                       ('stop-first', 'status', 'Stopped'), ('after-stop', 'state', 'Idle')):
            actual = [record for record in commands if '--id' in record['argv']
                      and record['argv'][record['argv'].index('--id') + 1] == ident]
            require(len(actual) == 1 and actual[0]['exit'] == 0 and actual[0]['timeout'] is False,
                    'missing actual public response: ' + ident)
            response = read_json(args.out / actual[0]['stdout'])['response']
            require(response.get(field) == expected and response.get('production_ready') is False,
                    'actual public closure response differs: ' + ident)
            self.report.setdefault('public_closure_responses', {})[ident] = response
        # Preserve the entire first controller command set at its original path, not a moved archive.
        retained = list((args.out / 'phase1/control').glob('command-*'))
        require(bool(retained), 'missing phase1 command archive')
        for path in retained:
            require(sha(path) == sha(args.root / 'control' / path.name)
                    == sha(args.out / 'control' / path.name), 'phase1 command was overwritten/moved')
        for relative in ('control/control.sock', 'control/controller.lock', 'run/meta.sock', 'run/node.sock'):
            require(not os.path.lexists(args.root / relative), 'owned socket/lock remains: ' + relative)
        runtime = self.command([args.runtime, '--root', args.root / 'control/runtime-state', 'list', '--format', 'json'])
        require(json.loads(runtime['stdout']) in (None, []), 'runtime not empty')
        after = inventory()
        verify_protected(before['protected_before'], after)
        self.report['protected_after'] = after
        owned = inventory(args.root)
        require(not owned['processes'] and not any(str(args.root) in row for row in owned['mountinfo']),
                'owned process/mount remains')
        amount, free = allocations([args.root, GUEST]), shutil.disk_usage('/opt').free
        verify_budget(amount['allocated_bytes'], free)
        self.report['capacity_after'] = {**amount, 'free_bytes': free, 'ceiling': CEILING, 'floor': FLOOR,
            'added_allocated_bytes': amount['allocated_bytes'] - before['capacity_before']['allocated_bytes']}
        self.report['scope'] = 'orderly restart; actual four wait0/eight PID absence; owned public Stop may KILL; not graceful drain'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('mode', choices=('admit', 'postcheck'))
    for name in ('root', 'out', 'tools-root', 'package', 'runtime', 'template-rootfs',
                 'rootfs-inputs', 'tool-inputs', 'snapshot', 'report'):
        parser.add_argument('--' + name, required=True, type=Path)
    for name in ('source-commit', 'package-sha256', 'afs-meta-sha256', 'afs-node-sha256',
                 'runtime-sha256', 'rootfs-inputs-sha256', 'tool-inputs-sha256'):
        parser.add_argument('--' + name, required=True)
    parser.add_argument('--admitted-checker-sha256', help='explicit original checker identity for a reviewed postcheck-only correction')
    args = parser.parse_args()
    checker = Checks(args)
    try:
        require(platform.system() == 'Linux' and platform.machine() == 'aarch64' and os.geteuid() == 0,
                'requires Linux ARM64 root')
        checker.check(args.mode, getattr(checker, args.mode))
        checker.report['status'] = 'PASS'
    except Exception as exc:
        checker.report['error'] = repr(exc)
    publish(args.report, checker.report)
    print(json.dumps(checker.report, indent=2))
    return 0 if checker.report['status'] == 'PASS' else 1


if __name__ == '__main__':
    raise SystemExit(main())
