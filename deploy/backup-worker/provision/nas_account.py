#!/usr/bin/python3
"""NAS 上备份服务（rest-server）的两件事：给备份 worker 加一个只追加账号；换上开机后会接着试的服务单元。
由 nas-account.command 经 ssh 调用。

    sudo -n python3 -I -B nas_account.py check      只读：备份服务的状态、几个文件的属主权限、现有账号名、端口、
                                                    这次开机和已装单元的情况
    sudo -n python3 -I -B nas_account.py append     从标准输入读一行 bcrypt（账号名固定），原子地加进账号文件
                                                    （同一时间只许一处在改；中途失败不留半份文件；被强杀或断电
                                                    留下的临时文件 <正式名>.new，下一次会认出来并停下，不删它）
    sudo -n python3 -I -B nas_account.py restart    重启备份服务，等它开始监听
    sudo -n python3 -I -B nas_account.py unit       从标准输入读新的单元文件：先在用完即删的临时单元上演一遍
                                                    「没准备好就隔一会儿再试」，再换上、重新加载、重启；
                                                    服务没起来就换回旧的
rest-server 的装法见 deploy/sync-bridge/recovery/README.md「NAS 追加备份」。NAS 上是 Python 3.8，这里只用它认得的写法。
不读、不打印任何口令或账号文件里的散列；清理权限照旧只在 NAS 管理员手里，不给这个账号。
"""
import fcntl
import hashlib
import os
from pathlib import Path
import re
import signal
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

UNIT = Path('/etc/systemd/system') / SERVICE
UNIT_OWNER = (0, 0)
UNIT_LIMIT = 16 * 1024
# 单元文件只认仓库里 deploy/sync-bridge/recovery/studiolo-rest-backup.service 的两版（内容的 sha256）：
# 现场是上一版才换，已经是这一版就不动，别的内容先由人看过。
# 上一版用 Condition…= 把关：开机时存储卷还没挂上，条件不成立就被跳过、之后不再试。
UNIT_SHA256 = 'bbb230cbd031d81f7569be47f2d553b2607dcf13e8bb37d622d88dafdee8095b'
UNIT_BEFORE_SHA256 = '98e065e30fa522b05f080b12173394d1bcb6ad3d85f31daa91025abdd10ebddd'
PRECHECK = re.compile(rb"^ExecStartPre=/bin/sh -c '([^'\n]+)'$", re.M)
# 演练用的临时单元放在 /run 下：重启就没了，不会留进开机启动。它只跑 sleep，启动前检查看的是旁边一个标记文件。
REHEARSAL_UNIT = Path('/run/systemd/system/studiolo-boot-rehearsal.service')
REHEARSAL_FLAG = Path('/run/studiolo-boot-rehearsal.ready')
REHEARSAL = ("[Service]\nType=simple\nExecStartPre=/bin/sh -c 'test -f {flag}'\nExecStart=/bin/sh -c 'exec sleep 120'\n"
             'Restart=always\nRestartSec=1\nStartLimitInterval=0\n')
# systemd 默认 10 秒内启动满 5 次就不再启动。演练单元每秒试一次，等 8 秒才放行：
# 那时已经试过不止 5 次，还在试才说明 StartLimitInterval=0 真把次数上限关掉了。
REHEARSAL_WAIT_SECONDS = 8
JOURNALCTL = ('/usr/bin/journalctl', '/bin/journalctl')


class Interrupted(KeyboardInterrupt):
    """收到 SIGHUP、SIGTERM（ssh 断了、被要求结束）：和 Ctrl-C 一样当成被打断，让收尾照常跑。"""


def interruptible():
    """这几个信号的默认动作是直接结束进程，finally 不跑：写到一半的文件、演练用的临时单元会留下。
    改成抛异常；收到第一个之后其余的都不再理，免得收尾做到一半又被打断。"""
    watched = (signal.SIGHUP, signal.SIGTERM, signal.SIGINT)

    def stop(number, frame):
        for other in watched:
            signal.signal(other, signal.SIG_IGN)
        raise Interrupted(signal.Signals(number).name)
    for number in watched:
        signal.signal(number, stop)


def accounts(raw):
    return [line.split(b':', 1)[0].decode('ascii', 'replace') for line in raw.splitlines() if line.strip()]


def digest(raw):
    return hashlib.sha256(raw).hexdigest()


def write_new(path, data, owner, mode):
    """只新建。没写完整就删掉：半份副本会让下次重跑对不上，半份新文件里还带着新账号的散列。"""
    descriptor = os.open(str(path), os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, mode)
    try:
        with os.fdopen(descriptor, 'wb') as stream:
            os.fchown(stream.fileno(), *owner)
            os.fchmod(stream.fileno(), mode)
            stream.write(data)
            stream.flush()
            os.fsync(stream.fileno())
    except BaseException:
        os.unlink(str(path))
        raise


def replacing(target):
    """换文件时用的临时名：和正式名在同一个目录里，名字是固定的。"""
    return target.parent / (target.name + '.new')


STALE = (' 已经在：上一次换到一半被强杀或断电留下的，里面是那一次要换上的内容。'
         '不删它、也不接着用：在 NAS 上看过之后 sudo rm 它，再重跑')


def refuse_stale(target):
    """换 target 用的临时名还在就停下，什么都不动。走到这里时锁在这边手里，没有别的 nas_account.py 在改：
    它只可能是留下来的（账号文件旁边的那一份里带着新账号的散列）。"""
    if os.path.lexists(str(replacing(target))):
        raise ValueError(str(replacing(target)) + STALE)


def replace_with(target, data, owner, mode):
    """同目录写一份完整的新文件，再用改名一次换上，并把所在目录落盘（不然断电后换上的那一下可能不算数）；
    换不上就删掉新写的那份。临时名是固定的、只新建：被强杀或断电留下的那一份，下一次认得出来（见 refuse_stale），
    不会带着各不相同的名字一份份攒下去。"""
    fresh = replacing(target)
    write_new(fresh, data, owner, mode)
    try:
        os.replace(str(fresh), str(target))
    except BaseException:
        os.unlink(str(fresh))
        raise
    directory = os.open(str(target.parent), os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(directory)
    finally:
        os.close(directory)


def exclusively(action):
    """NAS 上的改动同一时间只许一处在做：两处同时做，各自都会「成功」，后换上的那个悄悄盖掉前一个。
    锁加在目录本身上（不另建锁文件），进程一结束就放开。"""
    directory = os.open(str(ROOT), os.O_RDONLY | os.O_DIRECTORY)
    try:
        try:
            fcntl.flock(directory, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            raise ValueError('另一处正在改账号文件或服务单元：等它结束再跑')
        action()
        os.fsync(directory)
    finally:
        os.close(directory)


def append(raw):
    """旧内容一字不动，新账号加在最后；先留一份改之前的副本，再用改名一次换上。"""
    if not LINE.fullmatch(raw):
        raise ValueError('账号行的格式不对')
    exclusively(lambda: append_locked(raw))


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
    refuse_stale(target)
    keep = ROOT / ('backup.htpasswd.before-' + USER)
    if os.path.lexists(str(keep)):
        if keep.read_bytes() != old:
            raise ValueError('上一次留下的副本和现在的账号文件不一样')
    else:
        write_new(keep, old, ACCOUNT_FILE_OWNER, 0o640)
    replace_with(target, old + (b'' if old.endswith(b'\n') or not old else b'\n') + raw, ACCOUNT_FILE_OWNER, 0o640)


def listening():
    try:
        with socket.create_connection(ADDRESS, timeout=3):
            return True
    except OSError:
        return False


def state(*arguments, unit=SERVICE):
    done = subprocess.run([SYSTEMCTL] + list(arguments) + [unit], stdout=subprocess.PIPE,
                          stderr=subprocess.STDOUT, universal_newlines=True)
    return done.stdout.strip() or ('退出码 ' + str(done.returncode))


def journal():
    """本次开机以来 systemd 自己对备份服务记了什么（只取 systemd 的那几句，不取 rest-server 的输出）。"""
    for program in JOURNALCTL:
        if os.path.isfile(program):
            done = subprocess.run([program, '-b', '--no-pager', '-n', '12', '_PID=1', 'UNIT=' + SERVICE],
                                  stdout=subprocess.PIPE, stderr=subprocess.STDOUT, universal_newlines=True)
            return done.stdout.splitlines()
    return ['这台 NAS 上没有 journalctl']


def boot_facts(out):
    """开机这一段的情况：装的是哪一版单元，备份服务有没有赶在存储卷之前被启动。"""
    try:
        seen = digest(UNIT.read_bytes())
        out('已装的单元：' + ('这一版（没准备好会隔一会儿再试）' if seen == UNIT_SHA256
                              else '上一版（开机时存储卷没挂上就被跳过、不再试）' if seen == UNIT_BEFORE_SHA256
                              else '不认识的内容，摘要 ' + seen))
    except OSError:
        out('已装的单元：读不到')
    try:
        since = time.time() - float(Path('/proc/uptime').read_text().split()[0])
        out('NAS 这次开机：' + time.strftime('%Y-%m-%d %H:%M:%S %Z', time.localtime(since)))
    except (OSError, ValueError, IndexError):
        out('NAS 这次开机：读不到')
    out('存储卷挂上：' + state('show', '-p', 'ActiveEnterTimestamp', unit='volume1.mount'))
    out('备份服务：' + state('show', '-p', 'ConditionTimestamp', '-p', 'ConditionResult',
                             '-p', 'ActiveEnterTimestamp').replace('\n', '；'))
    out('---- 本次开机以来 systemd 对备份服务的记录（最后 12 行）')
    for line in journal():
        out(line)


def check(out):
    out('单元文件：' + ('在' if os.path.isfile(str(UNIT)) else '不在'))
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
    stale = [str(replacing(target)) for target in (ROOT / 'backup.htpasswd', UNIT) if os.path.lexists(str(replacing(target)))]
    out('换到一半留下的临时文件：' + ('、'.join(stale) + '（add、unit 会停在它上面：看过之后 sudo rm）' if stale else '没有'))
    try:
        certificate = ssl.PEM_cert_to_DER_cert((ROOT / 'tls.crt').read_text())
        out('证书摘要（sha256，公开信息）：' + hashlib.sha256(certificate).hexdigest())
    except (OSError, ValueError):
        out('证书摘要：读不到证书')
    boot_facts(out)
    out('---- systemctl status（前 15 行）')
    for line in state('status', '--no-pager', '-l').splitlines()[:15]:
        out(line)


def came_up():
    """等备份服务开始监听（最多 15 秒）；起来了才算：systemd 说它在跑，端口也有人监听。"""
    for _ in range(30):
        if listening():
            break
        time.sleep(0.5)
    return state('is-active') == 'active' and listening()


def restart(out):
    subprocess.run([SYSTEMCTL, 'restart', SERVICE], check=True)
    if not came_up():
        raise ValueError('备份服务重启之后没有在监听')
    out('nas_rest_server=listening')


def ready_now(raw):
    """新单元里的启动前检查，现在（存储卷挂着、文件都在）就该过：过不了的话，换上去服务起不来。"""
    found = PRECHECK.findall(raw)
    if len(found) != 1:
        raise ValueError('单元文件里没有那一条启动前检查')
    return subprocess.run(['/bin/sh', '-c', found[0].decode('ascii')]).returncode == 0


def rehearse(out):
    """不重启 NAS，先验这台 systemd 的三件事：启动前检查没过，隔一会儿会再试；试多少次都不会被次数上限拦住；
    检查过了，服务就起来。用一个只跑 sleep 的临时单元：先让检查过不了，等它试过不止 5 次再放行，
    看它是不是自己起来了。演完删掉。"""
    name = REHEARSAL_UNIT.name
    for path in (REHEARSAL_UNIT, REHEARSAL_FLAG):
        if os.path.lexists(str(path)):
            raise ValueError('上一次演练留下了 ' + str(path) + '：先由人看过')
    os.makedirs(str(REHEARSAL_UNIT.parent), mode=0o755, exist_ok=True)
    before = after = '没走到'
    try:
        write_new(REHEARSAL_UNIT, REHEARSAL.format(flag=REHEARSAL_FLAG).encode('ascii'), UNIT_OWNER, 0o644)
        subprocess.run([SYSTEMCTL, 'daemon-reload'], check=True)
        if state('show', '-p', 'LoadState', unit=name) != 'LoadState=loaded':
            raise ValueError('这台 NAS 的 systemd 没认到放在 ' + str(REHEARSAL_UNIT.parent) + ' 下的临时单元：什么都没换')
        # 标记文件还没有，这一次启动按设计失败。
        subprocess.run([SYSTEMCTL, 'start', name], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        time.sleep(REHEARSAL_WAIT_SECONDS)
        before = state('is-active', unit=name)
        write_new(REHEARSAL_FLAG, b'', UNIT_OWNER, 0o600)
        for _ in range(40):
            after = state('is-active', unit=name)
            if after == 'active':
                break
            time.sleep(0.5)
    finally:
        missed = end_rehearsal()
        if missed:
            raise ValueError('演练留下的东西没收干净（' + '；'.join(missed) + '）：先由人看过。什么都没换。')
    if before == 'active' or after != 'active':
        raise ValueError('演练没过：这台 NAS 的 systemd 在启动前检查没过之后没有一直再试（放行之前：' + before
                         + '；放行之后：' + after + '）。什么都没换。')
    out('nas_rehearsal=retries_until_ready')


def end_rehearsal():
    """收走演练留下的东西。哪一步没做成都接着做后面的，最后把没做成的列出来（空着才算收干净）。"""
    name, missed = REHEARSAL_UNIT.name, []

    def attempt(what, action):
        try:
            action()
        except (OSError, subprocess.CalledProcessError) as failure:
            missed.append(what + '没做成：' + str(failure))

    def remove(path):
        if os.path.lexists(str(path)):
            os.unlink(str(path))
    # 停和清失败记录都不看退出码（临时单元没被认到时它们本来就不成功）：看的是最后它还在不在跑。
    attempt('停临时单元', lambda: subprocess.run([SYSTEMCTL, 'stop', name], stdout=subprocess.DEVNULL,
                                                 stderr=subprocess.DEVNULL))
    for path in (REHEARSAL_UNIT, REHEARSAL_FLAG):
        attempt('删 ' + str(path), lambda path=path: remove(path))
    attempt('重新加载 systemd', lambda: subprocess.run([SYSTEMCTL, 'daemon-reload'], check=True))
    attempt('清临时单元的失败记录', lambda: subprocess.run([SYSTEMCTL, 'reset-failed', name], stdout=subprocess.DEVNULL,
                                                           stderr=subprocess.DEVNULL))
    try:
        left = state('is-active', unit=name)
    except OSError as failure:
        left = '查不到：' + str(failure)
    if left in ('active', 'activating', 'reloading', 'deactivating') or left.startswith('查不到'):
        missed.append('临时单元还在（' + left + '）')
    return missed


def roll_back(old, mode):
    """换回旧单元、重新加载、再起服务。每一步都做，最后照实说现场是什么样。"""
    try:
        replace_with(UNIT, old, UNIT_OWNER, mode)
    except OSError as failure:
        return '换回旧的也没成（' + str(failure) + '）：现场是新单元、服务没起来，要人处理'
    reloaded = subprocess.run([SYSTEMCTL, 'daemon-reload']).returncode == 0
    subprocess.run([SYSTEMCTL, 'restart', SERVICE])
    if reloaded and came_up():
        return '已换回旧的，服务起来了'
    return ('已换回旧的，' + ('' if reloaded else '但 systemd 重新加载没成，') + '服务没起来（现在：'
            + state('is-active') + '）：要人处理')


def install_unit(raw, out):
    """换上新的单元文件。只换认得的上一版；先核新单元的检查现在能过、这台 systemd 真会一直再试，
    换上后服务没起来就换回去，换回之后的现场照实说。"""
    if digest(raw) != UNIT_SHA256:
        raise ValueError('送来的单元文件不是这份脚本认的那一版')
    status = os.lstat(str(UNIT))
    if not (stat.S_ISREG(status.st_mode) and (status.st_uid, status.st_gid) == UNIT_OWNER and status.st_nlink == 1
            and not status.st_mode & 0o022 and status.st_size <= UNIT_LIMIT):
        raise ValueError('现场的单元文件不是预期的样子')
    refuse_stale(UNIT)
    old = UNIT.read_bytes()
    if old == raw:
        # 上一次可能正好停在「文件换上了，还没重新加载、没重启」：这里补上。服务没在跑就起它，起不来照实报错。
        subprocess.run([SYSTEMCTL, 'daemon-reload'], check=True)
        if not (state('is-active') == 'active' and listening()):
            restart(out)
        out('nas_unit=already_current')
        return
    if digest(old) != UNIT_BEFORE_SHA256:
        # 不认识的内容不打出来：里面可能有别人加进去的东西（比如带口令的 Environment=）。
        raise ValueError('现场的单元文件不是仓库里的上一版（摘要 ' + digest(old) + '）。什么都没换。先在 NAS 上自己看一遍 '
                         + str(UNIT) + '，确认里面没有秘密，再把内容交给开发会话')
    if not ready_now(raw):
        raise ValueError('新单元的启动前检查现在就过不了，换上去服务起不来。什么都没换。')
    rehearse(out)
    try:
        replace_with(UNIT, raw, UNIT_OWNER, 0o644)
        subprocess.run([SYSTEMCTL, 'daemon-reload'], check=True)
        restart(out)
    except (ValueError, OSError, subprocess.CalledProcessError) as failure:
        if UNIT.read_bytes() == old:
            raise ValueError('新单元没换上（' + str(failure) + '）。什么都没换。')
        raise ValueError('新单元换上后服务没起来（' + str(failure) + '），' + roll_back(old, stat.S_IMODE(status.st_mode)))
    out('nas_unit=installed')


def main(argv, out=print):
    if os.geteuid() != 0 or len(argv) != 1 or argv[0] not in ('check', 'append', 'restart', 'unit'):
        raise ValueError('用法见文件开头；要经 sudo 跑')
    os.umask(0o077)
    if argv[0] == 'check':
        check(out)
    elif argv[0] == 'append':
        append(sys.stdin.buffer.read(256))
        out('nas_account=appended')
    elif argv[0] == 'unit':
        raw = sys.stdin.buffer.read(UNIT_LIMIT + 1)
        exclusively(lambda: install_unit(raw, out))
    else:
        restart(out)


def entry(argv):
    """把失败变成一句话和退出码 1；被打断时让收尾照常跑完。"""
    interruptible()
    try:
        main(argv)
    except (ValueError, OSError, subprocess.CalledProcessError) as failure:
        print('nas_account=failed：' + str(failure), file=sys.stderr)
    except KeyboardInterrupt:
        print('nas_account=interrupted：被打断。写到一半的文件、演练用的临时单元已经收走；'
              '单元要是已经换上，重跑 unit 会补上重新加载和重启', file=sys.stderr)
    else:
        return 0
    return 1


if __name__ == '__main__':
    sys.exit(entry(sys.argv[1:]))
