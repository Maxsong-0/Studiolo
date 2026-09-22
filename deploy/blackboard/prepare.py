#!/usr/bin/python3
"""核验已激活的上一发行与两个现场单元，生成新的候选包；不切换服务。

每次都换 `web/`；人侧程序可选：给了 `--local-server/--local-server-sha256/--human-source-commit`
三件套就换成新构建，否则逐字节沿用上一发行。示例库、Auth 管理程序、初始化程序、agent 单元一律沿用。
manifest 记 `web_source_commit`：网页必须能追到构建它的提交——r8→r2 那条链路没有这一项，
网页因此在三个发行里悄悄沿用了 T-097 之前的旧构建（BUG-015 线上一直没修好的根因）。
程序来源同理：换了就记新提交，没换就照抄上一发行，绝不把沿用的程序冒称为新构建。
"""
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


def mode_for(c, name):
    """发行里的可执行位：两个 r8 时代的二进制外，r2 起还多一个初始化程序。"""
    return 0o755 if name in c.BINARIES | {'bin/studiolo-vault-admin'} else 0o600


def prepare(args, a, c, ops):
    c.require(os.geteuid() == c.UID)
    c.require(re.fullmatch('[0-9a-f]{40}', args.web_source_commit))
    program = (args.local_server, args.local_server_sha256, args.human_source_commit)
    c.require(all(v is None for v in program) or all(v is not None for v in program))
    human = None
    if args.local_server is not None:
        c.require(re.fullmatch('[0-9a-f]{64}', args.local_server_sha256))
        c.require(re.fullmatch('[0-9a-f]{40}', args.human_source_commit))
        human = c.read(c.absolute(args.local_server), c.UID)
        c.require(human.startswith(b'\x7fELF') and c.sha(human) == args.local_server_sha256)
    old = Path(a.OLD_RELEASE)
    # 上一发行是黑板形状的 manifest，用同一套闭集核验，而不是 r8 时代的 verifier。
    old_manifest = ops.verify_previous(a, c)
    units = {name: c.read(Path('/etc/systemd/system') / (name + '.service'), 0, 0o644)
             for name in ('studiolo-human', 'studiolo-agent')}
    c.require(c.sha(units['studiolo-human']) == a.HUMAN_SHA)
    c.require(c.sha(units['studiolo-agent']) == a.AGENT_SHA)
    web_root = c.absolute(args.web)
    web, _ = c.scan(web_root, c.UID)
    c.require(web and all(not name.startswith('.') for name in web))
    c.require('index.html' in web)
    output = Path(a.RELEASE)
    c.ancestors(output, c.UID)
    c.require(not os.path.lexists(output))
    output.mkdir(mode=0o700)
    c.new_file(output / 'INCOMPLETE', b'preparing\n')
    for name in old_manifest['files']:
        if name.startswith(a.WEB_PREFIX):
            continue
        target = output / c.relative(name)
        target.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
        content = c.read(old / name, c.UID, mode_for(c, name))
        if name == 'bin/studiolo-local-server' and human is not None:
            content = human
        elif name == 'launch-human.py':
            content = a.launcher(content.decode()).encode()
        elif name == 'studiolo-human.service':
            content = a.human_unit(units['studiolo-human'].decode()).encode()
        elif name == 'studiolo-agent.service':
            content = a.agent_unit(units['studiolo-agent'].decode()).encode()
        c.new_file(target, content, mode_for(c, name))
    for name in web:
        target = output / c.relative(a.WEB_PREFIX + name)
        target.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
        c.new_file(target, c.read(web_root / c.relative(name), c.UID), 0o600)
    ops.verify_previous(a, c)
    for name, raw in units.items():
        c.require(c.read(Path('/etc/systemd/system') / (name + '.service'), 0, 0o644) == raw)
    files, _ = c.scan(output, c.UID)
    del files['INCOMPLETE']
    manifest = {'build': a.BUILD, 'bootstrap_source_commit': old_manifest['bootstrap_source_commit'],
                'human_program_source_commit': (args.human_source_commit if human is not None
                                                else old_manifest['human_program_source_commit']),
                'web_source_commit': args.web_source_commit,
                'r8_manifest_sha256': a.R8_MANIFEST_SHA, 'assembly': a.ASSEMBLY,
                'before_units': {name: c.sha(raw) for name, raw in units.items()}, 'files': files}
    raw = (json.dumps(manifest, sort_keys=True, indent=2) + '\n').encode()
    c.new_file(output / 'manifest.json', raw)
    (output / 'INCOMPLETE').unlink()
    replaced = sorted(name for name in files if name.startswith(a.WEB_PREFIX))
    return {'build': a.BUILD, 'manifest_sha256': c.sha(raw), 'files': len(files),
            'web_files': len(replaced), 'human_binary_reused': human is None,
            'services_changed': False}


def main():
    os.umask(0o077)
    a = load('board_assembly', HERE / 'assembly.py')
    c = load('board_release_common', HERE.parent / 'human/release/release_common.py')
    ops = load('board_ops', HERE / 'activation_ops.py')
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    for name in ('web', 'web-source-commit'):
        parser.add_argument('--' + name, required=True)
    # 换人侧程序时三件套一起给；都不给则沿用上一发行的程序。
    for name in ('local-server', 'local-server-sha256', 'human-source-commit'):
        parser.add_argument('--' + name)
    args = parser.parse_args()
    print(json.dumps(prepare(args, a, c, ops), sort_keys=True))


if __name__ == '__main__':
    try:
        main()
    except Exception:
        print('blackboard prepare rejected; partial candidate retained', file=sys.stderr)
        raise SystemExit(1)
