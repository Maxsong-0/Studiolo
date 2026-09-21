#!/usr/bin/python3
"""核验既有人侧 r8 和 agent 单元，生成独立候选包；不切换服务。"""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import re
import stat
import sys

HERE = Path(os.path.abspath(__file__)).parent


def load(name, path):
    for directory in path.parents:
        s = directory.lstat()
        if not stat.S_ISDIR(s.st_mode) or s.st_uid not in (0, os.geteuid()) or s.st_mode & 0o7022:
            raise ValueError('unsafe tools')
    s = path.lstat()
    if not stat.S_ISREG(s.st_mode) or s.st_nlink != 1 or s.st_uid not in (0, os.geteuid()) or s.st_mode & 0o7022:
        raise ValueError('unsafe tools')
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def prepare(args, a, c):
    c.require(os.geteuid() == c.UID)
    c.require(re.fullmatch('[0-9a-f]{40}', args.source_commit))
    c.require(re.fullmatch('[0-9a-f]{40}', args.human_source_commit))
    c.require(re.fullmatch('[0-9a-f]{64}', args.vault_admin_sha256))
    c.require(re.fullmatch('[0-9a-f]{64}', args.local_server_sha256))
    old = Path(a.OLD_RELEASE)
    old_manifest = c.verify(old, a.R8_MANIFEST_SHA)
    units = {name: c.read(Path('/etc/systemd/system') / (name + '.service'), 0, 0o644)
             for name in ('studiolo-human', 'studiolo-agent')}
    c.require(c.sha(units['studiolo-human']) == a.HUMAN_SHA)
    c.require(c.sha(units['studiolo-agent']) == a.AGENT_SHA)
    bootstrap = c.read(c.absolute(args.vault_admin), c.UID)
    c.require(bootstrap.startswith(b'\x7fELF') and c.sha(bootstrap) == args.vault_admin_sha256)
    human = c.read(c.absolute(args.local_server), c.UID)
    c.require(human.startswith(b'\x7fELF') and c.sha(human) == args.local_server_sha256)
    output = Path(a.RELEASE)
    c.ancestors(output, c.UID)
    c.require(not os.path.lexists(output))
    output.mkdir(mode=0o700)
    c.new_file(output / 'INCOMPLETE', b'preparing\n')
    for name in old_manifest['files']:
        target = output / c.relative(name)
        target.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
        content = c.read(old / name, c.UID, c.mode_for(name))
        if name == 'bin/studiolo-local-server':
            content = human
        elif name == 'launch-human.py':
            content = a.launcher(content.decode()).encode()
        elif name == 'studiolo-human.service':
            content = a.human_unit(units['studiolo-human'].decode()).encode()
        c.new_file(target, content, c.mode_for(name))
    c.new_file(output / 'studiolo-agent.service', a.agent_unit(units['studiolo-agent'].decode()).encode())
    c.new_file(output / 'bin/studiolo-vault-admin', bootstrap, 0o755)
    c.verify(old, a.R8_MANIFEST_SHA)
    for name, raw in units.items():
        c.require(c.read(Path('/etc/systemd/system') / (name + '.service'), 0, 0o644) == raw)
    files, _ = c.scan(output, c.UID)
    del files['INCOMPLETE']
    manifest = {'build': a.BUILD, 'bootstrap_source_commit': args.source_commit,
                'human_program_source_commit': args.human_source_commit,
                'r8_manifest_sha256': a.R8_MANIFEST_SHA, 'assembly': 'canonical-board-consumer-only',
                'before_units': {name: c.sha(raw) for name, raw in units.items()}, 'files': files}
    raw = (json.dumps(manifest, sort_keys=True, indent=2) + '\n').encode()
    c.new_file(output / 'manifest.json', raw)
    (output / 'INCOMPLETE').unlink()
    return {'build': a.BUILD, 'manifest_sha256': c.sha(raw), 'files': len(files),
            'human_binary_reused': False, 'services_changed': False}


def main():
    os.umask(0o077)
    a = load('board_assembly', HERE / 'assembly.py')
    c = load('board_release_common', HERE.parent / 'human/release/release_common.py')
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    for name in ('source-commit', 'vault-admin', 'vault-admin-sha256',
                 'human-source-commit', 'local-server', 'local-server-sha256'):
        parser.add_argument('--' + name, required=True)
    args = parser.parse_args()
    print(json.dumps(prepare(args, a, c), sort_keys=True))


if __name__ == '__main__':
    try:
        main()
    except Exception:
        print('blackboard prepare rejected; partial candidate retained', file=sys.stderr)
        raise SystemExit(1)
