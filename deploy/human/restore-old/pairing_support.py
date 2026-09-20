#!/usr/bin/python3
"""私有控制终端、加密凭据和临时配对画面的共用函数。"""
import base64
import ctypes
import fcntl
import os
import resource
import stat
import struct
import subprocess
import sys
import tempfile

CREDENTIAL = None
CREDENTIAL_ID = None


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


def create_credential(seed):
    directory = CREDENTIAL.parent
    directory.mkdir(mode=0o700, exist_ok=True)
    item = directory.lstat()
    require(stat.S_ISDIR(item.st_mode) and item.st_uid == 0)
    require(stat.S_IMODE(item.st_mode) & 0o7022 == 0)
    encrypted = subprocess.run(
        ['/usr/bin/systemd-creds', 'encrypt', '--name=' + CREDENTIAL_ID, '--with-key=host', '-', '-'],
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
        ['/usr/bin/systemd-creds', 'decrypt', '--name=' + CREDENTIAL_ID, str(CREDENTIAL), '-'],
        stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        env={'PATH': '/usr/bin:/bin', 'LANG': 'C.UTF-8'}, check=False,
    )
    require(decrypted.returncode == 0 and len(decrypted.stdout) == 40)
    require(all(c in b'0123456789abcdef' for c in decrypted.stdout))
    return decrypted.stdout.decode('ascii')


def pairing_screen(seed, finish, label='Studiolo'):
    # 临时终端画面减少滚屏误复制；不承诺抵御截图、录屏或终端日志插件。
    os.write(1, b'\x1b[?1049h\x1b[2J\x1b[H')
    try:
        base32 = base64.b32encode(bytes.fromhex(seed)).decode('ascii').rstrip('=')
        print('手机验证器：手动添加；名称 ' + label + '；基于时间；SHA1；6位；30秒。')
        print('以下密钥只保存在你的密码管理器，不要截图或复制到聊天。')
        print('配对密钥：', base32, flush=True)
        finish()
    finally:
        os.write(1, b'\x1b[2J\x1b[H\x1b[?1049l')
