#!/usr/bin/env python3
"""Current frozen DFS R3 counter dataset; reuse the historical stdio protocol.

This driver records content/timing. Replica and lifecycle evidence is collected
independently. It neither qualifies 3FS parity nor changes the old R2 results.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import sys

import dfs_manyread_small as sync

PRODUCT = '93169c8530beedff9e8e510dc9b9210d704f244f'
MAP = '9661a313fb8b433caa00d03df5be43f880ad97859fc0299cd6d397cdf55cda0f'
DATASET = 'counter-1m-v1'
RELATIVE = 'r3-current-counter'
PAYLOAD = 'payload64m.bin'
META_SHA = '4150942fd873b879ab6dc9034904116cf1670904a259d6a081f3b0c1fd1e7343'
NODE_SHA = '9478f3e89905b310689c0727ce0adf28fc11a25444c9634f82055a664069b41d'


def require(ok, reason):
    if not ok:
        raise ValueError(reason)


def expected_content():
    """Independent SHA oracle, including the sixteen unique physical chunks."""
    whole, chunks = hashlib.sha256(), []
    for chunk_index in range(16):
        chunk = hashlib.sha256()
        for index in range(chunk_index * 4, chunk_index * 4 + 4):
            block = index.to_bytes(8, 'little') + b'a' * (sync.BLOCK_BYTES - 8)
            whole.update(block)
            chunk.update(block)
        chunks.append(chunk.hexdigest())
    return {'sha256': whole.hexdigest(), 'chunk_sha256': chunks,
            'bytes': sync.DATA_BYTES, 'dataset': DATASET}


def validate_identity(value):
    require(isinstance(value, dict), 'identity must be an object')
    for key, expected in (('product_source_commit', PRODUCT), ('compiler_input_map', MAP),
                          ('afs_meta_sha256', META_SHA), ('afs_node_sha256', NODE_SHA)):
        require(value.get(key) == expected, f'current identity mismatch: {key}')
    digest = value.get('io_sha256')
    require(isinstance(digest, str) and len(digest) == 64 and
            all(c in '0123456789abcdef' for c in digest), 'invalid probe digest')
    return value


def validate_manifest(value, identity):
    require(value.get('status') == 'DATA_RECORDED', 'writer was not confirmed')
    require(value.get('identity') == identity, 'writer identity mismatch')
    require(value.get('relative_dir') == RELATIVE, 'unexpected payload directory')
    require(value.get('content') == expected_content(), 'counter content/shape mismatch')
    return value


def validate_sample(value, operation):
    sync.validate_io_result(value, 'seq-' + operation, 'fdatasync' if operation == 'write' else 'close')
    require(value.get('dataset') == DATASET, 'uniform or unknown dataset cannot prove this case')
    require(type(value.get('wall_ns')) is int and value['wall_ns'] > 0, 'invalid C timer')


def prepare(args):
    sync.verify_linux_aarch64_root()
    output = sync.require_absolute_path(args.output, 'output')
    sync.validate_output_path(output)
    output.mkdir(mode=0o700)
    root = sync.require_absolute_path(args.dfs_root, 'dfs-root')
    sync.require_existing_directory(root, 'dfs-root')
    mount = sync.find_mount(root)
    sync.require_dfs_mount(mount, root)
    identity = validate_identity(sync.read_json(Path(args.identity)))
    tool = sync.require_absolute_path(args.io_tool, 'io-tool')
    require(tool.is_file() and not tool.is_symlink(), 'probe must be an actual file')
    require(sync.sha256_file(tool) == identity['io_sha256'], 'probe identity mismatch')
    require(not (root / RELATIVE).is_symlink(), 'payload directory is a symlink')
    payload = root / RELATIVE / PAYLOAD
    require(not payload.is_symlink(), 'payload is a symlink')
    sync.write_json(output / 'identity.json', {'identity': identity, 'mount': mount,
                                              'tool': sync.stat_identity(tool)})
    return output, root, payload, tool, identity


def run_sample(tool, payload, operation, output, index, timeout):
    argv = [str(tool), operation, str(payload)]
    result = {'argv': argv, 'round': index, 'measured': operation == 'read' and index > 0,
              'status': 'FAIL'}
    try:
        done = subprocess.run(argv, capture_output=True, text=True, timeout=timeout)
        result.update(rc=done.returncode, stdout=done.stdout, stderr=done.stderr)
        require(done.returncode == 0, f'probe exit {done.returncode}')
        parsed = json.loads(done.stdout)
        result['result'] = parsed
        validate_sample(parsed, operation)
        result['status'] = 'PASS'
    except Exception as error:
        result['error'] = repr(error)
    sync.write_json(output / f'{operation}-{index:02d}.json', result)
    return result


def verify_content(payload, expected):
    require(payload.is_file() and not payload.is_symlink(), 'payload not an ordinary file')
    size = payload.stat().st_size
    digest = sync.sha256_file(payload)
    eof = sync.verify_eof(payload)
    require(size == sync.DATA_BYTES and digest == expected['sha256'] and eof['status'] == 'PASS',
            'full SHA/size/EOF mismatch')
    return {'bytes': size, 'sha256': digest, 'eof': eof, 'status': 'PASS'}


def writer(args):
    result, output = {'role': 'writer', 'status': 'BLOCKED'}, None
    try:
        output, root, payload, tool, identity = prepare(args)
        (root / RELATIVE).mkdir(mode=0o755)
        sample = run_sample(tool, payload, 'write', output, 0, args.round_timeout)
        result['sample'] = sample
        require(sample['status'] == 'PASS', 'writer probe failed')
        sync.fsync_directory(payload.parent)
        sync.fsync_directory(root)
        content = expected_content()
        result.update(identity=identity, relative_dir=RELATIVE, content=content,
                      content_verify=verify_content(payload, content), status='DATA_RECORDED')
        sync.write_json(output / 'writer-confirmed.json', result)
    except Exception as error:
        result['error'] = repr(error)
    finally:
        if output is not None:
            sync.write_json(output / 'summary.json', result)
    return result


def reader(args):
    reader_id, session = args.reader_id, args.session_token
    result = {'role': 'reader', 'reader_id': reader_id, 'session_token': session, 'status': 'BLOCKED'}
    output, rounds = None, []
    sync.emit_json_line(sys.stdout, dict(event='HELLO', reader_id=reader_id,
                                        session_token=session, rounds=5, dataset=DATASET))
    control = sync.JsonLineReader(sys.stdin)
    try:
        sync.read_ack_event(control, 0, args.round_timeout, reader_id=reader_id, session_token=session)
        output, _, payload, tool, identity = prepare(args)
        manifest = validate_manifest(sync.read_json(Path(args.manifest)), identity)
        result['pre_content'] = verify_content(payload, manifest['content'])
        warmup = run_sample(tool, payload, 'read', output, 0, args.round_timeout)
        rounds.append(warmup)
        require(warmup['status'] == 'PASS', 'warmup failed')
        for index in range(1, 6):
            sync.emit_json_line(sys.stdout, dict(event='READY', reader_id=reader_id,
                                                session_token=session, round=index,
                                                payload_sha256=manifest['content']['sha256']))
            start = sync.read_start_event(control, index, args.round_timeout,
                                           reader_id=reader_id, session_token=session)
            sample = run_sample(tool, payload, 'read', output, index, args.round_timeout)
            sample['round_token'] = start['round_token']
            rounds.append(sample)
            sync.emit_json_line(sys.stdout, dict(event='DONE', reader_id=reader_id,
                                                session_token=session, round=index,
                                                round_token=start['round_token'], status=sample['status'],
                                                rc=sample.get('rc'), result=sample.get('result')))
            require(sample['status'] == 'PASS', f'read round {index} failed')
            sync.read_ack_event(control, index, args.round_timeout, reader_id=reader_id,
                                session_token=session, round_token=start['round_token'])
        result.update(final_content=verify_content(payload, manifest['content']),
                      identity=identity, status='DATA_RECORDED')
    except Exception as error:
        result['error'] = repr(error)
    finally:
        result['read_rounds'] = rounds
        if output is not None:
            sync.write_json(output / 'summary.json', result)
        sync.emit_json_line(sys.stdout, dict(event='FINAL', reader_id=reader_id,
                                            session_token=session, status=result['status'],
                                            error=result.get('error')))
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='role', required=True)
    for role in ('writer', 'reader'):
        item = sub.add_parser(role)
        for name in ('dfs-root', 'io-tool', 'identity', 'output'):
            item.add_argument('--' + name, required=True)
        item.add_argument('--round-timeout', type=float, default=60)
        if role == 'reader':
            item.add_argument('--manifest', required=True)
            item.add_argument('--reader-id', choices=('B', 'C'), required=True)
            item.add_argument('--session-token', required=True)
    item = sub.add_parser('coordinator')
    item.add_argument('--output', required=True)
    item.add_argument('--session-token', required=True)
    item.add_argument('--round-timeout', type=float, default=60)
    item.set_defaults(readers=['B', 'C'])
    args = parser.parse_args()
    result = writer(args) if args.role == 'writer' else reader(args) if args.role == 'reader' else sync.coordinator(args)
    # Coordinator protocol validates the common shape; readers separately reject
    # uniform content. Keep that boundary explicit instead of changing old code.
    print(json.dumps(result, indent=2), file=sys.stdout if args.role == 'writer' else sys.stderr)
    return 0 if result['status'] == 'DATA_RECORDED' else 1


if __name__ == '__main__':
    raise SystemExit(main())
