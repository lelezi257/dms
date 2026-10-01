#!/usr/bin/env python3
"""One-time archival of an explicit inactive package-extraction whitelist."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import stat
import subprocess
import time

DATA = Path('/mnt/lima-afsadata')
ARCHIVE = Path('/var/lib/afs-acceptance/archives/20261001-extractions-v65')
SELECTED = (
    'pkgfix-20260930T080823Z/afs-0.1.0-dev-pkgfix-linux-aarch64',
    'pkgfix3-20260930T081330Z/extract',
    'pkgfix4-20260930T081442Z/extract',
    'pkgfix5-20260930T081647Z/extract',
    'pkgfix5-extract-check/out',
    'lifecycle-20260930T082529Z/extract',
    'deploy-driver-lzc-084424/work-root/extract',
    'deploy-driver-lzc-084424/work-root-2/extract',
)


def digest(path):
    h = hashlib.sha256()
    with path.open('rb') as f:
        for block in iter(lambda: f.read(1024 * 1024), b''):
            h.update(block)
    return h.hexdigest()


def manifest(root):
    if not root.is_dir() or root.is_symlink():
        raise RuntimeError(f'not an ordinary directory: {root}')
    device = root.stat().st_dev
    rows = []
    for p in [root, *sorted(root.rglob('*'))]:
        s = p.lstat()
        if s.st_dev != device:
            raise RuntimeError(f'cross-filesystem entry: {p}')
        if not (stat.S_ISREG(s.st_mode) or stat.S_ISDIR(s.st_mode) or stat.S_ISLNK(s.st_mode)):
            raise RuntimeError(f'special entry: {p}')
        if stat.S_ISREG(s.st_mode) and s.st_nlink != 1:
            raise RuntimeError(f'hardlinked file requires separate review: {p}')
        row = dict(path=str(p.relative_to(root)), mode=s.st_mode, uid=s.st_uid,
                   gid=s.st_gid, mtime_ns=s.st_mtime_ns)
        row['xattrs'] = {k: hashlib.sha256(os.getxattr(p, k, follow_symlinks=False)).hexdigest()
                         for k in sorted(os.listxattr(p, follow_symlinks=False))}
        if stat.S_ISREG(s.st_mode):
            row.update(size=s.st_size, sha256=digest(p))
        if stat.S_ISLNK(s.st_mode):
            row['target'] = os.readlink(p)
        rows.append(row)
    return rows


def inside(path, root):
    return path == str(root) or path.startswith(str(root) + '/')


def assert_unused(roots):
    conflicts = []
    for line in Path('/proc/self/mountinfo').read_text().splitlines():
        mount = line.split()[4].replace('\\040', ' ').replace('\\134', '\\')
        if any(inside(mount, root) for root in roots):
            conflicts.append({'mount': mount})
    for process in Path('/proc').iterdir():
        if not process.name.isdigit():
            continue
        paths = []
        try:
            for entry in ('exe', 'cwd', 'root'):
                try:
                    paths.append(os.readlink(process / entry))
                except FileNotFoundError:
                    pass
            for fd in (process / 'fd').iterdir():
                try:
                    paths.append(os.readlink(fd))
                except FileNotFoundError:
                    pass
            try:
                for line in (process / 'maps').read_text().splitlines():
                    fields = line.split(None, 5)
                    if len(fields) == 6:
                        paths.append(fields[5])
            except FileNotFoundError:
                pass
        except (FileNotFoundError, ProcessLookupError):
            continue
        # Permission/errors other than disappearing processes fail closed.
        hits = sorted({p for p in paths if any(inside(p, root) for root in roots)})
        if hits:
            conflicts.append({'pid': process.name, 'paths': hits})
    if conflicts:
        raise RuntimeError(json.dumps({'active_paths': conflicts}, sort_keys=True))


def archive_one(source, destination, check_unused=assert_unused):
    backup = source.with_name(source.name + '.archive-v65-source')
    if destination.exists() or destination.is_symlink() or backup.exists() or backup.is_symlink():
        raise RuntimeError('destination or backup already exists; inspect rather than overwrite')
    check_unused([source])
    before = manifest(source)
    destination.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run(['cp', '-a', '--', str(source), str(destination)], check=True)
    if manifest(destination) != before or manifest(source) != before:
        raise RuntimeError('copy mismatch; preserve both trees without changing original path')
    check_unused([source, destination])
    os.rename(source, backup)
    try:
        os.symlink(str(destination), source)
    except BaseException:
        os.rename(backup, source)
        raise
    if source.resolve() != destination or manifest(backup) != before or manifest(destination) != before:
        raise RuntimeError('swap mismatch; preserve backup and archive for recovery')
    check_unused([source, backup, destination])
    # Delete only the redundant, verified copy; archive and original-path link remain.
    shutil.rmtree(backup)
    return dict(source=str(source), archive=str(destination), entries=before,
                logical_bytes=sum(row.get('size', 0) for row in before))


def capacity(path):
    s = os.statvfs(path)
    return dict(path=str(path), device=path.stat().st_dev, available=s.f_bavail*s.f_frsize,
                total=s.f_blocks*s.f_frsize)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--apply', action='store_true')
    parser.add_argument('--evidence', type=Path, required=True)
    args = parser.parse_args()
    if os.geteuid() != 0 or not DATA.is_mount():
        raise RuntimeError('requires root in the intended Linux guest with mounted data volume')
    args.evidence.mkdir(parents=True, exist_ok=True)
    roots = [DATA / relative for relative in SELECTED]
    assert_unused(roots)
    rows = [dict(source=str(p), entries=manifest(p)) for p in roots]
    report = dict(observed_at=time.time(), source_sha256=digest(Path(__file__)),
                  mode='apply' if args.apply else 'audit', before=[capacity(DATA), capacity(Path('/'))],
                  whitelist=list(SELECTED), audit=rows, migrations=[])
    (args.evidence / 'before.json').write_text(json.dumps(report, indent=2)+'\n')
    if args.apply:
        needed = sum(row.get('size', 0) for tree in rows for row in tree['entries'])
        if ARCHIVE.exists() or ARCHIVE.is_symlink():
            raise RuntimeError('archive already exists; inspect partial result rather than overwrite')
        if capacity(Path('/'))['available'] < needed + 4*1024**3:
            raise RuntimeError('insufficient root reserve for an exact archive copy')
        if DATA.stat().st_dev == Path('/').stat().st_dev:
            raise RuntimeError('archival would not release the data volume')
        for relative in SELECTED:
            record = archive_one(DATA / relative, ARCHIVE / relative)
            report['migrations'].append(record)
            (args.evidence / 'progress.json').write_text(json.dumps(report, indent=2)+'\n')
            print(json.dumps({k:v for k,v in record.items() if k!='entries'}), flush=True)
        report['after'] = [capacity(DATA), capacity(Path('/'))]
        report['data_reserve_pass'] = report['after'][0]['available'] >= 4*1024**3
        if not report['data_reserve_pass']:
            raise RuntimeError('migration completed but required data reserve is still not met')
    (args.evidence / 'result.json').write_text(json.dumps(report, indent=2)+'\n')
    print(json.dumps({'mode':report['mode'], 'trees':len(report['migrations']),
                      'before':report['before'], 'after':report.get('after')}), flush=True)


if __name__ == '__main__':
    main()
