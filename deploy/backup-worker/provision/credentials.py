#!/usr/bin/python3
"""169 上录入备份 worker 的三份凭据，再用 worker 账号把新仓库初始化好（README「安装前要有的东西」第 5 项）。

只由用户本人在自己的终端跑。口令只经不回显的提示输入：不进命令行参数、不进本脚本的输出、不进日志。
    python3 -I -B credentials.py                 预演：只核前提（不问口令、不写文件；对 NAS 只做一次不带凭据的 TLS 握手）
    sudo python3 -I -B credentials.py --apply    录入两个口令 → 写三份凭据文件 → 以 worker 账号初始化仓库并读回一次
先跑完同目录的 prereq.py，并在 NAS 上加好账号（nas-account.command）。已经写好的凭据不会被覆盖：
三份都在、内容也合 worker 的规则，就跳过录入、直接做初始化和读回；只有一部分、或内容 worker 不会认，就停下，由人看过再处理。
"""
import getpass
import hashlib
import importlib.util
import os
from pathlib import Path
import pwd
import socket
import ssl
import stat
import subprocess
import sys
import unicodedata

HERE = Path(__file__).resolve().parent
DIRECTORY = '/etc/studiolo/backup-worker-credentials'
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


site = load('prereq')
Refused = site.Refused
require = site.require


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
    require(values['rest-username'] == site.REST_USERNAME,
            'rest-username 里不是 ' + site.REST_USERNAME + '（仓库路径以这个账号名开头）。不覆盖，先由人看过')
    return values


def write_credentials(directory, values):
    """目录和三份文件都是新建（已存在就失败），root 所有，目录 0700、文件 0600，内容后面带一个换行。"""
    os.mkdir(directory, 0o700)
    for name in NAMES:
        descriptor = os.open(os.path.join(directory, name),
                             os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
        with os.fdopen(descriptor, 'wb') as stream:
            stream.write(values[name].encode() + b'\n')
            stream.flush()
            os.fsync(stream.fileno())
    descriptor = os.open(directory, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


# 在 worker 账号下，把三份凭据接成 restic 认的环境变量再执行；口令只在这个子进程的环境里，不进任何命令行。
SHELL = ('set -eu; d="$CREDENTIALS_DIRECTORY"; '
         'RESTIC_PASSWORD_FILE="$d/repository-password"; export RESTIC_PASSWORD_FILE; '
         'RESTIC_REST_USERNAME="$(cat "$d/rest-username")"; export RESTIC_REST_USERNAME; '
         'RESTIC_REST_PASSWORD="$(cat "$d/rest-password")"; export RESTIC_REST_PASSWORD; '
         'exec "$@"')


def restic_command(*subcommand):
    """和正式的 worker 单元同一条链：专用账号、systemd 的 LoadCredential、批准的 CA、固定的 restic。"""
    properties = ['User=' + site.WORKER, 'Group=' + site.WORKER, 'UMask=0077', 'NoNewPrivileges=yes']
    properties += ['LoadCredential=' + name + ':' + DIRECTORY + '/' + name for name in NAMES]
    argv = ['systemd-run', '--quiet', '--wait', '--collect', '--pipe']
    for entry in properties:
        argv += ['-p', entry]
    return argv + ['/bin/sh', '-c', SHELL, 'sh', site.RESTIC, '--cacert', site.CA, '--no-cache',
                   '--repo', site.REPOSITORY] + list(subcommand)


def endpoint():
    """不带凭据连一次：证书要能用批准的那张 CA 验过，而且就是那一张（自签，服务端证书和 CA 是同一张）。"""
    address = site.REPOSITORY[len('rest:https://'):].split('/')[0]
    host, port = address.rsplit(':', 1)
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
    context.load_verify_locations(site.CA)
    with socket.create_connection((host, int(port)), timeout=10) as raw:
        with context.wrap_socket(raw, server_hostname=host) as secured:
            served = hashlib.sha256(secured.getpeercert(binary_form=True)).hexdigest()
            version = secured.version()
    approved = hashlib.sha256(ssl.PEM_cert_to_DER_cert(Path(site.CA).read_text(encoding='ascii'))).hexdigest()
    require(served == approved, 'NAS 现在用的证书不是批准的那一张（指纹 ' + served + '）')
    return version


def preconditions(out):
    require(sys.platform == 'linux', '只在 169 上跑')
    require(site.has(site.RESTIC, stat.S_ISREG, 0, 0, (0o555, 0o755, 0o500)) and site.sha(site.RESTIC) == site.RESTIC_SHA,
            '固定的 restic 不在或摘要不对：先跑 prereq.py --apply')
    require(site.has(site.CA, stat.S_ISREG, 0, 0, (0o644, 0o444)) and site.sha(site.CA) == site.CA_SHA,
            'NAS 证书不在或摘要不对：先跑 prereq.py --apply')
    require(site.has(site.WORKER_CONFIG, stat.S_ISREG, 0, 0, (0o644, 0o444)), 'worker 配置不在：先跑 prereq.py --apply')
    try:
        pwd.getpwnam(site.WORKER)
    except KeyError:
        raise Refused('专用账号 ' + site.WORKER + ' 不在：先跑 prereq.py --apply')
    out('[已核] restic、NAS 证书、worker 配置、专用账号都在')
    try:
        out('[已核] NAS 备份服务连得上（' + endpoint() + '），证书就是批准的那一张')
    except (OSError, ssl.SSLError) as failure:
        raise Refused('NAS 的备份服务连不上或证书验不过（' + type(failure).__name__ + '：' + str(failure)[:120]
                      + '）。先在 NAS 那边看 nas-account.command check')


def prompt(ask=getpass.getpass):
    rest = validate_secret(ask('NAS 上 ' + site.REST_USERNAME + ' 这个账号的密码（刚在 NAS 那一步设的）：'), 1, '账号密码')
    repository = validate_secret(ask('给新仓库定一个加密口令（至少 16 位；先存进密码管理器，丢了这个仓库就读不回来）：'),
                                 REPOSITORY_PASSWORD_MINIMUM, '仓库口令')
    require(repository == ask('再输一遍仓库口令：'), '两次输入的仓库口令不一样')
    require(rest != repository, '仓库口令不能和 NAS 账号密码一样')
    return {'repository-password': repository, 'rest-username': site.REST_USERNAME, 'rest-password': rest}


def apply(out, run=subprocess.run, ask=getpass.getpass):
    require(os.geteuid() == 0, '--apply 要用 sudo 跑')
    require(all(stream.isatty() for stream in (sys.stdin, sys.stdout, sys.stderr)), '要在用户自己的终端里跑')
    require(run(['systemctl', 'is-active', '--quiet', SERVICE]).returncode != 0, 'worker 正在跑：先停掉再动凭据')
    os.umask(0o077)
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
    out('以 ' + site.WORKER + ' 账号初始化仓库 ' + site.REPOSITORY + ' ……')
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
        out('凭据目录现在：' + ('还没有' if not os.path.lexists(DIRECTORY) else '已经在（里面要 root 才看得到）'))
    out('预演结束：没问口令、没写任何东西。带 sudo 和 --apply 再跑一遍才录入。')


if __name__ == '__main__':
    try:
        main(sys.argv[1:])
    except Refused as refusal:
        print('停下：' + str(refusal), file=sys.stderr)
        sys.exit(1)
    except KeyboardInterrupt:
        print('\n停下：被打断。已经写下的文件没有回退，重跑会从现状接着做。', file=sys.stderr)
        sys.exit(1)
