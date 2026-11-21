#!/usr/bin/python3
"""169 上启用备份 worker 之前、不涉及秘密的那几样东西（README「安装前要有的东西」第 1～4、6 项和三个单元）。

用法（在 169 上以主服务账号跑；不要用 sudo 起这个脚本，要 root 的步骤它自己逐条经 sudo 执行）：
    python3 -I -B prereq.py            预演：只读。逐项说现在怎样、会做什么，把要写的两份配置原样打出来
    python3 -I -B prereq.py --apply    照做：每一步做完当场核对，不对就停
重复跑是安全的：已经对的跳过，在但不对的停下（不覆盖、不删除），缺的才建。
不碰凭据（见同目录 credentials.py），不启用、不启动任何单元，不动人侧服务。

「发行相关的值」那一段跟着人侧发行走：候选重出、worker 重建，都要改这里再重跑。
"""
import grp
import hashlib
import json
import os
from pathlib import Path
import pwd
import stat
import subprocess
import sys
import tempfile

HERE = Path(__file__).resolve().parent
TEMPLATES = HERE.parent / 'systemd'

# ---------------------------------------------------------------- 169 的固定事实
OWNER = 'maxsong'
OWNER_UID = 1000
WORKER = 'studiolo-backup'
READERS = 'studiolo-backup-read'
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


def has(path, kind, uid, gid, modes):
    """路径在不在、是不是预期的样子：None 不在，True 对，False 在但不对。"""
    try:
        status = os.lstat(path)
    except FileNotFoundError:
        return None
    return (kind(status.st_mode) and (status.st_uid, status.st_gid) == (uid, gid)
            and stat.S_IMODE(status.st_mode) in modes and (stat.S_ISDIR(status.st_mode) or status.st_nlink == 1))


# ---------------------------------------------------------------- 现场：会改东西的步骤
class Provision:
    def __init__(self, apply, run=subprocess.run, out=print):
        self.apply, self.run, self.out = apply, run, out
        self.pending = 0

    def root(self, *argv):
        self.out('    sudo ' + ' '.join(argv))
        if self.apply:
            self.run(['sudo', '--'] + list(argv), check=True)

    def step(self, title, state, action, verify):
        """state() 给 True（已经对）、None（缺）或 False（在但不对）。"""
        current = state()
        require(current is not False, title + '：已经在，但不是预期的样子。不覆盖，先由人看过')
        if current:
            self.out('[已有] ' + title)
            return
        self.pending += 1
        self.out(('[现在做] ' if self.apply else '[要做] ') + title)
        action()
        if self.apply:
            require(verify(), title + '：做完之后核对不过')
            self.out('    核对通过')

    def root_file(self, title, path, content, mode):
        def install():
            if not self.apply:  # 预演不落任何文件，连临时文件也不建
                self.out('    sudo install -o root -g root -m ' + oct(mode)[2:].zfill(4) + ' <内容见末尾或仓库模板> ' + path)
                return
            with tempfile.TemporaryDirectory(prefix='studiolo-backup-prereq.') as scratch:
                staged = Path(scratch) / 'content'
                staged.write_bytes(content)
                self.root('install', '-o', 'root', '-g', 'root', '-m', oct(mode)[2:].zfill(4), str(staged), path)

        def state():
            present = has(path, stat.S_ISREG, 0, 0, (mode,))
            return present if not present else Path(path).read_bytes() == content
        self.step(title, state, install, state)

    def root_copy(self, title, source, path, digest, mode, modes):
        def state():
            present = has(path, stat.S_ISREG, 0, 0, modes)
            return present if not present else sha(path) == digest
        self.step(title, state, lambda: self.root('install', '-o', 'root', '-g', 'root', '-m', mode, source, path), state)

    def identities(self):
        self.step('读取组 ' + READERS, lambda: True if group(READERS) else None,
                  lambda: self.root('groupadd', '--system', READERS), lambda: group(READERS) is not None)

        def worker_state():
            entry = account(WORKER)
            if entry is None:
                return None
            return (entry.pw_uid not in (0, OWNER_UID) and grp.getgrgid(entry.pw_gid).gr_name == WORKER
                    and entry.pw_shell.endswith('/nologin'))
        self.step('专用账号 ' + WORKER + '（系统账号，不能登录）', worker_state,
                  lambda: self.root('useradd', '--system', '--user-group', '--no-create-home', '--home-dir',
                                    '/nonexistent', '--shell', '/usr/sbin/nologin', WORKER), worker_state)
        for name in (WORKER, OWNER):
            def member(name=name):
                entry, readers = account(name), group(READERS)
                if entry is None or readers is None:
                    return None
                return True if readers.gr_gid in os.getgrouplist(name, entry.pw_gid) else None
            self.step(name + ' 加进读取组', member, lambda name=name: self.root('usermod', '-aG', READERS, name), member)

    def numbers(self):
        """两个号码建好账号之后才有；预演时还没建，就用占位的号码把其余内容生成出来给人看。"""
        entry, readers = account(WORKER), group(READERS)
        if entry and readers:
            return entry.pw_uid, readers.gr_gid, True
        return 64001, 64002, False

    def directories(self, reader_gid):
        self.root_file('tmpfiles 片段 ' + TMPFILES, TMPFILES, tmpfiles_text().encode(), 0o644)

        def state():
            found = [has(path, stat.S_ISDIR, OWNER_UID, reader_gid, (0o750,)) for path in (RUNTIME, STAGING)]
            return False if False in found else (None if None in found else True)
        self.step('两个目录 ' + RUNTIME + '、' + STAGING + '（' + OWNER + ':' + READERS + ' 0750）', state,
                  lambda: self.root('systemd-tmpfiles', '--create', TMPFILES), state)

    def programs(self):
        self.step('程序目录 ' + PROGRAMS, lambda: has(PROGRAMS, stat.S_ISDIR, 0, 0, (0o755,)),
                  lambda: self.root('install', '-d', '-o', 'root', '-g', 'root', '-m', '0755', PROGRAMS),
                  lambda: has(PROGRAMS, stat.S_ISDIR, 0, 0, (0o755,)))
        self.root_copy('worker 程序', WORKER_SOURCE, WORKER_BINARY, PACKAGES[2][1], '0755', (0o755,))
        self.root_copy('restic 0.19.1', RESTIC_SOURCE, RESTIC, RESTIC_SHA, '0555', (0o555,))
        self.root_copy('NAS 证书 ' + CA, CA_SOURCE, CA, CA_SHA, '0644', (0o644,))

    def units(self):
        before = self.pending
        for name in UNITS:
            self.root_file('单元 ' + name, UNIT_DIRECTORY + '/' + name, (TEMPLATES / name).read_bytes(), 0o644)
        if self.pending > before:
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
                os.mkdir(EPHEMERAL, 0o700)
        self.step('临时数据根 ' + EPHEMERAL + '（' + OWNER + ' 私有 0700）', ephemeral, make, ephemeral)

        def state():
            present = has(BACKUP_CONFIG, stat.S_ISREG, OWNER_UID, OWNER_UID, (0o600,))
            return present if not present else Path(BACKUP_CONFIG).read_bytes() == backup

        def write():
            if not self.apply:
                return
            descriptor = os.open(BACKUP_CONFIG, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
            with os.fdopen(descriptor, 'wb') as stream:
                stream.write(backup)
                stream.flush()
                os.fsync(stream.fileno())
        self.step('私有备份配置 ' + BACKUP_CONFIG + '（不用 sudo，0600）', state, write, state)

    def sources(self):
        entry = account(OWNER)
        require(sys.platform == 'linux' and entry is not None and entry.pw_uid == OWNER_UID == os.getuid() == os.geteuid(),
                '要在 169 上以 ' + OWNER + ' 跑，不要用 sudo 起这个脚本')
        for path, digest, what in ((WORKER_SOURCE, PACKAGES[2][1], 'worker 程序'), (RESTIC_SOURCE, RESTIC_SHA, 'restic'),
                                   (CA_SOURCE, CA_SHA, 'NAS 证书')):
            require(os.path.isfile(path) and sha(path) == digest, what + ' 的来源 ' + path + ' 不在或摘要不对')
        # 清单里登记的人侧程序要就是候选里的那两个：候选重出过而这里没跟着改，就在这儿拦下。
        for name, digest in PACKAGES[:2]:
            path = RELEASE_DIRECTORY + '/' + HUMAN_RELEASE + '/bin/' + name
            require(os.path.isfile(path) and sha(path) == digest, '候选里的 ' + name + ' 不在或摘要和这里登记的不一样')
        require(has('/etc/studiolo', stat.S_ISDIR, 0, 0, (0o755,)), '/etc/studiolo 不是 root 的 0755 目录')
        require(has(str(Path(BACKUP_CONFIG).parent), stat.S_ISDIR, OWNER_UID, OWNER_UID, (0o700,)),
                '人侧私有配置目录不是 ' + OWNER + ' 的 0700 目录')
        self.out('[已核] 要拷的三样都在、摘要对得上；候选里的两个人侧程序和清单登记的一致；/etc/studiolo 和人侧私有配置目录的属主权限对')

    def everything(self):
        self.sources()
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
        self.root_file('worker 配置 ' + WORKER_CONFIG, WORKER_CONFIG, worker, 0o644)
        self.private(backup)
        # 单元放最后：语法核对万一报问题，前面的东西都已经就位，不用重做。
        self.units()
        if not self.apply:
            self.out('\n---- ' + WORKER_CONFIG + '\n' + worker.decode() + '---- ' + BACKUP_CONFIG + '\n' + backup.decode())
        return self.pending


def main(argv):
    require(argv in ([], ['--apply']), __doc__)
    provision = Provision(apply=argv == ['--apply'])
    pending = provision.everything()
    if provision.apply:
        print('backup_prereq=ready（这一轮做了 ' + str(pending) + ' 步；凭据还没录，单元没启用）')
    else:
        print('预演结束：没改任何东西。还差 ' + str(pending) + ' 步；带 --apply 再跑一遍才照做。')


if __name__ == '__main__':
    try:
        main(sys.argv[1:])
    except Refused as refusal:
        print('停下：' + str(refusal), file=sys.stderr)
        sys.exit(1)
    except subprocess.CalledProcessError as failure:
        print('停下：命令失败（退出码 ' + str(failure.returncode) + '）：' + ' '.join(map(str, failure.cmd)), file=sys.stderr)
        sys.exit(1)
