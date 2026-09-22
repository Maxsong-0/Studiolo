"""有界双服务切换。只恢复程序文件，从不恢复、删除或迁移数据库。"""
import fcntl
import json
import os
from pathlib import Path
import pwd
import re
import shlex
import sqlite3
import stat
import subprocess
import sys
import tempfile
import time
import uuid

CONFIG = Path('/home/maxsong/.config/studiolo-human/owner.json')
RECORDS = Path('/var/lib/studiolo-deployments')
AUTH = '/home/maxsong/.local/share/studiolo-human-auth'
AUDIT = '/home/maxsong/.local/share/studiolo-human-audit'
UNIT_ROOT = Path('/etc/systemd/system')
NAMES = ('studiolo-human', 'studiolo-agent')


# manifest 的闭集：r2 那一版没有 web_source_commit，r3 起必须有。
MANIFEST_KEYS = {'build', 'bootstrap_source_commit', 'human_program_source_commit',
                 'web_source_commit', 'r8_manifest_sha256', 'assembly', 'before_units', 'files'}


def verify_previous(a, c):
    """上一发行也是黑板形状的 manifest，用同一闭集逐项核验；不沿用 r8 时代的 verifier。"""
    root = Path(a.OLD_RELEASE)
    raw = c.read(root / 'manifest.json', c.UID, 0o600)
    c.require(c.sha(raw) == a.R8_MANIFEST_SHA)
    manifest = c.document(raw)
    c.require(MANIFEST_KEYS - {'web_source_commit'} <= set(manifest) <= MANIFEST_KEYS)
    c.require(manifest['assembly'] == a.ASSEMBLY and isinstance(manifest['build'], str))
    for key in ('bootstrap_source_commit', 'human_program_source_commit'):
        c.require(isinstance(manifest[key], str) and re.fullmatch('[0-9a-f]{40}', manifest[key]))
    files = manifest['files']
    c.require(isinstance(files, dict) and files)
    c.require(any(name.startswith(a.WEB_PREFIX) for name in files))
    directories = set()
    for name, item in files.items():
        path = c.relative(name)
        c.require(name != 'manifest.json')
        c.require(isinstance(item, dict) and set(item) == {'bytes', 'sha256'})
        c.require(type(item['bytes']) is int and 0 <= item['bytes'] <= c.MAX_FILE)
        c.require(isinstance(item['sha256'], str) and re.fullmatch('[0-9a-f]{64}', item['sha256']))
        directories.update(p.as_posix() for p in path.parents if str(p) != '.')
    actual, dirs = c.scan(root, c.UID)
    c.require(dirs == directories and set(actual) == set(files) | {'manifest.json'})
    c.require(all(actual[name] == item for name, item in files.items()))
    c.require(c.read(root / 'manifest.json', c.UID, 0o600) == raw)
    return manifest


def verify_candidate(a, c, expected_sha):
    """候选包＝上一发行换掉整个 web/；其余逐字节相同，程序来源必须照抄不得冒称新构建。"""
    c.require(isinstance(expected_sha, str) and re.fullmatch('[0-9a-f]{64}', expected_sha))
    root = Path(a.RELEASE)
    old = verify_previous(a, c)
    raw = c.read(root / 'manifest.json', c.UID, 0o600)
    c.require(c.sha(raw) == expected_sha)
    manifest = c.document(raw)
    c.require(set(manifest) == MANIFEST_KEYS)
    c.require(manifest['build'] == a.BUILD and manifest['assembly'] == a.ASSEMBLY)
    c.require(re.fullmatch('[0-9a-f]{40}', manifest['bootstrap_source_commit']))
    c.require(isinstance(manifest['human_program_source_commit'], str)
              and re.fullmatch('[0-9a-f]{40}', manifest['human_program_source_commit']))
    # 网页必须能追到构建它的提交；程序沿用上一发行，照抄其来源，不冒称新构建。
    c.require(isinstance(manifest['web_source_commit'], str)
              and re.fullmatch('[0-9a-f]{40}', manifest['web_source_commit']))
    c.require(manifest['bootstrap_source_commit'] == old['bootstrap_source_commit'])
    c.require(manifest['human_program_source_commit'] == old['human_program_source_commit'])
    c.require(manifest['r8_manifest_sha256'] == a.R8_MANIFEST_SHA)
    c.require(manifest['before_units'] == dict(zip(NAMES, (a.HUMAN_SHA, a.AGENT_SHA))))
    files = manifest['files']
    c.require(isinstance(files, dict))
    # 这三份按现场重新派生：启动器与人侧单元改发行路径，agent 单元照抄现场
    # （读取范围可能在两次发行之间被 L3 改过，必须以现场为准而不是上一发行的存档）。
    changed = {'launch-human.py', 'studiolo-human.service', 'studiolo-agent.service'}
    old_web = {name for name in old['files'] if name.startswith(a.WEB_PREFIX)}
    new_web = {name for name in files if name.startswith(a.WEB_PREFIX)}
    c.require(new_web and a.WEB_PREFIX + 'index.html' in new_web)
    c.require(set(files) == (set(old['files']) - old_web) | new_web)
    directories = set()
    for name, item in files.items():
        path = c.relative(name)
        c.require(isinstance(item, dict) and set(item) == {'bytes', 'sha256'})
        c.require(type(item['bytes']) is int and 0 <= item['bytes'] <= c.MAX_FILE)
        c.require(isinstance(item['sha256'], str) and re.fullmatch('[0-9a-f]{64}', item['sha256']))
        if name not in changed and name not in new_web:
            c.require(item == old['files'][name])
        directories.update(p.as_posix() for p in path.parents if str(p) != '.')
    c.inspect(root, c.UID, 0o700, True)
    actual, dirs = c.scan(root, c.UID)
    c.require(dirs == directories and set(actual) == set(files) | {'manifest.json'})
    c.require(all(actual[name] == item for name, item in files.items()))
    for directory in dirs:
        c.inspect(root / directory, c.UID, 0o700, True)
    for name in actual:
        c.inspect(root / name, c.UID, 0o755 if name in c.BINARIES | {'bin/studiolo-vault-admin'} else 0o600)
    c.require(c.read(root / 'bin/studiolo-vault-admin', c.UID, 0o755).startswith(b'\x7fELF'))
    c.require(c.read(root / 'bin/studiolo-local-server', c.UID, 0o755).startswith(b'\x7fELF'))
    c.require(c.read(root / 'manifest.json', c.UID, 0o600) == raw)
    return manifest


# 黑板数据目录只允许出现控制库本体及其 WAL/SHM；停服时 SQLite 可能收掉后两个。
BOARD_FILES = {'control.sqlite3', 'control.sqlite3-wal', 'control.sqlite3-shm'}


def board_files(c, board):
    """起点是已在跑黑板的发行：目录里必须有控制库、不得有别的东西，每个文件私有且单链接。

    r8→r2 那一次是首建，当时要求这个目录为空；r2 起黑板已接单，库里是真实的认领与回执，
    因此「为空」会把每一次后续发行挡在预检，而「随便有什么都行」又会放过被篡改的目录。
    本函数在预检与装单元前各跑一次（binding 被调用两次），停服后 WAL/SHM 消失同样通过。
    """
    names = {path.name for path in board.iterdir()}
    c.require('control.sqlite3' in names and names <= BOARD_FILES)
    for name in sorted(names):
        c.inspect(board / name, c.UID, 0o600)


def binding(c, a):
    """逐项核验 Broker 的不可变配对；不打开写者，不补写任何目录。"""
    vault, writer, board = [Path(p) for p in (a.VAULT, a.WRITER, a.BOARD)]
    roots = []
    for path in (vault, writer, board):
        roots.append(c.inspect(path, c.UID, 0o700, True))
    c.require(len({(s.st_dev, s.st_ino) for s in roots}) == 3)
    for left in (vault, writer, board):
        for right in (vault, writer, board):
            c.require(left == right or not left.is_relative_to(right))
    board_files(c, board)
    control = vault / '.workbench'
    left = c.document(c.read(control / 'broker-binding.json', c.UID, 0o600))
    right = c.document(c.read(writer / 'broker-binding.json', c.UID, 0o600))
    fields = {'format_version', 'pair_id', 'vault_uuid', 'canonical', 'data', 'control',
              'marker', 'staging', 'history', 'idempotency', 'snapshots'}
    c.require(isinstance(left, dict) and set(left) == fields and left == right)
    c.require(type(left['format_version']) is int and left['format_version'] == 1)
    for name in ('pair_id', 'vault_uuid'):
        c.require(str(uuid.UUID(left[name])) == left[name])
    paths = {'canonical': vault, 'data': writer, 'control': control,
             'marker': control / 'vault.json', 'staging': control / 'staging',
             **{name: writer / name for name in ('history', 'idempotency', 'snapshots')}}
    for key, path in paths.items():
        s = c.inspect(path, c.UID, 0o600 if key == 'marker' else 0o700, key != 'marker')
        c.require(left[key] == {'dev': s.st_dev, 'ino': s.st_ino})
        c.require(all(type(v) is int for v in left[key].values()))
    marker = c.document(c.read(control / 'vault.json', c.UID, 0o600))
    c.require(marker == {'format_version': 1, 'vault_uuid': left['vault_uuid']})
    c.read(writer / 'writer-state.json', c.UID, 0o600)
    return left


class Operations:
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

    def worker(self, mode, descriptors=()):
        # 复用原 r8 快照 worker；代码允许 root 冻结、祖先仍可由 UID1000 拥有。
        # 不修改 r8 helper 的通用文件规则，也不读取认证数据到当前进程。
        c = self.c
        c.require(mode in ('binding', 'snapshot') and len(descriptors) == (3 if mode == 'snapshot' else 0))
        account = pwd.getpwnam('maxsong')
        c.require(account.pw_uid == c.UID == 1000)
        code = self.trusted_read(self.legacy.WORKER)
        c.require(len(code) <= 1024 * 1024)
        result = subprocess.run(['/usr/bin/python3', '-I', '-B', '-S', '-', mode,
            *(str(fd) for fd in descriptors)], input=code, stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL, pass_fds=tuple(descriptors), user=c.UID,
            group=account.pw_gid, extra_groups=[], cwd='/', env=self.runtime.ENV,
            timeout=40, check=True)
        c.require(len(result.stdout) <= 4096)
        value = c.document(result.stdout)
        c.require(set(value) == {'epoch', 'factor_ref', 'code_count', 'restore_event_count'})
        return value

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
        # 既有 agent 发行包可由 root 冻结；不改变其所有权或文件。
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

    def preflight(self):
        c, a = self.c, self.a
        c.require(sys.platform == 'linux' and os.getuid() == os.geteuid() == 0)
        c.require(pwd.getpwnam('maxsong').pw_uid == c.UID == 1000)
        self.manifest = verify_candidate(a, c, self.manifest_sha)
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
        self.runtime.command(['systemctl', 'stop', 'studiolo-agent.service'], 110)
        self.runtime.command(['systemctl', 'stop', 'studiolo-human.service'], 110)
        self.stopped()

    def backup(self):
        c = self.c
        self.stopped()
        self.unchanged()
        c.require({name: c.sha(raw) for name, raw in self.units().items()} == self.old_shas)
        for name in NAMES:
            c.new_file(self.record / (name + '.before.service'), self.old_units[name])
        c.new_file(self.record / 'owner.before.json', self.config)
        descriptors = []
        paths = [self.record / (name + '.sqlite3') for name in ('auth', 'audit', 'policy')]
        try:
            for path in paths:
                descriptors.append(os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600))
            c.require(self.worker('snapshot', descriptors) == self.before)
        finally:
            for fd in descriptors:
                os.close(fd)
        files = {}
        for path in paths:
            c.inspect(path, 0, 0o600)
            db = sqlite3.connect(path.as_uri() + '?mode=ro&immutable=1', uri=True)
            try:
                c.require(db.execute('PRAGMA quick_check').fetchall() == [('ok',)])
                c.require(not db.execute('PRAGMA foreign_key_check').fetchall())
            finally:
                db.close()
        for path in paths + [self.record / (name + '.before.service') for name in NAMES] + [self.record / 'owner.before.json']:
            raw = c.read(path, 0, 0o600)
            files[path.name] = {'bytes': len(raw), 'sha256': c.sha(raw)}
        self.unchanged()
        self.receipt('backup', {'state': 'complete', 'files': files, 'sqlite_write_barrier': True,
                                'agent_identity_database_changed': False})

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
