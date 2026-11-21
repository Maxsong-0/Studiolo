#!/usr/bin/python3
"""169 上启用备份 worker 之前、不涉及秘密的那几样东西（README「安装前要有的东西」第 1～4、6 项和三个单元）。

用法（在 169 上以主服务账号跑；不要用 sudo 起这个脚本，要 root 的步骤它自己逐条经 sudo 执行）：
    python3 -I -B prereq.py            预演：只读。逐项说现在怎样、会做什么，把要写的两份配置原样打出来
    python3 -I -B prereq.py --apply    照做：每一步做完当场核对，不对就停
重复跑是安全的：已经对的跳过，在但不对的停下（不覆盖、不删除），缺的才建。
要 root 放的文件由本脚本的 --place 入口放（只由本脚本自己经 sudo 调用）：只新建，目标已经在就失败；
内容和调用方核过的那一份不一样就不放。被打断（Ctrl-C、终端断开）时正在放的那一份不留半个文件。
不碰凭据（见同目录 credentials.py），不启用、不启动任何单元，不动人侧服务。

「发行相关的值」那一段跟着人侧发行走：候选重出、worker 重建，都要改这里再重跑。
"""
import grp
import hashlib
import io
import json
import os
from pathlib import Path
import pwd
import signal
import stat
import subprocess
import sys

HERE = Path(__file__).resolve().parent
TEMPLATES = HERE.parent / 'systemd'

# ---------------------------------------------------------------- 169 的固定事实
OWNER = 'maxsong'
OWNER_UID = 1000
WORKER = 'studiolo-backup'
# 专用账号建的时候给的家目录和 shell（没有家目录、不能登录）；已有的账号也得是这个样子才算对。
WORKER_HOME = '/nonexistent'
WORKER_SHELL = '/usr/sbin/nologin'
READERS = 'studiolo-backup-read'
# useradd、groupadd 的 --system 从这里取系统账号和系统组的号码范围。
LOGIN_DEFINITIONS = '/etc/login.defs'
RUNTIME = '/run/studiolo-backup'
STAGING = '/var/lib/studiolo-backup-staging'
SOCKET = RUNTIME + '/producer.sock'
EPHEMERAL = '/home/maxsong/.local/share/studiolo-backup-ephemeral'
BACKUP_CONFIG = '/home/maxsong/.config/studiolo-human/backup.json'
# 主人配置里的 Auth／Audit 私有目录：备份的三条路径不能和它们互相嵌套（crates/host-config/src/backup/file.rs）。
PRIVATE = ('/home/maxsong/.local/share/studiolo-human-auth', '/home/maxsong/.local/share/studiolo-human-audit')
PROGRAMS = '/usr/lib/studiolo-backup-worker'
WORKER_BINARY = PROGRAMS + '/studiolo-backup-worker'
RESTIC = PROGRAMS + '/restic-0.19.1'
WORKER_CONFIG = '/etc/studiolo/backup-worker.json'
CA = '/etc/studiolo/backup-worker-ca.pem'
TMPFILES = '/etc/tmpfiles.d/studiolo-backup.conf'
UNIT_DIRECTORY = '/etc/systemd/system'
UNITS = ('studiolo-backup-worker.service', 'studiolo-backup-schedule.service', 'studiolo-backup-schedule.timer')
# 要 root 放的文件和各自的权限：--place 只认这几个目标。
PLACED = {TMPFILES: 0o644, WORKER_BINARY: 0o755, RESTIC: 0o555, CA: 0o644, WORKER_CONFIG: 0o644}
PLACED.update({UNIT_DIRECTORY + '/' + name: 0o644 for name in UNITS})
PLACED_OWNER = (0, 0)
# 账号还没建时预演用的占位号码；给程序真实解析读的两份夹具也用这一组
# （crates/host-config/tests/support/provisioned-backup.json、crates/backup-worker/tests/support/provisioned-config.json）。
PLACEHOLDER_NUMBERS = (64001, 64002)
# NAS 的 rest-server 开着 --private-repos：仓库路径必须以账号名开头。
REST_USERNAME = 'studiolo-backup-worker'
REPOSITORY = 'rest:https://192.168.100.111:18443/' + REST_USERNAME + '/main/'
# 2026-11-18 用户拍板（T-216 Q2）：10 分钟一次、完成预算 5 分钟、RPO 15 分钟、冻结预算 30 秒、保留最近 30 加每日 14。
INTERVAL_MILLIS, COMPLETION_MILLIS, RPO_MILLIS, FREEZE_MILLIS = 600_000, 300_000, 900_000, 30_000
KEEP_LAST, KEEP_DAILY = 30, 14
# 封存副本只在到期后回收：保留期乘上数据量就是暂存区的盘占用（169 上每份约 1.1G，一小时约 8G）。
RETENTION_MILLIS = 3_600_000

# ---------------------------------------------------------------- 发行相关的值（跟着 r17 候选和 108 的构建走）
SOURCE_COMMIT = '2df75ca'
HUMAN_RELEASE = 'm5m6-board-20260920-r17'
RELEASE_DIRECTORY = '/home/maxsong/.local/share/studiolo-human-releases'
PACKAGES = (
    ('studiolo-local-server', 'adb868bb52f79f924f5392c3020f80434c31f67f55978d43450d223460979286'),
    ('studiolo-maintain', '38354e0081a52e704cafd3c4d7d3f55044bddc6f198d331b136745d24deb7652'),
    ('studiolo-backup-worker', '84945a8a0f1ea068fa9a86c1265a165281c3eb7986e934705c1982f2ce74d37d'),
)
RESTIC_SHA = '20d4142678d0d95ec11a4759def1b73fd9190abc9ca19e4b62d067c0b387e639'
CA_SHA = '4577693009b4c680420b7ff895970a4e8b2838e451cad0bb8a0dfe7c64e8c269'
# 要拷的三样现在放在哪（都在 169 上）：108 构建的 worker、P3 时装的 restic、P3 时取回的 NAS 公开证书。
WORKER_SOURCE = '/home/maxsong/studiolo-t216-r17-20261120-2df75cac-worker/studiolo-backup-worker'
RESTIC_SOURCE = '/usr/local/bin/studiolo-restic'
CA_SOURCE = '/home/maxsong/.config/studiolo-backup/ca.pem'


class Refused(Exception):
    """现场和预期不符：停下，不覆盖。"""


def require(condition, message):
    if not condition:
        raise Refused(message)


class Interrupted(KeyboardInterrupt):
    """收到 SIGHUP、SIGTERM（终端断开、被要求结束）：和 Ctrl-C 一样当成被打断，让 finally 里的收尾照常跑。"""


def interruptible():
    """这几个信号的默认动作是直接结束进程，finally 不跑：做到一半的临时文件、临时目录会留下。
    改成抛异常；收到第一个之后其余的都不再理，免得收尾做到一半又被打断。"""
    watched = (signal.SIGHUP, signal.SIGTERM, signal.SIGINT)

    def stop(number, frame):
        for other in watched:
            signal.signal(other, signal.SIG_IGN)
        raise Interrupted(signal.Signals(number).name)
    for number in watched:
        signal.signal(number, stop)


# ---------------------------------------------------------------- 纯逻辑：两份配置和它们的取值规则
def worker_document(worker_uid, reader_gid):
    return {
        'schema': 1, 'worker_uid': worker_uid, 'producer_uid': OWNER_UID, 'reader_gid': reader_gid,
        'sealed_parent': STAGING, 'producer_socket': SOCKET, 'repository': REPOSITORY,
        'restic_sha256': RESTIC_SHA, 'repository_ca_sha256': CA_SHA,
    }


def backup_document(worker_uid, reader_gid):
    return {
        'version': 1,
        'backup': {
            'version': 1, 'target_shape': 'dedicated_append_only_target',
            # 主人的声明：等用新账号验过「删除、覆盖被拒」再改成 true。
            'claims_immutability': False,
            'restic_package': 'restic', 'restic_version': '0.19.1',
            'schedule_interval_millis': INTERVAL_MILLIS, 'completion_budget_millis': COMPLETION_MILLIS,
            'rpo_millis': RPO_MILLIS, 'keep_last': KEEP_LAST, 'keep_daily': KEEP_DAILY,
            'credentials': [
                {'logical_name': 'restic_repository_password', 'environment_variable': 'RESTIC_PASSWORD_FILE'},
                {'logical_name': 'restic_rest_username', 'environment_variable': 'RESTIC_REST_USERNAME'},
                {'logical_name': 'restic_rest_password', 'environment_variable': 'RESTIC_REST_PASSWORD'},
            ],
        },
        'backup_producer': {
            'version': 1, 'worker_uid': worker_uid, 'reader_gid': reader_gid,
            'socket': SOCKET, 'staging_parent': STAGING, 'ephemeral_root': EPHEMERAL,
            'source_retention_millis': RETENTION_MILLIS, 'freeze_budget_millis': FREEZE_MILLIS,
            'application_packages': [{'name': name, 'version': SOURCE_COMMIT, 'digest': digest}
                                     for name, digest in PACKAGES],
            'restic_digest': RESTIC_SHA,
            'deployment': [{'unit': 'studiolo-human.service', 'revision': HUMAN_RELEASE},
                           {'unit': 'studiolo-backup-worker.service', 'revision': SOURCE_COMMIT}],
        },
    }


def encode(document):
    return (json.dumps(document, ensure_ascii=True, indent=1) + '\n').encode()


def is_digest(value):
    return type(value) is str and len(value) == 64 and all(c in '0123456789abcdef' for c in value)


def is_label(value):
    return (type(value) is str and 0 < len(value) <= 64
            and all(c.isascii() and (c.isalnum() or c in '._-@') for c in value))


def is_account_number(value):
    return type(value) is int and 0 < value < 0xFFFF_FFFF


def normal_path(value):
    """绝对、没有 `.`、`..`、重复或结尾的分隔符，而且不是根。"""
    return (type(value) is str and value.startswith('/') and value != '/'
            and all(part not in ('', '.', '..') for part in value[1:].split('/')))


def nested(left, right):
    left, right = Path(left), Path(right)
    return left == right or left in right.parents or right in left.parents


def check_backup_document(document):
    """照抄 crates/host-config/src/backup/{file,document,producer}.rs 的取值规则；激活预检不核这些，
    写错了要到停服之后启动失败才知道，所以生成时先核一遍。"""
    require(set(document) == {'version', 'backup', 'backup_producer'} and document['version'] == 1, '顶层键或版本不对')
    approval, producer = document['backup'], document['backup_producer']
    require(set(approval) == {
        'version', 'target_shape', 'claims_immutability', 'restic_package', 'restic_version',
        'schedule_interval_millis', 'completion_budget_millis', 'rpo_millis', 'keep_last', 'keep_daily',
        'credentials'}, '审批段的键不对')
    require(approval['version'] == 1 and approval['target_shape'] == 'dedicated_append_only_target', '审批段版本或目标形态不对')
    require(type(approval['claims_immutability']) is bool, 'claims_immutability 不是布尔值')
    version = approval['restic_version']
    require(approval['restic_package'] == 'restic' and type(version) is str and 0 < len(version) <= 32
            and all(c in '0123456789.' for c in version), 'restic 包名或版本不对')
    interval, completion, rpo = (approval[key] for key in (
        'schedule_interval_millis', 'completion_budget_millis', 'rpo_millis'))
    require(all(type(value) is int and value > 0 for value in (interval, completion, rpo))
            and interval + completion <= rpo, '间隔加完成预算超过了 RPO')
    require(all(type(approval[key]) is int and approval[key] > 0 for key in ('keep_last', 'keep_daily')), '保留个数不对')
    credentials = approval['credentials']
    require(type(credentials) is list and 0 < len(credentials) <= 8, '凭据引用条数不对')
    for entry in credentials:
        require(set(entry) == {'logical_name', 'environment_variable'}, '凭据引用的键不对')
        name, variable = entry['logical_name'], entry['environment_variable']
        require(type(name) is str and 0 < len(name) <= 64
                and all(c in 'abcdefghijklmnopqrstuvwxyz0123456789_' for c in name), '凭据逻辑名不对')
        require(type(variable) is str and 0 < len(variable) <= 64 and not variable[0].isdigit()
                and all(c in 'ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_' for c in variable), '凭据变量名不对')
    require(set(producer) == {
        'version', 'worker_uid', 'reader_gid', 'socket', 'staging_parent', 'ephemeral_root',
        'source_retention_millis', 'freeze_budget_millis', 'application_packages', 'restic_digest',
        'deployment'}, '部署段的键不对')
    require(producer['version'] == 1 and is_account_number(producer['worker_uid'])
            and is_account_number(producer['reader_gid']), '部署段版本或账号号码不对')
    require(producer['worker_uid'] != OWNER_UID, 'worker 账号不能就是主服务账号')
    require(type(producer['source_retention_millis']) is int
            and 600_000 <= producer['source_retention_millis'] <= 86_400_000, '封存保留期不在 10 分钟到 24 小时之间')
    require(type(producer['freeze_budget_millis']) is int
            and 1_000 <= producer['freeze_budget_millis'] <= 120_000, '冻结预算不在 1 秒到 120 秒之间')
    require(is_digest(producer['restic_digest']), 'restic 摘要不对')
    paths = [producer[key] for key in ('socket', 'staging_parent', 'ephemeral_root')]
    require(all(normal_path(path) for path in paths) and len(paths[0].encode()) <= 100, '三条路径不规范或 socket 路径太长')
    for index, left in enumerate(paths):
        require(not any(nested(left, right) for right in paths[index + 1:] + list(PRIVATE)), '路径互相嵌套')
    for key, fields in (('application_packages', ('name', 'version', 'digest')), ('deployment', ('unit', 'revision'))):
        entries = producer[key]
        require(type(entries) is list and 0 < len(entries) <= 16, key + ' 条数不对')
        for entry in entries:
            require(set(entry) == set(fields), key + ' 的键不对')
            require(all(is_digest(entry[field]) if field == 'digest' else is_label(entry[field]) for field in fields),
                    key + ' 的取值不对')


def check_worker_document(document, backup):
    """照抄 crates/backup-worker/src/config.rs 的取值规则，并核它和 backup.json 说的是同一组东西。"""
    require(set(document) == {
        'schema', 'worker_uid', 'producer_uid', 'reader_gid', 'sealed_parent', 'producer_socket', 'repository',
        'restic_sha256', 'repository_ca_sha256'}, 'worker 配置的键不对')
    require(document['schema'] == 1 and is_account_number(document['worker_uid'])
            and is_account_number(document['producer_uid']) and is_account_number(document['reader_gid'])
            and document['worker_uid'] != document['producer_uid'], 'worker 配置的版本或账号号码不对')
    sealed, socket = document['sealed_parent'], document['producer_socket']
    require(normal_path(sealed) and normal_path(socket) and len(socket.encode()) <= 100, 'worker 配置的路径不对')
    for protected in ('/var/lib/studiolo-backup-worker', '/run/studiolo-backup-worker', '/run/credentials'):
        require(not nested(sealed, protected) and not nested(socket, protected), '路径落在 worker 自己的目录里')
    require(not nested(sealed, socket), '封存父目录和 socket 互相嵌套')
    require(is_digest(document['restic_sha256']) and is_digest(document['repository_ca_sha256']), 'worker 配置的摘要不对')
    repository = document['repository']
    prefix = 'rest:https://'
    require(type(repository) is str and repository.startswith(prefix) and len(repository) <= 512
            and repository == repository.lower() and all(c.isascii() and (c.isalnum() or c in ':/.-_') for c in repository)
            and '..' not in repository and '//' not in repository[len(prefix):]
            and repository.endswith('/') and not repository[len(prefix):].split('/')[0].endswith(':443'),
            '仓库地址不是规范的 rest:https:// 写法')
    producer = backup['backup_producer']
    require((document['worker_uid'], document['reader_gid'], sealed, socket, document['restic_sha256'])
            == (producer['worker_uid'], producer['reader_gid'], producer['staging_parent'], producer['socket'],
                producer['restic_digest']) and document['producer_uid'] == OWNER_UID, '两份配置对不上')


def tmpfiles_text():
    """仓库里的片段把主服务账号写成示例名 studiolo：换成 169 的实际账号，别的不动。"""
    template = (TEMPLATES / 'studiolo-backup.tmpfiles.conf').read_text(encoding='utf-8')
    lines = [line for line in template.splitlines() if line.startswith('d ')]
    expected = ['d ' + path + ' 0750 studiolo ' + READERS + ' -' for path in (RUNTIME, STAGING)]
    require(lines == expected, '仓库里的 tmpfiles 片段不是预期的两行')
    return ''.join('d ' + path + ' 0750 ' + OWNER + ' ' + READERS + ' -\n' for path in (RUNTIME, STAGING))


# ---------------------------------------------------------------- 现场：只读的查看
def sha(path):
    digest = hashlib.sha256()
    with open(path, 'rb') as stream:
        for block in iter(lambda: stream.read(1 << 20), b''):
            digest.update(block)
    return digest.hexdigest()


def content(path):
    return Path(path).read_bytes()


def matches(path, digest):
    """在、是普通文件、摘要对。"""
    return os.path.isfile(path) and sha(path) == digest


def reading(path):
    return open(path, 'rb')


def make_directory(path, mode):
    os.mkdir(path, mode)


def account(name):
    try:
        return pwd.getpwnam(name)
    except KeyError:
        return None


def group(name):
    try:
        return grp.getgrnam(name)
    except KeyError:
        return None


def group_numbered(gid):
    """按号码找组；账号的主组号在组表里查不到时给 None（不让 KeyError 变成一屏回溯）。"""
    try:
        return grp.getgrgid(gid)
    except KeyError:
        return None


def groups_of(name, gid):
    return os.getgrouplist(name, gid)


def system_range(kind, path=None):
    """系统账号（kind 是 'UID'）或系统组（'GID'）的号码范围，和 useradd、groupadd 的 --system 取法一致：
    login.defs 里的 SYS_<kind>_MIN、SYS_<kind>_MAX；没写就用 shadow 的默认值（101 到 <kind>_MIN - 1，<kind>_MIN 默认 1000）。"""
    found = {}
    try:
        with open(path or LOGIN_DEFINITIONS, encoding='utf-8', errors='replace') as stream:
            for line in stream:
                words = line.split('#', 1)[0].split()
                if len(words) == 2 and words[1].isdigit():
                    found[words[0]] = int(words[1])
    except FileNotFoundError:
        pass
    return found.get('SYS_' + kind + '_MIN', 101), found.get('SYS_' + kind + '_MAX', found.get(kind + '_MIN', 1000) - 1)


def has(path, kind, uid, gid, modes):
    """路径在不在、是不是预期的样子：None 不在，True 对，False 在但不对。"""
    try:
        status = os.lstat(path)
    except FileNotFoundError:
        return None
    return (kind(status.st_mode) and (status.st_uid, status.st_gid) == (uid, gid)
            and stat.S_IMODE(status.st_mode) in modes and (stat.S_ISDIR(status.st_mode) or status.st_nlink == 1))


def chain(path, uid, top='/'):
    """path 上面的每一级目录都得是真目录（不是链接）、属主是 root 或 uid、组和其他人不可写。
    和用它们的一方逐级核的规则一致：激活预检（deploy/human/release/release_common.py 的 ancestors）、
    人侧读私有配置（crates/host-config/src/config_file.rs）、worker 打开 /etc/studiolo 和程序目录。
    只看最后一级的话，中间有一级是链接也能置备成功，要到停服之后才被拒。top 只给用例用。"""
    for node in reversed(Path(path).parents):
        if not node.is_relative_to(top):
            continue
        try:
            status = os.lstat(node)
        except FileNotFoundError:
            raise Refused(str(node) + ' 不在')
        require(stat.S_ISDIR(status.st_mode) and status.st_uid in (0, uid) and stat.S_IMODE(status.st_mode) & 0o7022 == 0,
                str(node) + ' 不是真目录（路径上不能有链接），或者属主不对、组和其他人可写')


def place(destination, mode, stream, owner, digest):
    """把 stream 的内容放成一个新文件。目标已经在（哪怕是个悬空链接）就失败，不覆盖；中途失败不留半个文件；
    内容的 sha256 不是 digest 就不放：调用方核过的是哪一份，放上去的就得是哪一份。
    先在同一个目录里写好临时文件，再用硬链接起正式的名字（名字已在时链接失败），最后删掉临时名。
    临时名是固定的：上一次被强杀留下的、或者另一处正在放的，这里会撞上并停下，不会悄悄攒下一堆。
    owner 给 (uid, gid) 就改属主（root 一侧放文件），给 None 就保持当前账号（主服务账号写自己的私有配置）。"""
    directory, name = os.path.split(destination)
    scratch = os.path.join(directory, '.' + name + '.placing')
    # 先记下「临时名是这一次建的」再去建：反过来的话，建好之后、记下之前被打断，收尾就不知道该删它。
    # 建的时候发现已经在（别人的）才改回去，那一份不动。
    mine = True
    try:
        try:
            descriptor = os.open(scratch, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
        except FileExistsError:
            mine = False
            raise Refused(scratch + ' 已经在：上一次放到一半被强杀留下的，或者另一处正在放。'
                          '确认没有别的 prereq.py 在跑，删掉它再重跑')
        seen = hashlib.sha256()
        with os.fdopen(descriptor, 'wb') as out:
            for block in iter(lambda: stream.read(1 << 20), b''):
                seen.update(block)
                out.write(block)
            if owner is not None:
                os.fchown(out.fileno(), *owner)
            os.fchmod(out.fileno(), mode)
            out.flush()
            os.fsync(out.fileno())
        require(seen.hexdigest() == digest, destination + ' 要放的内容和核过的那一份不一样（来源在核对之后被换了）。什么都没放')
        os.link(scratch, destination, follow_symlinks=False)
    finally:
        if mine:
            try:
                os.unlink(scratch)
            except FileNotFoundError:
                pass
    descriptor = os.open(directory, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


# ---------------------------------------------------------------- 现场：会改东西的步骤
class Provision:
    """apply：照做还是只预演。audit：只读地核「每一样产物都已经在、而且对」，缺一样就停下（见 settled）。"""

    def __init__(self, apply, run=subprocess.run, out=print, audit=False):
        self.apply, self.run, self.out, self.audit = apply, run, out, audit
        self.pending = self.settled = 0

    def root(self, *argv):
        self.out('    sudo ' + ' '.join(argv))
        if self.apply:
            self.run(['sudo', '--'] + list(argv), check=True)

    def step(self, title, state, action, verify):
        """state() 给 True（已经对）、None（缺）或 False（在但不对）。"""
        current = state()
        require(current is not False, title + '：已经在，但不是预期的样子。不覆盖，先由人看过')
        if current:
            self.settled += 1
            self.out('[已有] ' + title)
            return
        require(not self.audit, title + '：还没做好。先跑完 prereq.py --apply')
        self.pending += 1
        self.out(('[现在做] ' if self.apply else '[要做] ') + title)
        action()
        if self.apply:
            require(verify(), title + '：做完之后核对不过')
            self.out('    核对通过')

    def place(self, path, source, digest, **stdin):
        """经 sudo 调本脚本的 --place：只新建，内容的摘要不是 digest 就不放。
        从检查「不在」到真正去放之间目标被别人建了，这里失败，不覆盖。"""
        argv = ['/usr/bin/python3', '-I', '-B', str(HERE / 'prereq.py'), '--place', path, digest]
        self.out('    sudo ' + ' '.join(argv) + ' < ' + source + '    （权限 ' + oct(PLACED[path])[2:].zfill(4) + '，已经在就失败）')
        if self.apply:
            self.run(['sudo', '--'] + argv, check=True, **stdin)

    def root_file(self, title, path, raw):
        def state():
            present = has(path, stat.S_ISREG, 0, 0, (PLACED[path],))
            return present if not present else content(path) == raw
        # 内容走标准输入：不落临时文件，预演和照做都一样。
        self.step(title, state,
                  lambda: self.place(path, '<内容见末尾或仓库模板>', hashlib.sha256(raw).hexdigest(), input=raw), state)

    def root_copy(self, title, source, path, digest):
        def state():
            present = has(path, stat.S_ISREG, 0, 0, (PLACED[path],))
            return present if not present else sha(path) == digest

        def copy():
            if not self.apply:
                self.place(path, source, digest)
                return
            with reading(source) as stream:
                self.place(path, source, digest, stdin=stream)
        self.step(title, state, copy, state)

    def identities(self):
        def readers_state():
            found = group(READERS)
            if found is None:
                return None
            low, high = system_range('GID')
            return low <= found.gr_gid <= high
        self.step('读取组 ' + READERS + '（系统组）', readers_state,
                  lambda: self.root('groupadd', '--system', READERS), readers_state)

        def worker_state():
            """已有的同名账号得和新建出来的一个样：系统账号（号码在系统范围里）、主组是同名的系统组、
            不能登录、家目录是建的时候给的那个。普通账号占了这个名字不算数。"""
            entry = account(WORKER)
            if entry is None:
                return None
            primary = group_numbered(entry.pw_gid)
            (low, high), (group_low, group_high) = system_range('UID'), system_range('GID')
            return (entry.pw_uid not in (0, OWNER_UID) and low <= entry.pw_uid <= high
                    and primary is not None and primary.gr_name == WORKER and group_low <= primary.gr_gid <= group_high
                    and entry.pw_shell.endswith('/nologin') and entry.pw_dir == WORKER_HOME)
        self.step('专用账号 ' + WORKER + '（系统账号，不能登录）', worker_state,
                  lambda: self.root('useradd', '--system', '--user-group', '--no-create-home', '--home-dir',
                                    WORKER_HOME, '--shell', WORKER_SHELL, WORKER), worker_state)
        for name in (WORKER, OWNER):
            def member(name=name):
                entry, readers = account(name), group(READERS)
                if entry is None or readers is None:
                    return None
                return True if readers.gr_gid in groups_of(name, entry.pw_gid) else None
            self.step(name + ' 加进读取组', member, lambda name=name: self.root('usermod', '-aG', READERS, name), member)

    def numbers(self):
        """两个号码建好账号之后才有；预演时还没建，就用占位的号码把其余内容生成出来给人看。"""
        entry, readers = account(WORKER), group(READERS)
        if entry and readers:
            return entry.pw_uid, readers.gr_gid, True
        return PLACEHOLDER_NUMBERS + (False,)

    def directories(self, reader_gid):
        self.root_file('tmpfiles 片段 ' + TMPFILES, TMPFILES, tmpfiles_text().encode())

        def state():
            found = [has(path, stat.S_ISDIR, OWNER_UID, reader_gid, (0o750,)) for path in (RUNTIME, STAGING)]
            return False if False in found else (None if None in found else True)
        self.step('两个目录 ' + RUNTIME + '、' + STAGING + '（' + OWNER + ':' + READERS + ' 0750）', state,
                  lambda: self.root('systemd-tmpfiles', '--create', TMPFILES), state)

    def programs(self):
        self.step('程序目录 ' + PROGRAMS, lambda: has(PROGRAMS, stat.S_ISDIR, 0, 0, (0o755,)),
                  # mkdir 遇到已经在的目录会失败；install -d 会把已有目录的属主权限改掉。
                  lambda: self.root('mkdir', '-m', '0755', PROGRAMS),
                  lambda: has(PROGRAMS, stat.S_ISDIR, 0, 0, (0o755,)))
        self.root_copy('worker 程序', WORKER_SOURCE, WORKER_BINARY, PACKAGES[2][1])
        self.root_copy('restic 0.19.1', RESTIC_SOURCE, RESTIC, RESTIC_SHA)
        self.root_copy('NAS 证书 ' + CA, CA_SOURCE, CA, CA_SHA)

    def units(self):
        for name in UNITS:
            self.root_file('单元 ' + name, UNIT_DIRECTORY + '/' + name, (TEMPLATES / name).read_bytes())
        if self.audit:
            return
        # 每次都重新加载一遍：上一次要是停在「单元放好了、还没重新加载」，这里补上（多加载一次没有代价）。
        self.root('systemctl', 'daemon-reload')
        if self.apply:
            # 只看这三个单元自己的问题：它们依赖的别的单元（人侧、网络）有告警不算在这里。
            self.run(['systemd-analyze', 'verify', '--recursive-errors=no']
                     + [UNIT_DIRECTORY + '/' + name for name in UNITS], check=True)
            for name in ('studiolo-backup-worker.service', 'studiolo-backup-schedule.timer'):
                enabled = self.run(['systemctl', 'is-enabled', name], capture_output=True, text=True).stdout.strip()
                active = self.run(['systemctl', 'is-active', name], capture_output=True, text=True).stdout.strip()
                require(enabled == 'disabled' and active == 'inactive', name + ' 不该是启用或在跑的')
            self.out('    三个单元语法核过，都没启用、没在跑')

    def private(self, backup):
        def ephemeral():
            return has(EPHEMERAL, stat.S_ISDIR, OWNER_UID, OWNER_UID, (0o700,))

        def make():
            if self.apply:
                make_directory(EPHEMERAL, 0o700)
        self.step('临时数据根 ' + EPHEMERAL + '（' + OWNER + ' 私有 0700）', ephemeral, make, ephemeral)

        def state():
            present = has(BACKUP_CONFIG, stat.S_ISREG, OWNER_UID, OWNER_UID, (0o600,))
            return present if not present else content(BACKUP_CONFIG) == backup

        def write():
            # 和 root 放文件同一个写法：先写临时名、落盘，再起正式的名字；写到一半出错或被打断不留半份。
            if self.apply:
                place(BACKUP_CONFIG, 0o600, io.BytesIO(backup), None, hashlib.sha256(backup).hexdigest())
        self.step('私有备份配置 ' + BACKUP_CONFIG + '（不用 sudo，0600）', state, write, state)

    def sources(self):
        entry = account(OWNER)
        require(sys.platform == 'linux' and entry is not None and entry.pw_uid == OWNER_UID == os.getuid() == os.geteuid(),
                '要在 169 上以 ' + OWNER + ' 跑，不要用 sudo 起这个脚本')
        for path, digest, what in ((WORKER_SOURCE, PACKAGES[2][1], 'worker 程序'), (RESTIC_SOURCE, RESTIC_SHA, 'restic'),
                                   (CA_SOURCE, CA_SHA, 'NAS 证书')):
            require(matches(path, digest), what + ' 的来源 ' + path + ' 不在或摘要不对')
        # 清单里登记的人侧程序要就是候选里的那两个：候选重出过而这里没跟着改，就在这儿拦下。
        for name, digest in PACKAGES[:2]:
            path = RELEASE_DIRECTORY + '/' + HUMAN_RELEASE + '/bin/' + name
            require(matches(path, digest), '候选里的 ' + name + ' 不在或摘要和这里登记的不一样')
        self.ground()
        self.out('[已核] 要拷的三样都在、摘要对得上；候选里的两个人侧程序和清单登记的一致；'
                 '/etc/studiolo 和人侧私有配置目录的属主权限对；各条目录链上没有链接、没有别人可写的目录')

    def ground(self):
        """要建的东西落脚的地方：所在目录的属主权限，和它上面的整条目录链。"""
        require(has('/etc/studiolo', stat.S_ISDIR, 0, 0, (0o755,)), '/etc/studiolo 不是 root 的 0755 目录')
        require(has(str(Path(BACKUP_CONFIG).parent), stat.S_ISDIR, OWNER_UID, OWNER_UID, (0o700,)),
                '人侧私有配置目录不是 ' + OWNER + ' 的 0700 目录')
        # 上面两条只看最后一级；每个要建的东西，它上面的整条目录链也照使用方的规则核一遍。
        for path in (BACKUP_CONFIG, EPHEMERAL):
            chain(path, OWNER_UID)
        for path in (WORKER_CONFIG, PROGRAMS, TMPFILES, UNIT_DIRECTORY + '/' + UNITS[0], RUNTIME, STAGING):
            chain(path, 0)

    def products(self):
        self.identities()
        worker_uid, reader_gid, real = self.numbers()
        backup_document_ = backup_document(worker_uid, reader_gid)
        worker_document_ = worker_document(worker_uid, reader_gid)
        check_backup_document(backup_document_)
        check_worker_document(worker_document_, backup_document_)
        backup, worker = encode(backup_document_), encode(worker_document_)
        if not real:
            self.out('（账号还没建：下面两份配置里的 64001、64002 是占位的号码，照做时换成真号码）')
        self.directories(reader_gid)
        self.programs()
        self.root_file('worker 配置 ' + WORKER_CONFIG, WORKER_CONFIG, worker)
        self.private(backup)
        # 单元放最后：语法核对万一报问题，前面的东西都已经就位，不用重做。
        self.units()
        if not (self.apply or self.audit):
            self.out('\n---- ' + WORKER_CONFIG + '\n' + worker.decode() + '---- ' + BACKUP_CONFIG + '\n' + backup.decode())
        return self.pending

    def everything(self):
        self.sources()
        return self.products()


def settled(out=lambda line: None):
    """只读：prereq.py 的每一样产物都已经在、而且对；缺一样、或者有一样不对，就停下。返回核过的项数。
    给 credentials.py 用：它以 root 跑，过不了 sources() 的「以主服务账号跑」，也不该再去看要拷的来源。
    不核「三个单元没启用、没在跑」：那是置备当时的条件，credentials.py 自己另核 worker 没在跑。"""
    audit = Provision(apply=False, out=out, audit=True)
    audit.ground()
    audit.products()
    return audit.settled


def main(argv):
    if argv[:1] == ['--place']:
        require(len(argv) == 3 and argv[1] in PLACED and is_digest(argv[2]) and os.geteuid() == 0
                and not sys.stdin.isatty(), '--place 只由本脚本自己经 sudo 调用，目标只能是它要放的那几个文件')
        place(argv[1], PLACED[argv[1]], sys.stdin.buffer, PLACED_OWNER, argv[2])
        return
    require(argv in ([], ['--apply']), __doc__)
    provision = Provision(apply=argv == ['--apply'])
    pending = provision.everything()
    if provision.apply:
        print('backup_prereq=ready（这一轮做了 ' + str(pending) + ' 步；凭据还没录，单元没启用）')
    else:
        print('预演结束：没改任何东西。还差 ' + str(pending) + ' 步；带 --apply 再跑一遍才照做。')


def entry(argv):
    """把「停下」变成一句话和退出码 1；被打断时让收尾照常跑完。"""
    interruptible()
    try:
        main(argv)
    except Refused as refusal:
        print('停下：' + str(refusal), file=sys.stderr)
    except subprocess.CalledProcessError as failure:
        print('停下：命令失败（退出码 ' + str(failure.returncode) + '）：' + ' '.join(map(str, failure.cmd)), file=sys.stderr)
    except OSError as failure:
        print('停下：' + type(failure).__name__ + '：' + str(failure), file=sys.stderr)
    except KeyboardInterrupt:
        print('\n停下：被打断。正在放的文件不留半份，已经做完的步骤留着；重跑会从现状接着做。', file=sys.stderr)
    else:
        return 0
    return 1


if __name__ == '__main__':
    sys.exit(entry(sys.argv[1:]))
