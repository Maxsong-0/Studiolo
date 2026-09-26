#!/usr/bin/python3
"""构建固定r8发行目录；仅复制显式输入，既有输出绝不覆盖。"""
import os
from pathlib import Path
import re
import stat
import sys

# -I 下只从已核验的本脚本目录加载固定辅助模块，不使用cwd/PYTHONPATH。
HERE = Path(os.path.abspath(__file__)).parent


def trusted_modules():
    for entry in [HERE, *HERE.parents]:
        item = entry.lstat()
        if not stat.S_ISDIR(item.st_mode) or item.st_uid not in (0, os.geteuid()) or item.st_mode & 0o022:
            raise SystemExit('package: unsafe tool path')
    item = (HERE / 'release_common.py').lstat()
    if not stat.S_ISREG(item.st_mode) or item.st_nlink != 1 or item.st_uid not in (0, os.geteuid()) or item.st_mode & 0o7022:
        raise SystemExit('package: unsafe tool')
    sys.path.insert(0, str(HERE))


def package(args):
    import release_common as c
    uid = os.geteuid()
    repo, example = c.absolute(args.repo), c.absolute(args.example_vault)
    output = c.absolute(args.output)
    c.require(re.fullmatch('[0-9a-f]{40}', args.source_commit) is not None)
    c.ancestors(output, uid)
    c.inspect(output.parent, uid if output.parent.stat().st_uid == uid else 0, directory=True)
    c.require(not os.path.lexists(output))
    roots = {'web': repo / 'web/dist', 'example-vault': example}
    initial = {label: c.scan(root, uid) for label, root in roots.items()}
    c.require(initial['web'][0] and initial['example-vault'][0])
    sources = {'bin/studiolo-local-server': c.absolute(args.local_server),
               'bin/studiolo-auth-admin': c.absolute(args.auth_admin),
               # T-132 P1：可信 launcher 随人侧发行目录出包，由 deploy/sync-bridge/provision.sh 装到 /usr/local/sbin。
               'bin/studiolo-sync-launcher': c.absolute(args.sync_launcher),
               'launch-human.py': repo / 'deploy/human/current-v2/launch-human.py',
               'studiolo-human.service': repo / 'deploy/human/current-v2/studiolo-human.service'}
    raw = {name: c.read(path, uid) for name, path in sources.items()}
    c.require(all(raw[name] for name in c.BINARIES))
    for name in ('launch-human.py', 'studiolo-human.service'):
        c.require(c.OLD_RELEASE.encode() in raw[name])
    output.mkdir(mode=0o700)
    c.new_file(output / 'INCOMPLETE', b'packing\n')
    for label, root in roots.items():
        for relative in initial[label][0]:
            target = output / label / relative
            target.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
            c.new_file(target, c.read(root / relative, uid))
    for name, content in raw.items():
        target = output / name
        target.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
        if name not in c.BINARIES:
            content = content.replace(c.OLD_RELEASE.encode(), str(c.RELEASE).encode())
        c.new_file(target, content, c.mode_for(name))
    c.require(all(c.scan(root, uid) == initial[label] for label, root in roots.items()))
    c.require(all(c.read(path, uid) == raw[name] for name, path in sources.items()))
    files, _ = c.scan(output, uid)
    del files['INCOMPLETE']
    manifest = {'build': c.BUILD, 'source_commit': args.source_commit,
                'assembly': 'human-v2-r8', 'files': files}
    import json
    encoded = (json.dumps(manifest, ensure_ascii=False, sort_keys=True, indent=2) + '\n').encode()
    c.new_file(output / 'manifest.json', encoded)
    # 临时标记只在完整写完后移除；失败绝不清理发行目录。
    (output / 'INCOMPLETE').unlink()
    try:
        c.verify(output, c.sha(encoded), uid)
    except BaseException:
        c.new_file(output / 'INCOMPLETE', b'verification failed\n')
        raise
    return {'build': c.BUILD, 'source_commit': args.source_commit,
            'manifest_sha256': c.sha(encoded), 'files': len(files)}


def options(argv):
    import release_common as c
    parser = c.ClosedParser(description=__doc__, allow_abbrev=False)
    for option in ('repo', 'example-vault', 'local-server', 'auth-admin', 'sync-launcher', 'source-commit', 'output'):
        parser.add_argument('--' + option, required=True)
    args = parser.parse_args(argv)
    c.require(len(argv) == 14)
    return args


def main():
    trusted_modules()
    args = options(sys.argv[1:])
    os.umask(0o077)
    import json
    print(json.dumps(package(args), sort_keys=True))


if __name__ == '__main__':
    try:
        main()
    except Exception:
        print('package: rejected; incomplete output retained', file=sys.stderr)
        raise SystemExit(1)
