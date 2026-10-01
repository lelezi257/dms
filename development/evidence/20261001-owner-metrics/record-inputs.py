import hashlib
import json
import pathlib
import platform
import subprocess
import sys

assert platform.system() == 'Linux'
root = pathlib.Path(sys.argv[1]).resolve()
folders = ('src', 'tests', 'common', 'client', 'third_party', 'examples')
paths = {path.relative_to(root) for folder in folders
         for path in (root / folder).rglob('*')
         if path.is_file() and (path.suffix in ('.rs', '.proto', '.c', '.h')
                                or path.name == 'Cargo.toml')}
paths.update(map(pathlib.Path, ('Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml',
                              'error-codes.toml', 'examples/meta.toml', 'examples/node.toml',
                              'third_party/fuser/deny.toml', 'third_party/fuser/rustfmt.toml')))
files = {str(path): hashlib.sha256((root / path).read_bytes()).hexdigest()
         for path in sorted(paths)}
print(json.dumps({'snapshot': str(root), 'platform': platform.platform(),
                  'file_count': len(files), 'files': files,
                  'rustc': subprocess.check_output(['rustc', '--version', '--verbose'], text=True)}, indent=2))
