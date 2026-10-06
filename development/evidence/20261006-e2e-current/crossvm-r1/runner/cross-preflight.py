#!/usr/bin/env python3
"""Linux cross-VM preflight; no source, service or environment mutation."""
import hashlib
import json
import os
import pathlib
import platform
import shutil
import socket
import subprocess
import sys
import tomllib

r = pathlib.Path('/var/tmp/afs-e2e-cross-20261006-r1')
node, address, peer = sys.argv[1:]
prefix = pathlib.Path('/var/tmp/afs-e2e-20261006-r1/prefix') if node == 'node-a' else r / 'prefix'
expected = {'afs-meta': '38f0e76a5b4c4afc3efdde3ee7bfa96b0e7e01a0403d556343ec385a86b20d45',
            'afs-node': '04b68193d7cdd04dea8c861f07e47336be05281de11a91e9ea8f3890123d1900'}
result = {'node': node, 'address': address, 'peer': peer, 'kernel': platform.release()}
try:
    assert platform.system() == 'Linux' and platform.machine() == 'aarch64' and os.geteuid() == 0
    result['dependencies'] = {n: shutil.which(n) for n in ('bash', 'python3', 'openssl', 'timeout', 'findmnt', 'fusermount3')}
    assert all(result['dependencies'].values())
    assert pathlib.Path('/dev/fuse').is_char_device()
    observed = {}
    for name, sha in expected.items():
        with (prefix / 'bin' / name).open('rb') as f:
            observed[name] = hashlib.file_digest(f, 'sha256').hexdigest()
        assert observed[name] == sha
        libs = subprocess.check_output(['ldd', str(prefix / 'bin' / name)], text=True)
        assert 'not found' not in libs
    result['binary_sha256'] = observed
    for script in ('afs-processctl', 'afs-selfcheck'):
        assert os.access(prefix / 'bin' / script, os.X_OK)
    with socket.create_connection((peer, 22), 3) as stream:
        result['peer_tcp'] = stream.recv(120).decode().strip()
    for port in ([20400, 20401, 20500, 20501] if node == 'node-a' else [20500, 20501]):
        with socket.socket() as stream:
            stream.bind((address, port))
    result['ports'] = 'AVAILABLE'
    config = tomllib.loads((r / 'etc/node.toml').read_text())
    assert config['id'] == node and config['meta_endpoint'] == 'https://192.168.109.3:20400'
    assert config['advertise_endpoint'] == f'https://{address}:20500'
    assert config['data_mode'] == 'grpc' and config['grpc_listen'] == '0.0.0.0:20500'
    assert config['dfs_sync_required_copies'] == 2 and config['dfs_min_distinct_nodes'] == 2
    assert set(config['trusted_node_certs']) == {'node-a', 'node-b'}
    for key in ('data_dir', 'uds_path', 'dfs_mount', 'ownerfs_mount'):
        assert pathlib.Path(config[key]).is_relative_to(r)
    certs = {}
    for name in ('meta', 'node-a', 'node-b'):
        cert = r / f'etc/tls/{name}.pem'
        subprocess.run(['openssl', 'verify', '-CAfile', str(r / 'etc/tls/ca.pem'), str(cert)],
                       text=True, capture_output=True, check=True)
        certs[name] = subprocess.check_output(['openssl', 'x509', '-in', str(cert), '-noout',
                                             '-subject', '-ext', 'subjectAltName'], text=True)
    assert address in certs[node] and 'afs-meta' in certs['meta']
    result['certificate_public_identity'] = certs
    capacity = os.statvfs(r / 'state')
    result['free_bytes'] = capacity.f_bavail * capacity.f_frsize
    assert result['free_bytes'] >= 2 * 1024 ** 3
    filesystem = json.loads(subprocess.check_output(['findmnt', '-J', '-T', str(r / 'state')], text=True))
    assert filesystem['filesystems'][0]['fstype'] == 'ext4'
    result['filesystem'] = filesystem
    assert not (r / 'mount/ownerfs').is_mount() and not (r / 'mount/dfs').is_mount()
    result['status'] = 'PASS'
except Exception as exc:
    result.update(status='BLOCKED', error=f'{type(exc).__name__}: {exc}')
    raise
finally:
    (r / 'results/preflight.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps(result))
