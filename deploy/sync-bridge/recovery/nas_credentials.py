#!/usr/bin/python3
"""只由用户私有终端调用；bcrypt 经 stdin，TLS 私钥仅在 NAS 本机生成。"""
import os
from pathlib import Path
import re
import subprocess
import sys

ROOT = Path('/volume1/studiolo-dr/rest-server')
USER = 'studiolo-backup'


def main():
    if os.geteuid() != 0 or len(sys.argv) != 2:
        raise ValueError('operator')
    os.umask(0o077)
    if sys.argv[1] == 'install-auth':
        raw = sys.stdin.buffer.read(256)
        if not re.fullmatch(rb'studiolo-backup:\$2[aby]\$12\$[./A-Za-z0-9]{53}\n', raw):
            raise ValueError('auth format')
        fd = os.open(ROOT / 'backup.htpasswd', os.O_WRONLY | os.O_CREAT | os.O_EXCL
                     | os.O_NOFOLLOW, 0o640)
        os.fchown(fd, 0, 61032)
        os.fchmod(fd, 0o640)
        with os.fdopen(fd, 'wb') as stream:
            stream.write(raw)
            stream.flush()
            os.fsync(stream.fileno())
    elif sys.argv[1] == 'activate':
        if not sys.stdin.isatty() or not sys.stdout.isatty():
            raise ValueError('tty')
        if not (ROOT / 'backup.htpasswd').is_file():
            raise ValueError('auth missing')
        if (ROOT / 'tls.key').exists() or (ROOT / 'tls.crt').exists():
            raise ValueError('existing TLS material')
        config = ROOT / 'tls-request.cnf'
        with config.open('x') as stream:
            stream.write('[req]\ndistinguished_name=dn\nx509_extensions=ext\nprompt=no\n'
                         '[dn]\nCN=studiolo-rest-111\n[ext]\nsubjectAltName=IP:192.168.100.111\n'
                         'basicConstraints=critical,CA:TRUE\nkeyUsage=critical,digitalSignature,keyEncipherment,keyCertSign\n')
        subprocess.run(['/usr/bin/openssl', 'req', '-x509', '-newkey', 'rsa:3072',
                        '-nodes', '-days', '1095', '-config', str(config),
                        '-keyout', str(ROOT / 'tls.key'), '-out', str(ROOT / 'tls.crt')],
                       check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        for name in ('tls.key', 'tls.crt'):
            os.chown(ROOT / name, 0, 61032)
            os.chmod(ROOT / name, 0o640)
        subprocess.run(['/usr/bin/systemctl', 'enable', 'studiolo-rest-backup.service'], check=True)
        subprocess.run(['/usr/bin/systemctl', 'start', 'studiolo-rest-backup.service'], check=True)
        subprocess.run(['/usr/bin/systemctl', 'is-active', '--quiet', 'studiolo-rest-backup.service'], check=True)
    else:
        raise ValueError('command')
    print('nas_backup_setup=ready')


if __name__ == '__main__':
    try:
        main()
    except Exception:
        print('nas_backup_setup=failed', file=sys.stderr)
        sys.exit(1)
