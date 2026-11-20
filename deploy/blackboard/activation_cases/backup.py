"""激活预检里的备份前置核对（T-216，r17 起）：缺口要在停服之前拦下。

每条拒绝用例都从同一份「全备好」的现场出发、只改一处；全备好能过由第一条用例证明。
"""
from .fixture import *

# 预检里读文件、读目录、取字段失败各自的异常；都发生在任何服务命令之前，激活脚本一律按拒绝处理。
REJECTED = (ValueError, OSError, KeyError, TypeError)


class BackupReadyTests(FilesBase):
    def setUp(self):
        super().setUp()
        self.config = self.root / 'config'
        self.runtime_dir, self.staging = self.root / 'run-backup', self.root / 'staging'
        self.ephemeral, self.private = self.root / 'ephemeral', self.root / 'private-tmp'
        self.directory(self.runtime_dir, 0o750)
        self.directory(self.staging, 0o750)
        self.directory(self.ephemeral, 0o700)
        self.directory(self.private, 0o700)
        # 读取组取目录实际的属组：普通账号改不了属组，用例也不需要改。
        self.reader = self.runtime_dir.lstat().st_gid
        if self.reader == 0:
            super().tearDown()
            self.skipTest('目录属组是 0：读取组不许是 0，这组用例要在非 root 组下跑')
        self.groups = {self.reader}
        self.a = types.SimpleNamespace(
            BUILD=assembly.BUILD, HUMAN_SHA='0' * 64, AGENT_SHA='0' * 64,
            BACKUP_CONFIG=str(self.config / 'backup.json'),
            SYNC_APPROVAL=str(self.config / 'sync-transport.json'),
            BACKUP_RUNTIME=str(self.runtime_dir), BACKUP_STAGING=str(self.staging),
            BACKUP_FREEZE_BUDGET_SECONDS=assembly.BACKUP_FREEZE_BUDGET_SECONDS,
            BRIDGE_STOP_GRACE_SECONDS=assembly.BRIDGE_STOP_GRACE_SECONDS)
        self.backup(self.producer())
        self.approval(assembly.BRIDGE_STOP_GRACE_SECONDS)

    def directory(self, path, mode):
        path.mkdir(parents=True)
        path.chmod(mode)

    def producer(self, **changes):
        document = {'worker_uid': 990, 'reader_gid': self.reader,
                    'socket': str(self.runtime_dir / 'producer.sock'),
                    'staging_parent': str(self.staging), 'ephemeral_root': str(self.ephemeral),
                    'source_retention_millis': 3600000,
                    'freeze_budget_millis': assembly.BACKUP_FREEZE_BUDGET_SECONDS * 1000}
        document.update(changes)
        return {name: value for name, value in document.items() if value is not None}

    def replace(self, name, document, mode=0o600):
        path = self.config / name
        if path.exists():
            path.unlink()
        self.write(path, json.dumps(document).encode(), mode)

    def backup(self, producer, mode=0o600):
        self.replace('backup.json', {'version': 1, 'backup': {}, 'backup_producer': producer}, mode)

    def approval(self, grace):
        self.replace('sync-transport.json', {'budget': {'stop_grace_seconds': grace}})

    def ready(self):
        ops = Operations('0' * 64, self.a, c, Mock(), legacy)
        # 测试目录本身就在 /tmp 下：把「服务里看不见的目录」换成沙盒里的一处，核对逻辑不变。
        with patch.object(ops, 'owner_groups', return_value=self.groups), \
                patch.object(ops_module, 'PRIVATE_TMP', (self.private,)):
            ops.backup_ready()
        ops.runtime.command.assert_not_called()

    def rejected(self):
        with self.assertRaises(REJECTED):
            self.ready()

    def test_everything_in_place_passes(self):
        self.ready()
        # 预算配得比装配假设的小也行：停止超时是按上限算的。
        self.backup(self.producer(freeze_budget_millis=1000))
        self.approval(0)
        self.ready()

    def test_private_config_missing_or_wrong_mode_is_rejected(self):
        self.backup(self.producer(), mode=0o644)
        self.rejected()
        (self.config / 'backup.json').unlink()
        self.rejected()

    def test_missing_producer_fields_are_rejected(self):
        for name in ('reader_gid', 'socket', 'staging_parent', 'ephemeral_root', 'freeze_budget_millis'):
            with self.subTest(missing=name):
                self.backup(self.producer(**{name: None}))
                self.rejected()
        self.replace('backup.json', {'version': 1, 'backup': {}})
        self.rejected()

    def test_owner_must_be_in_the_reader_group(self):
        self.groups = {self.reader + 1}
        self.rejected()
        self.groups = set()
        self.rejected()

    def test_reader_group_must_be_a_positive_integer(self):
        for bad in (str(self.reader), float(self.reader), True, 0, -1):
            with self.subTest(reader=bad):
                self.groups = {self.reader, bad}
                self.backup(self.producer(reader_gid=bad))
                self.rejected()

    def test_directories_must_be_owned_by_the_reader_group_with_0750(self):
        # 配置里的读取组换成另一个号（账号也在那个组里）：两个目录的属组就对不上了。
        self.groups = {self.reader + 1}
        self.backup(self.producer(reader_gid=self.reader + 1))
        self.rejected()
        self.groups = {self.reader}
        self.backup(self.producer())
        for path in (self.runtime_dir, self.staging):
            for mode in (0o700, 0o755, 0o770):
                with self.subTest(path=path.name, mode=oct(mode)):
                    path.chmod(mode)
                    self.rejected()
                    path.chmod(0o750)
            with self.subTest(path=path.name, missing=True):
                moved = path.with_name(path.name + '-moved')
                path.rename(moved)
                self.rejected()
                moved.rename(path)
        self.ready()

    def test_config_must_point_at_the_directories_the_unit_opens_for_writing(self):
        elsewhere = self.root / 'elsewhere'
        self.directory(elsewhere, 0o750)
        self.backup(self.producer(staging_parent=str(elsewhere)))
        self.rejected()
        self.backup(self.producer(socket=str(elsewhere / 'producer.sock')))
        self.rejected()
        self.backup(self.producer(socket='run-backup/producer.sock'))
        self.rejected()

    def test_leftover_socket_node_is_rejected_and_left_alone(self):
        node = self.runtime_dir / 'producer.sock'
        self.write(node, b'leftover')
        self.rejected()
        self.assertEqual(node.read_bytes(), b'leftover')
        node.unlink()
        self.ready()

    def test_ephemeral_root_must_exist_private_and_visible_to_the_service(self):
        for mode in (0o750, 0o755):
            with self.subTest(mode=oct(mode)):
                self.ephemeral.chmod(mode)
                self.rejected()
                self.ephemeral.chmod(0o700)
        hidden = self.private / 'ephemeral'
        self.directory(hidden, 0o700)
        linked = self.root / 'linked'
        linked.symlink_to(self.ephemeral)
        for bad in (str(self.root / 'absent'), str(hidden), str(self.private), str(linked), 'ephemeral'):
            with self.subTest(ephemeral=bad):
                self.backup(self.producer(ephemeral_root=bad))
                self.rejected()
        # 同一个目录，只要不算「服务里看不见」的就通过：上面那两条确实是因为落在私有临时目录里才被拒。
        self.backup(self.producer(ephemeral_root=str(hidden)))
        self.private = self.root / 'unrelated'
        self.ready()

    def test_budgets_larger_than_the_stop_timeout_assumes_are_rejected(self):
        limit = assembly.BACKUP_FREEZE_BUDGET_SECONDS * 1000
        for bad in (limit + 1, 120000, 0, -1, str(limit), float(limit), True):
            with self.subTest(freeze=bad):
                self.backup(self.producer(freeze_budget_millis=bad))
                self.rejected()
        self.backup(self.producer())
        for bad in (assembly.BRIDGE_STOP_GRACE_SECONDS + 1, -1, '30', 30.0, True):
            with self.subTest(grace=bad):
                self.approval(bad)
                self.rejected()
        (self.config / 'sync-transport.json').unlink()
        self.rejected()
