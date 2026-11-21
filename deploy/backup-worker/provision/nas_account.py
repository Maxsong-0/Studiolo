#!/usr/bin/python3
"""NAS 上给备份 worker 加一个只追加账号：往 rest-server 的账号文件里加一行。由 nas-account.command 经 ssh 调用。

    sudo -n python3 -I -B nas_account.py check      只读：备份服务的状态、几个文件的属主权限、现有账号名、端口
    sudo -n python3 -I -B nas_account.py append     从标准输入读一行 bcrypt（账号名固定），原子地加进账号文件
                                                    （同一时间只许一处在改；中途失败不留半份文件）
    sudo -n python3 -I -B nas_account.py restart    重启备份服务，等它开始监听
rest-server 的装法见 deploy/sync-bridge/recovery/README.md「NAS 追加备份」。NAS 上是 Python 3.8，这里只用它认得的写法。
不读、不打印任何口令或账号文件里的散列；清理权限照旧只在 NAS 管理员手里，不给这个账号。
"""
import fcntl
import hashlib
import os
from pathlib import Path
import re
import socket
import ssl
import stat
import subprocess
import sys
import time

ROOT = Path('/volume1/studiolo-dr/rest-server')
SERVICE = 'studiolo-rest-backup.service'
SYSTEMCTL = '/usr/bin/systemctl'
USER = 'studiolo-backup-worker'
RUNTIME_ID = 61032
# 账号文件：root 所有，属组是 rest-server 的运行账号（它只读得到），0640。
ACCOUNT_FILE_OWNER = (0, RUNTIME_ID)
ADDRESS = ('192.168.100.111', 18443)
LINE = re.compile(rb'studiolo-backup-worker:\$2[aby]\$12\$[./A-Za-z0-9]{53}\n')
ACCOUNT_FILE_LIMIT = 64 * 1024


def accounts(raw):
    return [line.split(b':', 1)[0].decode('ascii', 'replace') for line in raw.splitlines() if line.strip()]


def write_new(path, data):
    """只新建。没写完整就删掉：半份副本会让下次重跑对不上，半份新文件里还带着新账号的散列。"""
    descriptor = os.open(str(path), os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o640)
    try:
        with os.fdopen(descriptor, 'wb') as stream:
            os.fchown(stream.fileno(), *ACCOUNT_FILE_OWNER)
            os.fchmod(stream.fileno(), 0o640)
            stream.write(data)
            stream.flush()
            os.fsync(stream.fileno())
    except BaseException:
        os.unlink(str(path))
        raise


def append(raw):
    """旧内容一字不动，新账号加在最后；先留一份改之前的副本，再用改名一次换上。"""
    if not LINE.fullmatch(raw):
        raise ValueError('账号行的格式不对')
    # 从读旧内容到换上新文件整段只许一处在做：两处同时做，各自都会「成功」，后换上的那个悄悄盖掉前一个。
    # 锁加在目录本身上（不另建锁文件），进程一结束就放开。
    directory = os.open(str(ROOT), os.O_RDONLY | os.O_DIRECTORY)
    try:
        try:
            fcntl.flock(directory, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            raise ValueError('另一处正在改账号文件：等它结束再跑')
        append_locked(raw)
        os.fsync(directory)
    finally:
        os.close(directory)


def append_locked(raw):
    target = ROOT / 'backup.htpasswd'
    status = os.lstat(str(target))
    if not (stat.S_ISREG(status.st_mode) and (status.st_uid, status.st_gid) == ACCOUNT_FILE_OWNER
            and stat.S_IMODE(status.st_mode) == 0o640 and status.st_nlink == 1
            and status.st_size <= ACCOUNT_FILE_LIMIT):
        raise ValueError('账号文件不是预期的样子')
    old = target.read_bytes()
    if USER in accounts(old):
        raise ValueError('这个账号已经在了')
    keep = ROOT / ('backup.htpasswd.before-' + USER)
    if os.path.lexists(str(keep)):
        if keep.read_bytes() != old:
            raise ValueError('上一次留下的副本和现在的账号文件不一样')
    else:
        write_new(keep, old)
    fresh = ROOT / ('backup.htpasswd.new-' + str(os.getpid()))
    write_new(fresh, old + (b'' if old.endswith(b'\n') or not old else b'\n') + raw)
    try:
        os.replace(str(fresh), str(target))
    except BaseException:
        os.unlink(str(fresh))
        raise


def listening():
    try:
        with socket.create_connection(ADDRESS, timeout=3):
            return True
    except OSError:
        return False


def state(*arguments):
    done = subprocess.run([SYSTEMCTL] + list(arguments) + [SERVICE], stdout=subprocess.PIPE,
                          stderr=subprocess.STDOUT, universal_newlines=True)
    return done.stdout.strip() or ('退出码 ' + str(done.returncode))


def check(out):
    out('单元文件：' + ('在' if os.path.isfile('/etc/systemd/system/' + SERVICE) else '不在'))
    out('开机启用：' + state('is-enabled') + '；现在：' + state('is-active'))
    out('端口 ' + ADDRESS[0] + ':' + str(ADDRESS[1]) + '：' + ('有人监听' if listening() else '没人监听'))
    for name in ('rest-server', 'backup.htpasswd', 'tls.crt', 'tls.key', 'data'):
        try:
            status = os.lstat(str(ROOT / name))
            out('  ' + name + '：' + stat.filemode(status.st_mode) + ' ' + str(status.st_uid) + ':' + str(status.st_gid)
                + ' ' + str(status.st_size))
        except OSError:
            out('  ' + name + '：不在')
    try:
        out('现有账号：' + '、'.join(accounts((ROOT / 'backup.htpasswd').read_bytes())))
    except OSError:
        out('现有账号：读不到账号文件')
    try:
        beside = sorted(name for name in os.listdir(str(ROOT)) if name.startswith('backup.htpasswd.'))
        out('账号文件旁边的其他文件（只列名字）：' + ('、'.join(beside) or '没有'))
    except OSError:
        out('账号文件旁边的其他文件：列不出来')
    try:
        certificate = ssl.PEM_cert_to_DER_cert((ROOT / 'tls.crt').read_text())
        out('证书摘要（sha256，公开信息）：' + hashlib.sha256(certificate).hexdigest())
    except (OSError, ValueError):
        out('证书摘要：读不到证书')
    out('---- systemctl status（前 15 行）')
    for line in state('status', '--no-pager', '-l').splitlines()[:15]:
        out(line)


def restart(out):
    subprocess.run([SYSTEMCTL, 'restart', SERVICE], check=True)
    for _ in range(30):
        if listening():
            break
        time.sleep(0.5)
    if state('is-active') != 'active' or not listening():
        raise ValueError('备份服务重启之后没有在监听')
    out('nas_rest_server=listening')


def main(argv, out=print):
    if os.geteuid() != 0 or len(argv) != 1 or argv[0] not in ('check', 'append', 'restart'):
        raise ValueError('用法见文件开头；要经 sudo 跑')
    os.umask(0o077)
    if argv[0] == 'check':
        check(out)
    elif argv[0] == 'append':
        append(sys.stdin.buffer.read(256))
        out('nas_account=appended')
    else:
        restart(out)


if __name__ == '__main__':
    try:
        main(sys.argv[1:])
    except (ValueError, OSError, subprocess.CalledProcessError) as failure:
        print('nas_account=failed：' + str(failure), file=sys.stderr)
        sys.exit(1)
