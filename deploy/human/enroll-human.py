#!/usr/bin/python3
"""主人控制终端的一次性 TOTP 配对；秘密不进入日志或普通配置。"""
import base64
import ctypes
import fcntl
import getpass
import sqlite3
import json
import os
from pathlib import Path
import pwd
import resource
import secrets
import stat
import struct
import subprocess
import sys
import tempfile

RELEASE = Path('/home/maxsong/.local/share/studiolo-human-releases/m4-t092-20260920-r2')
CONFIG = Path('/home/maxsong/.config/studiolo-human/owner.json')
CREDENTIAL = Path('/etc/credstore.encrypted/studiolo-human-totp')
BINARY = RELEASE / 'bin/studiolo-auth-admin'


def require(value):
    if not value:
        raise RuntimeError('precondition')


def protect():
    require(sys.platform == 'linux' and os.getuid() == os.geteuid() == 0)
    require(sys.flags.isolated and sys.flags.no_site and sys.dont_write_bytecode)
    require(os.environ.get('TERM', '') not in ('', 'dumb', 'unknown'))
    require(all(os.isatty(fd) for fd in (0, 1, 2)))
    require(os.tcgetpgrp(0) == os.getpgrp())
    terminal = os.open('/dev/tty', os.O_RDWR | os.O_NOCTTY | os.O_CLOEXEC)
    try:
        require(len({os.fstat(fd).st_rdev for fd in (0, 1, 2)}) == 1)
        for fd in (0, 1, 2, terminal):
            require(os.tcgetpgrp(fd) == os.getpgrp())
            session = struct.unpack('i', fcntl.ioctl(fd, 0x5429, bytes(4)))[0]
            require(session == os.getsid(0))  # Linux TIOCGSID，必须同一控制终端会话。
    finally:
        os.close(terminal)
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
    resource.setrlimit(resource.RLIMIT_MEMLOCK, (resource.RLIM_INFINITY, resource.RLIM_INFINITY))
    libc = ctypes.CDLL(None, use_errno=True)
    require(libc.prctl(4, 0, 0, 0, 0) == 0 and libc.prctl(3, 0, 0, 0, 0) == 0)
    require(libc.mlockall(3) == 0)
    os.umask(0o077)


def regular(path, uid, mode):
    metadata = path.lstat()
    require(stat.S_ISREG(metadata.st_mode) and metadata.st_nlink == 1)
    require(metadata.st_uid == uid and stat.S_IMODE(metadata.st_mode) == mode)
    for parent in path.parents:
        item = parent.lstat()
        require(stat.S_ISDIR(item.st_mode) and item.st_uid in (0, uid))
        require(stat.S_IMODE(item.st_mode) & 0o7022 == 0)


def run_admin(owner, command, seed=None):
    env = {'PATH': '/usr/bin:/bin', 'LANG': 'C.UTF-8'}
    if seed is not None:
        env['STUDIOLO_TOTP_SEED_HEX'] = seed
    args = [str(BINARY), command, '--config', str(CONFIG)]
    if command == 'migrate':
        args += ['--from', '3']
    # 继承真实控制终端，OTP 由产品隐藏读取；不采集输出、不经管道灌验证码。
    result = subprocess.run(args, env=env, user=owner.pw_uid, group=owner.pw_gid,
                            extra_groups=[], check=False)
    require(result.returncode == 0)


def create_credential(seed):
    directory = CREDENTIAL.parent
    directory.mkdir(mode=0o700, exist_ok=True)
    item = directory.lstat()
    require(stat.S_ISDIR(item.st_mode) and item.st_uid == 0)
    require(stat.S_IMODE(item.st_mode) & 0o7022 == 0)
    encrypted = subprocess.run(
        ['/usr/bin/systemd-creds', 'encrypt', '--name=totp-seed', '--with-key=host', '-', '-'],
        input=seed.encode('ascii'), stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        env={'PATH': '/usr/bin:/bin', 'LANG': 'C.UTF-8'}, check=False,
    )
    require(encrypted.returncode == 0 and 0 < len(encrypted.stdout) < 16384)
    fd, temporary = tempfile.mkstemp(prefix='.studiolo-human-totp-', dir=directory)
    try:
        with os.fdopen(fd, 'wb') as output:
            output.write(encrypted.stdout)
            output.flush()
            os.fsync(output.fileno())
        os.link(temporary, CREDENTIAL, follow_symlinks=False)  # 不覆盖已有凭据。
    finally:
        os.unlink(temporary)
    directory_fd = os.open(directory, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        os.fsync(directory_fd)
    finally:
        os.close(directory_fd)


def read_credential():
    regular(CREDENTIAL, 0, 0o600)
    decrypted = subprocess.run(
        ['/usr/bin/systemd-creds', 'decrypt', '--name=totp-seed', str(CREDENTIAL), '-'],
        stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        env={'PATH': '/usr/bin:/bin', 'LANG': 'C.UTF-8'}, check=False,
    )
    require(decrypted.returncode == 0 and len(decrypted.stdout) == 40)
    require(all(c in b'0123456789abcdef' for c in decrypted.stdout))
    return decrypted.stdout.decode('ascii')


def main():
    resume = sys.argv[1:] == ['--resume']
    require(resume or len(sys.argv) == 1)
    protect()
    owner = pwd.getpwnam('maxsong')
    regular(CONFIG, owner.pw_uid, 0o600)
    regular(BINARY, owner.pw_uid, 0o755)
    config = json.loads(CONFIG.read_text())
    require(config['seed_ref'] == 'studiolo-human-totp-v1')
    require(CREDENTIAL.exists() == resume)
    database = Path(config['auth_directory'])/'auth.sqlite3'
    if database.exists():
        regular(database, owner.pw_uid, 0o600)
        with sqlite3.connect('file:' + str(database) + '?mode=ro', uri=True) as db:
            db.execute('PRAGMA query_only=ON')
            db.execute('PRAGMA trusted_schema=OFF')
            require(db.execute('SELECT count(*) FROM totp_factor').fetchone() == (0,))
            require(db.execute('SELECT count(*) FROM totp_pending').fetchone() == (0,))
    print('此步骤需要你在手机验证器中添加 Studiolo，验证码只在本终端输入。')
    print('不要截图、录屏或把下面的配对密钥发到聊天。请将密钥另存于你的密码管理器。')
    require(getpass.getpass('准备好后只按回车开始；这里不输入验证码：') == '')
    run_admin(owner, 'migrate')
    seed = read_credential() if resume else secrets.token_hex(20)
    if not resume:
        create_credential(seed)
    os.write(1, b'\x1b[?1049h\x1b[2J\x1b[H')
    try:
        base32 = base64.b32encode(bytes.fromhex(seed)).decode('ascii').rstrip('=')
        print('验证器：手动添加 Studiolo；SHA1；6位；30秒。密钥只保存在密码管理器，不发聊天。')
        print('配对密钥：', base32, flush=True)
        run_admin(owner, 'enroll', seed)
    finally:
        os.write(1, b'\x1b[2J\x1b[H\x1b[?1049l')
    print('首次 TOTP 登记已完成；尚未签发应用会话或 L3。请回到 Codex 继续上线验收。')


if __name__ == '__main__':
    try:
        main()
    except (Exception, KeyboardInterrupt):
        # 不打印 traceback/对象。失败可留下已确认库或密文，禁止清库或覆盖旧因子重试。
        os.write(2, '配对未确认完成；保留现有状态，请让 Codex 核查。不要删除数据库或密文。\n'.encode())
        raise SystemExit(1)
