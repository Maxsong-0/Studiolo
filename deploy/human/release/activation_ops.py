"""r8操作后端：只管理human单元，失败保留本批root私有证据。"""
import fcntl
import json
import os
from pathlib import Path
import pwd
import sqlite3
import subprocess
import tempfile
import release_common as c
import runtime_checks as runtime

CONFIG = Path('/home/maxsong/.config/studiolo-human/owner.json')
RECORDS = Path('/var/lib/studiolo-deployments')
AUTH = '/home/maxsong/.local/share/studiolo-human-auth'
AUDIT = '/home/maxsong/.local/share/studiolo-human-audit'
WORKER = Path(__file__).parent / 'snapshot_worker.py'


class Operations:
    def __init__(self, manifest_sha):
        self.manifest_sha = manifest_sha
        self.record = None
        self.lock = None
        self.before = None
        self.config = None
        self.old_unit = None
        self.new_unit = None
        self.new_sha = None

    def begin(self):
        c.require(os.getuid() == os.geteuid() == 0)
        c.inspect(RECORDS, 0, directory=True)
        path = RECORDS / '.human-r8.lock'
        self.lock = os.open(path, os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW | os.O_CLOEXEC, 0o600)
        c.inspect(path, 0, 0o600)
        fcntl.flock(self.lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        self.record = Path(tempfile.mkdtemp(prefix='human-r8-', dir=RECORDS))
        c.inspect(self.record, 0, 0o700, True)

    def worker(self, mode, descriptors=()):
        account = pwd.getpwnam('maxsong')
        c.require(account.pw_uid == c.UID)
        owner = WORKER.lstat().st_uid
        c.require(owner in (0, c.UID))
        code = c.read(WORKER, owner)
        # 真实数据库字节只经继承FD写入；stdout仅固定非秘密统计。
        result = subprocess.run(['/usr/bin/python3', '-I', '-B', '-S', '-', mode,
                                 *(str(fd) for fd in descriptors)], input=code,
                                stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
                                pass_fds=tuple(descriptors), user=c.UID, group=account.pw_gid,
                                extra_groups=[], cwd='/', env=runtime.ENV, timeout=40, check=True)
        c.require(len(result.stdout) <= 4096)
        value = c.document(result.stdout)
        c.require(set(value) == {'epoch', 'factor_ref', 'code_count', 'restore_event_count'})
        return value

    def preflight(self):
        self.manifest = c.verify(c.RELEASE, self.manifest_sha)
        self.old_unit = c.read(runtime.UNIT, 0, 0o644)
        c.require(c.sha(self.old_unit) == runtime.OLD_UNIT)
        self.new_unit = c.read(c.RELEASE / 'studiolo-human.service', c.UID, 0o600)
        self.new_sha = c.sha(self.new_unit)
        c.require(self.new_sha == self.manifest['files']['studiolo-human.service']['sha256'])
        # 除固定RELEASE路径外，systemd安全边界与旧单元逐字相同。
        c.require(self.new_unit == self.old_unit.replace(c.OLD_RELEASE.encode(), str(c.RELEASE).encode()))
        self.config = c.read(CONFIG, c.UID, 0o600)
        document = c.document(self.config)
        c.require(document['seed_ref'] == 'studiolo-human-totp-v2')
        c.require(document['auth_directory'] == AUTH and document['audit_directory'] == AUDIT)
        c.inspect(CONFIG.parent, c.UID, 0o700, True)
        c.inspect(Path('/etc/credstore.encrypted/studiolo-human-totp-v2'), 0, 0o600)
        self.before = self.worker('binding')
        runtime.command(['systemd-analyze', 'verify', c.RELEASE / 'studiolo-human.service'])
        runtime.ready(Path(c.OLD_RELEASE) / 'bin/studiolo-local-server', runtime.OLD_BINARY)
        self.receipt('preflight', {'before': self.before, 'manifest_sha256': self.manifest_sha})

    def stop(self):
        runtime.command(['systemctl', 'stop', runtime.SERVICE], 110)
        runtime.stopped()

    def unchanged(self):
        c.require(c.read(CONFIG, c.UID, 0o600) == self.config)
        c.require(self.worker('binding') == self.before)

    def backup(self):
        runtime.stopped()
        self.unchanged()
        c.require(c.sha(c.read(runtime.UNIT, 0, 0o644)) == runtime.OLD_UNIT)
        c.new_file(self.record / 'human.before.service', self.old_unit)
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
            raw = c.read(path, 0, 0o600)
            files[path.name] = {'bytes': len(raw), 'sha256': c.sha(raw)}
        for name in ('human.before.service', 'owner.before.json'):
            raw = c.read(self.record / name, 0, 0o600)
            files[name] = {'bytes': len(raw), 'sha256': c.sha(raw)}
        self.unchanged()
        self.receipt('backup', {'state': 'complete', 'files': files, 'sqlite_write_barrier': True})

    def install(self):
        c.verify(c.RELEASE, self.manifest_sha)
        self.unchanged()
        c.atomic_unit(runtime.UNIT, self.new_unit, runtime.OLD_UNIT)
        runtime.command(['systemctl', 'daemon-reload'])

    def start(self):
        runtime.command(['systemctl', 'start', runtime.SERVICE], 30)

    def accept(self):
        binary = c.RELEASE / 'bin/studiolo-local-server'
        pid = runtime.ready(binary, self.manifest['files']['bin/studiolo-local-server']['sha256'])
        runtime.denials()
        self.unchanged()
        c.require(c.sha(c.read(runtime.UNIT, 0, 0o644)) == self.new_sha)
        c.verify(c.RELEASE, self.manifest_sha)
        return {'pid': pid, 'after': self.before, 'sole_credential': 'totp-next',
                'anonymous_endpoints_401': 6, 'unit_sha256': self.new_sha}

    def recover(self, unit_may_have_changed):
        # 只恢复程序。未知unit/配置/因子变更或异常socket一律拒绝自动处理。
        current = c.sha(c.read(runtime.UNIT, 0, 0o644))
        c.require(current in (runtime.OLD_UNIT, self.new_sha))
        if unit_may_have_changed:
            self.stop()
        self.unchanged()
        if current != runtime.OLD_UNIT:
            c.atomic_unit(runtime.UNIT, self.old_unit, current)
            runtime.command(['systemctl', 'daemon-reload'])
        else:
            c.require(c.sha(c.read(runtime.UNIT, 0, 0o644)) == runtime.OLD_UNIT)
        values = runtime.properties()
        if values['ActiveState'] not in ('active', 'activating'):
            runtime.stopped()
            self.start()
        runtime.ready(Path(c.OLD_RELEASE) / 'bin/studiolo-local-server', runtime.OLD_BINARY)
        runtime.denials()
        self.unchanged()
        return 'old_program_restored'

    def receipt(self, name, value):
        c.require(name in ('preflight', 'backup', 'accepted', 'failed'))
        c.new_file(self.record / (name + '.json'), (json.dumps(value, sort_keys=True, indent=2) + '\n').encode())
        fd = os.open(self.record, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(fd)
        finally:
            os.close(fd)

    def close(self):
        if self.lock is not None:
            os.close(self.lock)
            self.lock = None
