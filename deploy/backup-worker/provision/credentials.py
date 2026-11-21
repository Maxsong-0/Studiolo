#!/usr/bin/python3
"""169 上录入备份 worker 的三份凭据，再用 worker 账号把新仓库初始化好（README「安装前要有的东西」第 5 项）。

只由用户本人在自己的终端跑。口令只经不回显的提示输入：不进命令行参数、不进本脚本的输出、不进日志。
    python3 -I -B credentials.py                 预演：只核前提（不问口令、不写文件；对 NAS 只做一次不带凭据的 TLS 握手）
    sudo python3 -I -B credentials.py --apply    录入两个口令 → 写三份凭据文件 → 以 worker 账号初始化仓库并读回一次
先跑完同目录的 prereq.py（这里会把它的每一样产物只读地核一遍，缺一样就停下），并在 NAS 上加好账号（nas-account.command）。
已经写好的凭据不会被覆盖：三份都在、内容也合 worker 的规则，就跳过录入、直接做初始化和读回；
只有一部分、或内容 worker 不会认，就停下，由人看过再处理。
三份凭据要么都写好、要么一份都不留：先在旁边的临时目录（正式目录名加 .writing）里写齐，再一次换成正式的名字；
写到一半出错或被打断（Ctrl-C、终端断开）会把这一次写的全部删掉。被强杀、断电，或者收尾自己又出了错，才会留下那个临时目录
（正好死在把它换成正式名字的那一下，还会多留一个空的正式目录）。下次跑会先停下，说清留下了什么、怎么收拾；预演也会报。
不带 sudo 的预演看不进正式目录（root 0700）：临时目录它报得出，空的正式目录它只能提醒「可能有」，
带 sudo 跑预演（sudo python3 -I -B credentials.py，不带 --apply）才看得清。
"""
import getpass
import hashlib
import importlib.util
import os
from pathlib import Path
import socket
import ssl
import stat
import subprocess
import sys
import unicodedata

HERE = Path(__file__).resolve().parent
DIRECTORY = '/etc/studiolo/backup-worker-credentials'
# 写到一半用的临时目录：正式目录名后面加这一段（和正式目录在同一个父目录里，才能一次改名换上）。
WRITING = '.writing'
# 文件名和 worker 的三个槽位一一对应（crates/backup-worker/src/credentials/file.rs）。
NAMES = ('repository-password', 'rest-username', 'rest-password')
SECRET_LIMIT = 4096
REPOSITORY_PASSWORD_MINIMUM = 16
SERVICE = 'studiolo-backup-worker.service'
# 凭据目录和三份文件的属主：root（单元用 LoadCredential 从这里转交给 worker）。
FILE_OWNER = 0


def load(name):
    spec = importlib.util.spec_from_file_location(name, HERE / (name + '.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


prereq = load('prereq')
Refused = prereq.Refused
require = prereq.require


def validate_secret(value, minimum, what):
    """和 worker 读凭据时的规则一致：不空、不超长、首尾没有空白、没有控制字符；仓库口令另有最短长度。"""
    require(type(value) is str and minimum <= len(value) and len(value.encode()) <= SECRET_LIMIT, what + '太短或太长')
    require(value.strip() == value and not any(unicodedata.category(c) == 'Cc' for c in value),
            what + '首尾有空白或带控制字符')
    return value


def credentials_state(directory):
    """'absent' 没有；'complete' 三份都在且属主权限对；'partial' 其余情况（不覆盖，由人看）。"""
    try:
        status = os.lstat(directory)
    except FileNotFoundError:
        return 'absent'
    if not (stat.S_ISDIR(status.st_mode) and status.st_uid == FILE_OWNER
            and stat.S_IMODE(status.st_mode) == 0o700 and set(os.listdir(directory)) == set(NAMES)):
        return 'partial'
    for name in NAMES:
        entry = os.lstat(os.path.join(directory, name))
        if not (stat.S_ISREG(entry.st_mode) and entry.st_uid == FILE_OWNER and stat.S_IMODE(entry.st_mode) == 0o600
                and entry.st_nlink == 1 and 0 < entry.st_size <= SECRET_LIMIT + 1):
            return 'partial'
    return 'complete'


def read_credentials(directory):
    """读回三份凭据，按 worker 读它们时的规则核内容（crates/backup-worker/src/credentials/file.rs 的 parse）：
    只去掉结尾的一个换行，剩下的照 validate_secret；账号名还得是这个仓库路径开头的那个。
    为什么要核内容：restic 经 shell 读到的值会去掉结尾全部换行，多一个换行它照样连得上，worker 却会拒绝启动。
    不回显内容，不合规则只说是哪一份。"""
    values = {}
    for name in NAMES:
        descriptor = os.open(os.path.join(directory, name), os.O_RDONLY | os.O_NOFOLLOW)
        with os.fdopen(descriptor, 'rb') as stream:
            raw = stream.read(SECRET_LIMIT + 2)
        raw = raw[:-1] if raw.endswith(b'\n') else raw
        try:
            value = raw.decode('utf-8')
        except UnicodeDecodeError:
            raise Refused(name + ' 这一份不是 UTF-8 文本。不覆盖，先由人看过')
        values[name] = validate_secret(value, 1, name + ' 这一份的内容 worker 不会认（不覆盖，先由人看过）：')
    require(values['rest-username'] == prereq.REST_USERNAME,
            'rest-username 里不是 ' + prereq.REST_USERNAME + '（仓库路径以这个账号名开头）。不覆盖，先由人看过')
    return values


def sync_directory(path):
    descriptor = os.open(path, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def make_own(path, mine):
    """新建一个目录，并在 mine 里记下「这是这一次建的」。先记再建：反过来的话，建好之后、记下之前被打断，
    收尾就不知道该删它。建的时候发现已经在（别人的）才把这一笔划掉，那一个不动。"""
    mine.append(path)
    try:
        os.mkdir(path, 0o700)
    except FileExistsError:
        mine.remove(path)
        raise


def write_credentials(directory, values):
    """三份凭据要么都在、要么一份都不留。目录和三份文件都是新建（已存在就失败，空目录也不接着用），
    root 所有，目录 0700、文件 0600，内容后面带一个换行。
    先在旁边的临时目录里写齐、落盘；再建出正式的目录占住名字（已经有人建了就失败），把临时目录一次改名换上去：
    别人看到的正式目录要么是空的那一瞬，要么是完整的三份。中途出错或被打断，这一次建的东西全部删掉。"""
    if os.path.lexists(directory):
        raise FileExistsError(directory)
    scratch = directory + WRITING
    made, mine = [], []
    try:
        make_own(scratch, mine)
        for name in NAMES:
            path = os.path.join(scratch, name)
            # 临时目录是这一次新建的，里面的文件只可能是这一次写的：同样先记再建。
            made.append(path)
            descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
            with os.fdopen(descriptor, 'wb') as stream:
                stream.write(values[name].encode() + b'\n')
                stream.flush()
                os.fsync(stream.fileno())
        sync_directory(scratch)
        make_own(directory, mine)
        os.rename(scratch, directory)
    except BaseException:
        for path in made:
            try:
                os.unlink(path)
            except FileNotFoundError:
                pass
        for path in mine:
            try:
                # 只删得掉空目录：改名已经成功的话，正式目录里是完整的三份，这里删不动，留着。
                os.rmdir(path)
            except OSError:
                pass
        raise
    sync_directory(os.path.dirname(directory))


def directory_inside(path):
    """正式目录里面现在是哪一种：'empty' 是个真目录（不是链接），里面什么都没有；'unseen' 是个真目录，但这个身份看不了里面
    （正式目录是 root 0700，不带 sudo 的预演就是这样）；'other' 其余（不在、不是真目录、里面有东西）。
    看不了不能当成「不是空的」：那样不带 sudo 的预演会把上一次留下的空目录漏掉。"""
    try:
        if not stat.S_ISDIR(os.lstat(path).st_mode):
            return 'other'
    except OSError:
        return 'other'
    try:
        return 'other' if os.listdir(path) else 'empty'
    except PermissionError:
        return 'unseen'
    except OSError:
        return 'other'


# 不带 sudo 的预演看不进正式目录时接在后面的一句：是不是上一次留下的空目录说不准，照实说。
UNSEEN = ' 也在，这个身份看不了里面：要是空的，也是那一次留下的，--apply 还会停下叫你 sudo rmdir 它；带 sudo 跑预演看得清'


def leftover():
    """上一次录到一半被强杀或断电留下的东西：有就给一句「是什么、怎么收拾」，没有给 None。
    留下的是临时目录（里面可能有那一次输的口令）；正好死在「占住正式名字」和「把临时目录换上去」之间的话，
    还多一个空的正式目录。只删了临时目录，剩下的就是那个空目录：这两种都得说到，不然人要撞两回。
    正式目录在、里面看不了（不带 sudo 的预演）：临时目录也在就把这一层说上；只有它自己时这里不报
    （三份都写好的正常现场也是这个样子），由 main 在「凭据目录现在」那一行说明要带 sudo 才看得清。"""
    inside = directory_inside(DIRECTORY)
    if os.path.lexists(DIRECTORY + WRITING):
        also = '、sudo rmdir ' + DIRECTORY + '（空的，也是那一次留下的）' if inside == 'empty' else ''
        unseen = '。' + DIRECTORY + UNSEEN if inside == 'unseen' else ''
        return (DIRECTORY + WRITING + ' 还在：上一次录到一半被强杀或断电留下的，里面可能有那一次输的口令。'
                '确认没有别的 credentials.py 在跑，sudo rm -r 它' + also + '之后重跑' + unseen)
    if inside == 'empty':
        return (DIRECTORY + ' 已经在，但是空的：多半是上一次录到一半被强杀或断电留下的。'
                '确认没有别的 credentials.py 在跑，sudo rmdir 它之后重跑')
    return None


# 在 worker 账号下，把三份凭据接成 restic 认的环境变量再执行；口令只在这个子进程的环境里，不进任何命令行。
SHELL = ('set -eu; d="$CREDENTIALS_DIRECTORY"; '
         'RESTIC_PASSWORD_FILE="$d/repository-password"; export RESTIC_PASSWORD_FILE; '
         'RESTIC_REST_USERNAME="$(cat "$d/rest-username")"; export RESTIC_REST_USERNAME; '
         'RESTIC_REST_PASSWORD="$(cat "$d/rest-password")"; export RESTIC_REST_PASSWORD; '
         'exec "$@"')


def restic_command(*subcommand):
    """和正式的 worker 单元同一条链：专用账号、systemd 的 LoadCredential、批准的 CA、固定的 restic。"""
    properties = ['User=' + prereq.WORKER, 'Group=' + prereq.WORKER, 'UMask=0077', 'NoNewPrivileges=yes']
    properties += ['LoadCredential=' + name + ':' + DIRECTORY + '/' + name for name in NAMES]
    argv = ['systemd-run', '--quiet', '--wait', '--collect', '--pipe']
    for entry in properties:
        argv += ['-p', entry]
    return argv + ['/bin/sh', '-c', SHELL, 'sh', prereq.RESTIC, '--cacert', prereq.CA, '--no-cache',
                   '--repo', prereq.REPOSITORY] + list(subcommand)


def endpoint():
    """不带凭据连一次：证书要能用批准的那张 CA 验过，而且就是那一张（自签，服务端证书和 CA 是同一张）。"""
    address = prereq.REPOSITORY[len('rest:https://'):].split('/')[0]
    host, port = address.rsplit(':', 1)
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
    context.load_verify_locations(prereq.CA)
    with socket.create_connection((host, int(port)), timeout=10) as raw:
        with context.wrap_socket(raw, server_hostname=host) as secured:
            served = hashlib.sha256(secured.getpeercert(binary_form=True)).hexdigest()
            version = secured.version()
    approved = hashlib.sha256(ssl.PEM_cert_to_DER_cert(Path(prereq.CA).read_text(encoding='ascii'))).hexdigest()
    require(served == approved, 'NAS 现在用的证书不是批准的那一张（指纹 ' + served + '）')
    return version


def preconditions(out):
    require(sys.platform == 'linux', '只在 169 上跑')
    # prereq.py 的每一样产物（账号和组、目录、程序和证书、两份配置的内容、三个单元）都得已经在、而且对，
    # 三个单元的语法核对也得过（它停在那一步的话，文件都已经放好了）：它停在半路，这里不录口令、不初始化仓库。
    count = prereq.settled()
    out('[已核] prereq.py 的 ' + str(count) + ' 样产物都在、都对（账号和组、目录、程序和证书、两份配置、三个单元），单元的语法核对也过了')
    try:
        out('[已核] NAS 备份服务连得上（' + endpoint() + '），证书就是批准的那一张')
    except (OSError, ssl.SSLError) as failure:
        raise Refused('NAS 的备份服务连不上或证书验不过（' + type(failure).__name__ + '：' + str(failure)[:120]
                      + '）。先在 NAS 那边看 nas-account.command check')


def prompt(ask=getpass.getpass):
    rest = validate_secret(ask('NAS 上 ' + prereq.REST_USERNAME + ' 这个账号的密码（刚在 NAS 那一步设的）：'), 1, '账号密码')
    repository = validate_secret(ask('给新仓库定一个加密口令（至少 16 位；先存进密码管理器，丢了这个仓库就读不回来）：'),
                                 REPOSITORY_PASSWORD_MINIMUM, '仓库口令')
    require(repository == ask('再输一遍仓库口令：'), '两次输入的仓库口令不一样')
    require(rest != repository, '仓库口令不能和 NAS 账号密码一样')
    return {'repository-password': repository, 'rest-username': prereq.REST_USERNAME, 'rest-password': rest}


def apply(out, run=subprocess.run, ask=getpass.getpass):
    require(os.geteuid() == 0, '--apply 要用 sudo 跑')
    require(all(stream.isatty() for stream in (sys.stdin, sys.stdout, sys.stderr)), '要在用户自己的终端里跑')
    require(run(['systemctl', 'is-active', '--quiet', SERVICE]).returncode != 0, 'worker 正在跑：先停掉再动凭据')
    os.umask(0o077)
    # 先看有没有上一次留下的东西，再问口令：有的话问了也白问。
    stale = leftover()
    require(stale is None, stale)
    state = credentials_state(DIRECTORY)
    require(state != 'partial', DIRECTORY + ' 已经在，但不是完整的三份 root 0600 文件。不覆盖，先由人看过')
    if state == 'complete':
        read_credentials(DIRECTORY)
        out('[已有] 三份凭据文件已在，内容合 worker 的规则，不重新录入')
    else:
        write_credentials(DIRECTORY, prompt(ask))
        require(credentials_state(DIRECTORY) == 'complete', '凭据文件写完之后核对不过')
        read_credentials(DIRECTORY)
        out('[做完] 三份凭据文件已写好（root，目录 0700、文件 0600）')
    out('以 ' + prereq.WORKER + ' 账号初始化仓库 ' + prereq.REPOSITORY + ' ……')
    if run(restic_command('init')).returncode != 0:
        # 没成功有两种：仓库早就初始化过（那就看这套凭据打不打得开），或者真的失败。
        require(run(restic_command('cat', 'config'), stdout=subprocess.DEVNULL).returncode == 0,
                '仓库初始化没成功，上面是 restic 的原话（里面没有口令）。要是 NAS 账号密码输错了：'
                '确认 worker 没在跑，sudo rm -r ' + DIRECTORY + ' 之后重跑本脚本')
        out('[已有] 仓库早就初始化过，这套凭据打得开，不重复初始化')
    require(run(restic_command('snapshots', '--json'), stdout=subprocess.DEVNULL).returncode == 0,
            '仓库初始化之后读不回快照清单')
    out('[已核] 用 worker 账号和这套凭据读得到仓库')
    out('backup_credentials=ready')


def main(argv, out=print):
    require(argv in ([], ['--apply']), __doc__)
    preconditions(out)
    if argv:
        apply(out)
        return
    if os.geteuid() == 0:
        out('凭据目录现在：' + {'absent': '还没有', 'complete': '三份都在', 'partial': '在，但不完整（要人看）'}[
            credentials_state(DIRECTORY)])
    else:
        # 不带 sudo 看不进正式目录（root 0700）：三份写好的和上一次留下的空目录在这里是一个样子，照实说，并说怎么看清。
        out('凭据目录现在：' + ('还没有' if not os.path.lexists(DIRECTORY) else
                          '已经在（里面要 root 才看得到：三份齐不齐、是不是上一次留下的空目录，带 sudo 跑预演才看得清）'))
    stale = leftover()
    if stale is not None:
        out('--apply 会先停在这一条上：' + stale)
    out('预演结束：没问口令、没写任何东西。带 sudo 和 --apply 再跑一遍才录入。')


def entry(argv):
    """把「停下」变成一句话和退出码 1；被打断时让收尾照常跑完。报错里只有路径和错误类别，没有口令。"""
    prereq.interruptible()
    try:
        main(argv)
    except Refused as refusal:
        print('停下：' + str(refusal), file=sys.stderr)
    except OSError as failure:
        print('停下：' + type(failure).__name__ + '：' + str(failure), file=sys.stderr)
    except KeyboardInterrupt:
        print('\n停下：被打断。三份凭据要么都写好了、要么一份都没留；重跑会从现状接着做（写好的不会再问）。', file=sys.stderr)
    else:
        return 0
    return 1


if __name__ == '__main__':
    sys.exit(entry(sys.argv[1:]))
