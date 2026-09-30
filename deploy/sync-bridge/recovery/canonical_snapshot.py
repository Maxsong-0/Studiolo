#!/usr/bin/python3
"""停写窗口的 canonical/writer/board 三根恢复点；不替调用方停服务。"""
import argparse
from contextlib import closing
import hashlib
import json
import os
from pathlib import Path
import re
import sqlite3
import subprocess
import sys
import uuid

# 支持 python3 -I -B：辅助模块只从脚本所在的管理员固定目录载入。
import importlib.util
_helper = Path(__file__).resolve().with_name('snapshot_io.py')
_spec = importlib.util.spec_from_file_location('t132_snapshot_io', _helper)
_io = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(_io)
directory, inventory, new_json, require, read_regular = (
    _io.directory, _io.inventory, _io.new_json, _io.require, _io.read_regular)

BASE = Path('/home/maxsong/.local/share')
ROOTS = {'canonical': BASE / 'studiolo-canonical',
         'writer': BASE / 'studiolo-canonical-writer',
         'board': BASE / 'studiolo-board'}


def stopped():
    for name in ('studiolo-human.service', 'studiolo-agent.service'):
        raw = subprocess.check_output(['/usr/bin/systemctl', 'show', name,
              '--property=ActiveState,MainPID'], text=True)
        props = dict(s.split('=', 1) for s in raw.splitlines())
        require(props['ActiveState'] == 'inactive' and props['MainPID'] == '0')


def binding(roots):
    left = json.loads(read_regular(roots['canonical'] / '.workbench/broker-binding.json', 65536))
    right = json.loads(read_regular(roots['writer'] / 'broker-binding.json', 65536))
    require(left == right and left['format_version'] == 1)
    for key in ('vault_uuid', 'pair_id'):
        require(str(uuid.UUID(left[key])) == left[key])
    marker = json.loads(read_regular(roots['canonical'] / '.workbench/vault.json', 65536))
    require(marker == {'format_version': 1, 'vault_uuid': left['vault_uuid']})
    return {k: left[k] for k in ('vault_uuid', 'pair_id')}


def board_metadata(root, vault_uuid):
    # 干净退出必须已 checkpoint。不可用 immutable 模式忽略未回收的 WAL。
    require({p.name for p in root.iterdir()} == {'control.sqlite3'})
    with closing(sqlite3.connect((root / 'control.sqlite3').as_uri()
                                + '?mode=ro&immutable=1', uri=True)) as db:
        require(db.execute('PRAGMA quick_check').fetchall() == [('ok',)])
        require(not db.execute('PRAGMA foreign_key_check').fetchall())
        require(db.execute('SELECT vault_uuid FROM vault_binding WHERE singleton=1')
                .fetchall() == [(vault_uuid,)])
        versions = db.execute('SELECT version FROM schema_migrations ORDER BY version').fetchall()
        require(versions and versions[-1][0] == 3)
        counts = {name: db.execute('SELECT count(*) FROM ' + name).fetchone()[0]
                  for name in ('tasks', 'attempts', 'snapshots', 'projection_outbox')}
    return {'schema_versions': [v[0] for v in versions], 'counts': counts}


def new_destination(destination, sources):
    require(destination.is_absolute() and '..' not in destination.parts)
    directory(destination.parent)
    require(not destination.exists() and not destination.is_symlink())
    require(all(destination != p and not destination.is_relative_to(p)
                and not p.is_relative_to(destination) for p in sources))


def freeze(roots, destination, snapshot_id, barrier):
    require(re.fullmatch('[a-zA-Z0-9_-]{1,96}', snapshot_id) is not None)
    require(set(roots) == set(ROOTS))
    new_destination(destination, roots.values())
    paths = [directory(p) for p in roots.values()]
    require(len(paths) == 3 and len(set(paths)) == 3)
    require(all(a == b or not a.is_relative_to(b) for a in paths for b in paths))
    barrier()
    before = {name: inventory(path) for name, path in roots.items()}
    ids = binding(roots)
    board = board_metadata(roots['board'], ids['vault_uuid'])
    destination.mkdir(mode=0o700)
    try:
        copied = {name: inventory(path, destination / name) for name, path in roots.items()}
        barrier()
        require(before == copied == {name: inventory(path) for name, path in roots.items()})
        require(board_metadata(destination / 'board', ids['vault_uuid']) == board)
        document = {'schema_version': 1, 'snapshot_id': snapshot_id, **ids,
                    'board_metadata': board, 'files': copied}
        digest = new_json(destination / 'manifest.json', document)
        return {'status': 'frozen', 'snapshot_id': snapshot_id,
                'manifest_sha256': digest, 'files': sum(len(v) for v in copied.values())}
    except BaseException:
        (destination / 'INCOMPLETE').touch(mode=0o600, exist_ok=False)
        raise


def verify(source, expected):
    directory(source)
    require(re.fullmatch('[0-9a-f]{64}', expected) is not None)
    require({p.name for p in source.iterdir()} == {'manifest.json', *ROOTS})
    raw = read_regular(source / 'manifest.json', 32 * 1024 ** 2)
    require(len(raw) <= 32 * 1024 ** 2 and hashlib.sha256(raw).hexdigest() == expected)
    document = json.loads(raw)
    require(set(document) == {'schema_version', 'snapshot_id', 'vault_uuid',
                             'pair_id', 'board_metadata', 'files'})
    require(document['schema_version'] == 1 and set(document['files']) == set(ROOTS))
    actual = {name: inventory(source / name) for name in ROOTS}
    require(actual == document['files'])
    roots = {name: source / name for name in ROOTS}
    require(binding(roots) == {k: document[k] for k in ('vault_uuid', 'pair_id')})
    require(board_metadata(roots['board'], document['vault_uuid']) == document['board_metadata'])
    return document


def restore(source, expected, destination):
    document = verify(source, expected)
    new_destination(destination, [source])
    destination.mkdir(mode=0o700)
    copied = {name: inventory(source / name, destination / name) for name in ROOTS}
    require(copied == document['files'])
    require(verify(source, expected) == document)
    return document


def main():
    os.umask(0o077)
    p = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    commands = p.add_subparsers(dest='command', required=True)
    f = commands.add_parser('freeze', allow_abbrev=False)
    f.add_argument('--output', type=Path, required=True)
    f.add_argument('--snapshot-id', required=True)
    for command in ('verify', 'restore'):
        q = commands.add_parser(command, allow_abbrev=False)
        q.add_argument('--snapshot', type=Path, required=True)
        q.add_argument('--manifest-sha256', required=True)
        if command == 'restore':
            q.add_argument('--output', type=Path, required=True)
    args = p.parse_args()
    if args.command == 'freeze':
        result = freeze(ROOTS, args.output, args.snapshot_id, stopped)
    else:
        if args.command == 'verify':
            document = verify(args.snapshot, args.manifest_sha256)
        else:
            document = restore(args.snapshot, args.manifest_sha256, args.output)
        result = {'status': args.command + '_bytes_verified',
                  'snapshot_id': document['snapshot_id'],
                  'broker_rebind_accepted': False}
    print(json.dumps(result, sort_keys=True))


if __name__ == '__main__':
    try:
        main()
    except Exception:
        print(json.dumps({'status': 'failed', 'detail': 'snapshot invariant'}))
        sys.exit(1)
