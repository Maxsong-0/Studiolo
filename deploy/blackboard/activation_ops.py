"""有界双服务切换。只恢复程序文件，从不恢复、删除或迁移数据库。"""
import fcntl
import os
from pathlib import Path
import pwd
import shlex
import stat
import subprocess
import sys
import tempfile
import time

from board_candidate import verify_previous, verify_candidate
from board_binding import binding, board_files
from board_snapshot import SnapshotOperations

CONFIG = Path('/home/maxsong/.config/studiolo-human/owner.json')
RECORDS = Path('/var/lib/studiolo-deployments')
AUTH = '/home/maxsong/.local/share/studiolo-human-auth'
AUDIT = '/home/maxsong/.local/share/studiolo-human-audit'
UNIT_ROOT = Path('/etc/systemd/system')
NAMES = ('studiolo-human', 'studiolo-agent')
# 停服务的命令时限 = 人侧单元的停止超时（assembly.STOP_TIMEOUT_SECONDS）+ 这个余量（秒）：留给 systemd 到点强杀之后的收尾。
# 命令时限比停止超时短的话，慢关停会让激活和回滚在这条命令上超时。agent 单元自己的停止超时是 20 秒，共用这个上限。
STOP_COMMAND_MARGIN_SECONDS = 20
# backup.json 的键名，和程序解析它的两个结构一一对应（crates/host-config/src/backup/file.rs、producer.rs 的 Document，
# 都是多一个少一个键就拒绝）；test_activation 对着源码钉。
BACKUP_DOCUMENT_KEYS = frozenset({'version', 'backup', 'backup_producer'})
BACKUP_PRODUCER_KEYS = frozenset({
    'version', 'worker_uid', 'reader_gid', 'socket', 'staging_parent', 'ephemeral_root', 'source_retention_millis',
    'freeze_budget_millis', 'application_packages', 'restic_digest', 'deployment'})
# 程序接受的下限（producer.rs 的 FREEZE_MILLIS、sync_transport/document/validate.rs 的 stop_grace_seconds）：
# 比它小的程序启动时就拒绝，预检不能放过。上限用装配里算停止超时的那两个值。test_activation 对着源码钉。
FREEZE_BUDGET_FLOOR_MILLIS = 1000
STOP_GRACE_FLOOR_SECONDS = 1
# 人侧单元 PrivateTmp=yes：宿主这两处事先建好的目录，服务里看不见。
PRIVATE_TMP = (Path('/tmp'), Path('/var/tmp'))


class Operations(SnapshotOperations):
    def __init__(self, manifest_sha, a, c, runtime, legacy):
        self.a, self.c, self.runtime, self.legacy = a, c, runtime, legacy
        self.manifest_sha, self.build = manifest_sha, a.BUILD
        self.record = self.lock = self.before = self.config = None
        self.old_units, self.new_units, self.new_shas = {}, {}, {}
        self.old_shas = dict(zip(NAMES, (a.HUMAN_SHA, a.AGENT_SHA)))

    def begin(self):
        c = self.c
        c.require(sys.platform == 'linux' and os.getuid() == os.geteuid() == 0)
        c.require(pwd.getpwnam('maxsong').pw_uid == c.UID == 1000)
        c.inspect(RECORDS, 0, directory=True)
        path = RECORDS / '.board-r1.lock'
        self.lock = os.open(path, os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW | os.O_CLOEXEC, 0o600)
        before = c.inspect(path, 0, 0o600)
        c.require(c.mark(os.fstat(self.lock)) == c.mark(before))
        fcntl.flock(self.lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        self.record = Path(tempfile.mkdtemp(prefix='board-' + self.build.rsplit('-', 1)[1] + '-', dir=RECORDS))
        c.inspect(self.record, 0, 0o700, True)

    def unchanged(self):
        return self.legacy.Operations.unchanged(self)

    def receipt(self, name, value):
        return self.legacy.Operations.receipt(self, name, value)

    def properties(self, name):
        c = self.c
        c.require(name in NAMES)
        unit = UNIT_ROOT / (name + '.service')
        raw = self.runtime.command(['systemctl', 'show', name + '.service',
            '--property=ActiveState,SubState,MainPID,NRestarts,FragmentPath,DropInPaths,UnitFileState'], 2).stdout
        values = dict(line.split('=', 1) for line in raw.splitlines())
        c.require(values['FragmentPath'] == str(unit) and values['DropInPaths'] == '')
        c.require(values['UnitFileState'] == 'enabled')
        return values

    def trusted_read(self, path, mode=None):
        c = self.c
        c.ancestors(path, c.UID)
        s = path.lstat()
        c.require(stat.S_ISREG(s.st_mode) and s.st_uid in (0, c.UID)
                  and s.st_mode & 0o7022 == 0 and s.st_nlink == 1 and s.st_size <= c.MAX_FILE)
        c.require(mode is None or stat.S_IMODE(s.st_mode) == mode)
        fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC)
        try:
            c.require(c.mark(os.fstat(fd)) == c.mark(s))
            with os.fdopen(fd, 'rb', closefd=False) as stream:
                raw = stream.read(c.MAX_FILE + 1)
            c.require(len(raw) == s.st_size and c.mark(os.fstat(fd)) == c.mark(s)
                      and c.mark(path.lstat()) == c.mark(s))
            return raw
        finally:
            os.close(fd)

    def read_agent(self):
        # 既有 agent 发行包由 root 冻结（root:maxsong 0550）；不改变其所有权或文件。
        # 2026-09-22 T-101 曾带出一版未冻结的 0700，已由其发布线冻结回本形态，这里不为它放宽。
        return self.trusted_read(self.agent_binary, 0o550)

    def agent_http_ready(self, deadline):
        """固定 loopback、无凭据 HEAD：不下载正文，不跟随跳转或读取代理配置。"""
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            return False
        timeout = min(1.0, remaining)
        try:
            result = self.runtime.command([
                '/usr/bin/curl', '--disable', '--head', '--silent',
                '--output', '/dev/null', '--write-out', '%{http_code}',
                '--noproxy', '*', '--proxy', '', '--proto', '=http',
                '--max-redirs', '0', '--max-time', str(timeout),
                '--url', 'http://127.0.0.1:4400/mcp'], timeout)
        except (OSError, subprocess.SubprocessError):
            return False
        return result.stdout == '401' and time.monotonic() < deadline

    def agent_ready(self):
        c = self.c
        deadline = time.monotonic() + 20
        pid = self.runtime.wait_program(self.agent_binary,
            get_properties=lambda: self.properties('studiolo-agent'),
            socket_exists=lambda: self.agent_http_ready(deadline),
            deadline_seconds=20, monotonic=time.monotonic, sleep=time.sleep)
        c.require(c.sha(self.read_agent()) == self.agent_sha)
        opened, supplied = os.stat('/proc/' + str(pid) + '/exe'), self.agent_binary.stat()
        c.require((opened.st_dev, opened.st_ino) == (supplied.st_dev, supplied.st_ino))
        c.require(self.properties('studiolo-agent')['MainPID'] == str(pid))
        c.require(time.monotonic() < deadline)
        return pid

    def board_ready(self):
        c = self.c
        path = Path(self.a.BOARD_SOCKET)
        c.inspect(path.parent, c.UID, 0o700, True)
        s = path.lstat()
        c.require(stat.S_ISSOCK(s.st_mode) and s.st_uid == c.UID
                  and stat.S_IMODE(s.st_mode) == 0o600 and s.st_nlink == 1)

    def units(self):
        return {name: self.c.read(UNIT_ROOT / (name + '.service'), 0, 0o644) for name in NAMES}

    def owner_groups(self):
        account = pwd.getpwnam('maxsong')
        return set(os.getgrouplist(account.pw_name, account.pw_gid))

    def backup_ready(self):
        """人侧带 --backup-config 启动要的东西，在停服之前核完（T-216）。
        私有配置、两个目录、读取组成员、socket 残留：缺一样人侧起不来。临时数据根、冻结预算、Bridge 停止宽限：
        配错了人侧照常起，要到封存或慢关停才出事，所以也在这里拦。ACL 不在这里核，程序打开目录时自己核。

        backup.json 只核到这一层：键名齐不齐（拼错键、漏段是手写时最常见的错）、两处 version、和部署有关的几个值。
        其余字段的取值规则（审批段内容、worker 账号、保留期、清单里的包和摘要、路径互不嵌套、socket 路径长度）
        不在这里重写一遍，由程序启动时核：那些错了，会在停服之后启动失败、退回上一发行。"""
        c, a = self.c, self.a
        document = c.document(c.read(Path(a.BACKUP_CONFIG), c.UID, 0o600))
        c.require(type(document) is dict and set(document) == BACKUP_DOCUMENT_KEYS)
        approval, producer = document['backup'], document['backup_producer']
        c.require(type(document['version']) is int and document['version'] == 1)
        c.require(type(approval) is dict and bool(approval))
        c.require(type(producer) is dict and set(producer) == BACKUP_PRODUCER_KEYS)
        c.require(type(producer['version']) is int and producer['version'] == 1)
        # 这两条路径下面要按路径解析：先核是字符串（封存父目录只做相等比较，类型不对自然不等）。
        c.require(all(type(producer[key]) is str for key in ('socket', 'ephemeral_root')))
        socket, reader = c.absolute(producer['socket']), producer['reader_gid']
        c.require(type(reader) is int and reader > 0 and reader in self.owner_groups())
        c.require(str(socket.parent) == a.BACKUP_RUNTIME and producer['staging_parent'] == a.BACKUP_STAGING)
        for path in (a.BACKUP_RUNTIME, a.BACKUP_STAGING):
            s = os.lstat(path)
            c.require(stat.S_ISDIR(s.st_mode) and (s.st_uid, s.st_gid) == (c.UID, reader)
                      and stat.S_IMODE(s.st_mode) == 0o750)
        # 上一发行（r16）不监听备份 socket：此刻它必须不存在，残留的由人看过再删（程序不清理也不接管）。
        # 起点已带备份监听的发行要改成像 board_ready 那样核活的 socket，「必须不存在」挪到 stopped()。
        c.require(not os.path.lexists(socket))
        ephemeral = c.absolute(producer['ephemeral_root'])
        c.require(not any(ephemeral.is_relative_to(private) for private in PRIVATE_TMP))
        c.inspect(ephemeral, c.UID, 0o700, True)
        # 单元的停止超时按这两个预算算出来（assembly.STOP_TIMEOUT_SECONDS）：现场配得更大就得先改装配。
        freeze = producer['freeze_budget_millis']
        c.require(type(freeze) is int
                  and FREEZE_BUDGET_FLOOR_MILLIS <= freeze <= a.BACKUP_FREEZE_BUDGET_SECONDS * 1000)
        transport = c.document(c.read(Path(a.SYNC_APPROVAL), c.UID, 0o600))
        budget = transport.get('budget') if type(transport) is dict else None
        grace = budget.get('stop_grace_seconds') if type(budget) is dict else None
        c.require(type(grace) is int and STOP_GRACE_FLOOR_SECONDS <= grace <= a.BRIDGE_STOP_GRACE_SECONDS)

    def preflight(self):
        c, a = self.c, self.a
        c.require(sys.platform == 'linux' and os.getuid() == os.geteuid() == 0)
        c.require(pwd.getpwnam('maxsong').pw_uid == c.UID == 1000)
        self.manifest = verify_candidate(a, c, self.manifest_sha)
        self.backup_ready()
        self.old_units = self.units()
        c.require({name: c.sha(raw) for name, raw in self.old_units.items()} == self.old_shas)
        self.new_units = {name: c.read(Path(a.RELEASE) / (name + '.service'), c.UID, 0o600) for name in NAMES}
        c.require(self.new_units['studiolo-human'] == a.human_unit(self.old_units['studiolo-human'].decode()).encode())
        c.require(self.new_units['studiolo-agent'] == a.agent_unit(self.old_units['studiolo-agent'].decode()).encode())
        expected = a.launcher(c.read(Path(a.OLD_RELEASE) / 'launch-human.py', c.UID, 0o600).decode()).encode()
        c.require(c.read(Path(a.RELEASE) / 'launch-human.py', c.UID, 0o600) == expected)
        self.new_shas = {name: c.sha(raw) for name, raw in self.new_units.items()}
        self.config = c.read(CONFIG, c.UID, 0o600)
        document = c.document(self.config)
        c.require(document['seed_ref'] == 'studiolo-human-totp-v2')
        c.require(document['auth_directory'] == AUTH and document['audit_directory'] == AUDIT)
        c.inspect(CONFIG.parent, c.UID, 0o700, True)
        c.inspect(Path('/etc/credstore.encrypted/studiolo-human-totp-v2'), 0, 0o600)
        self.before = self.worker('binding')
        self.broker_binding = binding(c, a)
        # 起点是已经在跑黑板的 r2：socket 此刻必须是活的那一个（类型/属主/权限/链接数都核）。
        # 「必须不存在」的那一条留在停服之后，见 stopped()——那才是切换前的真正保证。
        self.board_ready()
        starts = [line[len('ExecStart='):] for line in self.old_units['studiolo-agent'].decode().splitlines()
                  if line.startswith('ExecStart=')]
        c.require(len(starts) == 1)
        command = shlex.split(starts[0])
        c.require(len(command) > 1 and command[1] == 'serve')
        self.agent_binary = c.absolute(command[0])
        c.require(str(self.agent_binary).endswith('/bin/studiolo-agent'))
        self.agent_sha = c.sha(self.read_agent())
        for name in NAMES:
            self.properties(name)
        self.runtime.command(['systemd-analyze', 'verify',
            Path(a.RELEASE) / 'studiolo-human.service', Path(a.RELEASE) / 'studiolo-agent.service'])
        self.human_sha = self.manifest['files']['bin/studiolo-local-server']['sha256']
        old_manifest = c.read(Path(a.OLD_RELEASE) / 'manifest.json', c.UID, 0o600)
        c.require(c.sha(old_manifest) == a.R8_MANIFEST_SHA)
        self.old_human_sha = c.document(old_manifest)['files']['bin/studiolo-local-server']['sha256']
        self.runtime.ready(Path(a.OLD_RELEASE) / 'bin/studiolo-local-server', self.old_human_sha)
        self.agent_ready()
        self.receipt('preflight', {'before': self.before, 'manifest_sha256': self.manifest_sha,
            'old_units': self.old_shas, 'new_units': self.new_shas,
            'agent_binary_sha256': self.agent_sha, 'board_authorization_accepted': False})

    def stopped(self):
        deadline = time.monotonic() + 20
        while True:
            states = [self.properties(name) for name in NAMES]
            if all(p['ActiveState'] in ('inactive', 'failed') and p['MainPID'] == '0' for p in states):
                self.c.require(not os.path.lexists(self.runtime.SOCKET)
                               and not os.path.lexists(self.a.BOARD_SOCKET))
                return
            self.c.require(time.monotonic() < deadline)
            time.sleep(0.1)

    def known_units(self):
        current = {name: self.c.sha(raw) for name, raw in self.units().items()}
        self.c.require(all(current[name] in (self.old_shas[name], self.new_shas.get(name)) for name in NAMES))
        for name in NAMES:
            self.properties(name)
        return current

    def stop(self):
        self.known_units()
        limit = self.a.STOP_TIMEOUT_SECONDS + STOP_COMMAND_MARGIN_SECONDS
        self.runtime.command(['systemctl', 'stop', 'studiolo-agent.service'], limit)
        self.runtime.command(['systemctl', 'stop', 'studiolo-human.service'], limit)
        self.stopped()

    def install(self):
        c = self.c
        self.stopped()
        verify_candidate(self.a, c, self.manifest_sha)
        self.unchanged()
        c.require(binding(c, self.a) == self.broker_binding)
        c.require({name: c.sha(raw) for name, raw in self.units().items()} == self.old_shas)
        for name in NAMES:
            c.atomic_unit(UNIT_ROOT / (name + '.service'), self.new_units[name], self.old_shas[name])
        self.runtime.command(['systemctl', 'daemon-reload'])

    def start_human(self):
        self.runtime.command(['systemctl', 'start', 'studiolo-human.service'], 30)
        self.runtime.ready(Path(self.a.RELEASE) / 'bin/studiolo-local-server', self.human_sha)
        self.board_ready()

    def start_agent(self):
        self.runtime.command(['systemctl', 'start', 'studiolo-agent.service'], 30)
        self.agent_ready()

    def accept(self):
        c = self.c
        human = self.runtime.ready(Path(self.a.RELEASE) / 'bin/studiolo-local-server', self.human_sha)
        self.board_ready()
        agent = self.agent_ready()
        self.runtime.denials()
        self.unchanged()
        c.require({name: c.sha(raw) for name, raw in self.units().items()} == self.new_shas)
        verify_candidate(self.a, c, self.manifest_sha)
        return {'pids': {'human': human, 'agent': agent}, 'after': self.before,
                'sole_credential': 'totp-next', 'anonymous_endpoints_401': 6,
                'agent_anonymous_mcp_401': True,
                'unit_sha256': self.new_shas, 'board_socket_private': True,
                'board_authorization_accepted': False, 'native_clients_accepted': False}

    def recover(self):
        # 未知单元连 stop 都不替其决定；已知配置才停止受控服务。
        self.known_units()
        self.stop()
        self.unchanged()
        current = self.known_units()
        for name in NAMES:
            if current[name] != self.old_shas[name]:
                self.c.atomic_unit(UNIT_ROOT / (name + '.service'), self.old_units[name], current[name])
        self.runtime.command(['systemctl', 'daemon-reload'])
        self.runtime.command(['systemctl', 'start', 'studiolo-human.service'], 30)
        self.runtime.ready(Path(self.a.OLD_RELEASE) / 'bin/studiolo-local-server', self.old_human_sha)
        self.runtime.command(['systemctl', 'start', 'studiolo-agent.service'], 30)
        self.agent_ready()
        self.runtime.denials()
        self.unchanged()
        self.c.require({name: self.c.sha(raw) for name, raw in self.units().items()} == self.old_shas)
        return 'old_programs_restored'

    def close(self):
        if self.lock is not None:
            os.close(self.lock)
            self.lock = None
