#!/usr/bin/python3
"""用户在 169 私有 TTY 配置备份账号与仓库口令；不回显或写进 argv。"""
import getpass
import json
import os
from pathlib import Path
import subprocess
import sys

ROOT = Path('/home/maxsong/.config/studiolo-backup')
RESTIC = '/usr/local/bin/studiolo-restic'
REPOSITORY = 'rest:https://192.168.100.111:18443/studiolo-backup/t132/'


def environment():
    with (ROOT / 'credentials.json').open() as stream:
        config = json.load(stream)
    return {'PATH': '/usr/bin:/bin', 'HOME': '/home/maxsong',
            'RESTIC_REPOSITORY': REPOSITORY, 'RESTIC_REST_USERNAME': 'studiolo-backup',
            'RESTIC_REST_PASSWORD': config['rest_password'],
            'RESTIC_PASSWORD_FILE': str(ROOT / 'repository-password'),
            'RESTIC_CACERT': str(ROOT / 'ca.pem')}


def main():
    if os.getuid() != 1000 or sys.argv[1:] or not all(s.isatty() for s in (sys.stdin, sys.stdout, sys.stderr)):
        raise ValueError('operator')
    os.umask(0o077)
    if ROOT.is_symlink() or not (ROOT / 'ca.pem').is_file():
        raise ValueError('prepare first')
    if any((ROOT / name).exists() for name in ('credentials.json', 'repository-password')):
        raise ValueError('already configured')
    http = getpass.getpass('请输入刚设置的 NAS 追加备份账号密码：')
    repo = getpass.getpass('设置 restic 仓库加密口令（请存入密码管理器）：')
    if not http or len(repo) < 16 or repo != getpass.getpass('再次输入仓库加密口令：'):
        raise ValueError('password confirmation')
    if http == repo or '\n' in repo or '\r' in repo or '\0' in http + repo:
        raise ValueError('distinct passwords')
    with (ROOT / 'credentials.json').open('x') as stream:
        json.dump({'rest_password': http}, stream)
    with (ROOT / 'repository-password').open('x') as stream:
        stream.write(repo + '\n')
    result = subprocess.run([RESTIC, 'init', '--json'], env=environment(),
                            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    if result.returncode:
        raise ValueError('repository init')
    print('backup_repository_initialized=true')


if __name__ == '__main__':
    try:
        main()
    except (Exception, KeyboardInterrupt):
        print('backup_client_setup=failed; 请勿粘贴口令或配置正文。', file=sys.stderr)
        sys.exit(1)
