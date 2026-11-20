"""激活预检里的备份前置核对（T-216，r17 起）：缺口要在停服之前拦下。

每条拒绝用例都从同一份「全备好」的现场出发、只改一处，并写明拒绝时该是哪一种异常；全备好能过由第一条用例证明。
两个目录的属主、属组普通账号改不了：这几条用 `reported` 只替换预检看到的那两个目录的 stat，别的路径照常。
"""
from .fixture import *

ABSENT = FileNotFoundError


class BackupReadyTests(FilesBase):
    def setUp(self):
        super().setUp()
        self.config = self.root / 'config'
        self.runtime_dir, self.staging = self.root / 'run-backup', self.root / 'staging'
        self.ephemeral, self.private = self.root / 'ephemeral', self.root / 'private-tmp'
        self.reader = None
        self.directory(self.runtime_dir, 0o750)
        # 读取组取目录实际的属组。以 root 组跑（容器默认）时是 0，而读取组不许是 0：
        # root 改得了属组，换成非 0 的组接着测，不跳过；改不了就让用例直接报错。
        self.reader = self.runtime_dir.lstat().st_gid or 1
        self.regroup(self.runtime_dir)
        self.directory(self.staging, 0o750)
        self.directory(self.ephemeral, 0o700)
        self.directory(self.private, 0o700)
        self.groups = {self.reader}
        # 预检看到的两个目录的 stat 里要替换的字段；空着就是真实的。
        self.reported = {}
        self.a = types.SimpleNamespace(
            BUILD=assembly.BUILD, HUMAN_SHA='0' * 64, AGENT_SHA='0' * 64,
            BACKUP_CONFIG=str(self.config / 'backup.json'),
            SYNC_APPROVAL=str(self.config / 'sync-transport.json'),
            BACKUP_RUNTIME=str(self.runtime_dir), BACKUP_STAGING=str(self.staging),
            BACKUP_FREEZE_BUDGET_SECONDS=assembly.BACKUP_FREEZE_BUDGET_SECONDS,
            BRIDGE_STOP_GRACE_SECONDS=assembly.BRIDGE_STOP_GRACE_SECONDS)
        self.backup(self.producer())
        self.approval(assembly.BRIDGE_STOP_GRACE_SECONDS)

    def regroup(self, path):
        if self.reader is not None and path.lstat().st_gid != self.reader:
            os.chown(path, -1, self.reader)

    def directory(self, path, mode):
        path.mkdir(parents=True)
        path.chmod(mode)
        self.regroup(path)

    def producer(self, **changes):
        """和程序认的结构同形（crates/host-config/src/backup/producer.rs）；值是虚构的。传 None 表示去掉这个键。"""
        document = {'version': 1, 'worker_uid': 990, 'reader_gid': self.reader,
                    'socket': str(self.runtime_dir / 'producer.sock'),
                    'staging_parent': str(self.staging), 'ephemeral_root': str(self.ephemeral),
                    'source_retention_millis': 3600000,
                    'freeze_budget_millis': assembly.BACKUP_FREEZE_BUDGET_SECONDS * 1000,
                    'application_packages': [{'name': 'fictional', 'version': '1', 'digest': '0' * 64}],
                    'restic_digest': '0' * 64,
                    'deployment': [{'unit': 'fictional.service', 'revision': '1'}]}
        document.update(changes)
        return {name: value for name, value in document.items() if value is not None}

    def replace(self, name, document, mode=0o600):
        path = self.config / name
        if path.exists():
            path.unlink()
        self.write(path, json.dumps(document).encode(), mode)

    def backup(self, producer, mode=0o600, **top):
        """`top` 改顶层的键；传 None 表示去掉。"""
        document = {'version': 1, 'backup': {'fictional_approval': True}, 'backup_producer': producer}
        document.update(top)
        self.replace('backup.json', {k: v for k, v in document.items() if v is not None}, mode)

    def approval(self, grace):
        self.replace('sync-transport.json', {'budget': {'stop_grace_seconds': grace}})

    def ready(self):
        ops = Operations('0' * 64, self.a, c, Mock(), legacy)
        real, targets = os.lstat, {str(self.runtime_dir), str(self.staging)}

        def lstat(path, *args, **kwargs):
            actual = real(path, *args, **kwargs)
            if not self.reported or os.fspath(path) not in targets:
                return actual
            fields = dict(st_mode=actual.st_mode, st_uid=actual.st_uid, st_gid=actual.st_gid)
            fields.update(self.reported)
            return types.SimpleNamespace(**fields)
        # 测试目录本身就在 /tmp 下：把「服务里看不见的目录」换成沙盒里的一处，核对逻辑不变
        # （真实取值由 OperationTests 钉）。
        with patch.object(ops, 'owner_groups', return_value=self.groups), \
                patch.object(ops_module, 'PRIVATE_TMP', (self.private,)), \
                patch.object(os, 'lstat', side_effect=lstat):
            ops.backup_ready()
        ops.runtime.command.assert_not_called()

    def rejected(self, kind=ValueError):
        """拒绝，而且恰好是这一种异常：核对没过是 ValueError，路径不存在是 FileNotFoundError。"""
        with self.assertRaises(kind) as caught:
            self.ready()
        self.assertIs(type(caught.exception), kind)

    def test_everything_in_place_passes(self):
        self.ready()
        # 预算配得比装配假设的小也行（停止超时按上限算），小到程序接受的下限为止。
        self.backup(self.producer(freeze_budget_millis=ops_module.FREEZE_BUDGET_FLOOR_MILLIS))
        self.approval(ops_module.STOP_GRACE_FLOOR_SECONDS)
        self.ready()

    def test_private_config_missing_or_wrong_mode_is_rejected(self):
        self.backup(self.producer(), mode=0o644)
        self.rejected()
        (self.config / 'backup.json').unlink()
        self.rejected(ABSENT)

    def test_document_must_have_exactly_the_keys_the_program_reads(self):
        for name in ops_module.BACKUP_DOCUMENT_KEYS:
            with self.subTest(missing=name):
                self.backup(self.producer(), **{name: None})
                self.rejected()
        for name in ops_module.BACKUP_PRODUCER_KEYS:
            with self.subTest(missing_producer=name):
                self.backup(self.producer(**{name: None}))
                self.rejected()
        # 拼错的键在程序里是「未知字段」，一样拒绝启动。
        self.backup(self.producer(), backup_producers={})
        self.rejected()
        self.backup(self.producer(freeze_budget_milis=30000))
        self.rejected()
        self.replace('backup.json', [self.producer()])
        self.rejected()
        for bad in ([self.producer()], 'producer', 7):
            with self.subTest(producer=type(bad).__name__):
                self.backup(bad)
                self.rejected()
        self.backup(self.producer())
        self.ready()

    def test_versions_and_the_approval_section_are_checked(self):
        for bad in (2, 0, '1', 1.0, True):
            with self.subTest(version=bad):
                self.backup(self.producer(), version=bad)
                self.rejected()
            with self.subTest(producer_version=bad):
                self.backup(self.producer(version=bad))
                self.rejected()
        for bad in ({}, [], 'approved', 1):
            with self.subTest(approval=bad):
                self.backup(self.producer(), backup=bad)
                self.rejected()

    def test_owner_must_be_in_the_reader_group(self):
        self.groups = {self.reader + 1}
        self.rejected()
        self.groups = set()
        self.rejected()

    def test_reader_group_must_be_a_positive_integer(self):
        # 目录属组和账号的组都报成这个坏值：能拦下它的只剩「必须是正整数」这一条。
        for bad in (0, -1, True):
            with self.subTest(reader=bad):
                self.groups, self.reported = {bad}, {'st_gid': bad}
                self.backup(self.producer(reader_gid=bad))
                self.rejected()
        # 对照：同样的替换配上一个正常的组号就通过，说明上面不是被替换本身拦下的。
        other = self.reader + 1
        self.groups, self.reported = {other}, {'st_gid': other}
        self.backup(self.producer(reader_gid=other))
        self.ready()
        self.groups, self.reported = {self.reader}, {}
        for bad in (str(self.reader), float(self.reader)):
            with self.subTest(reader=bad):
                self.backup(self.producer(reader_gid=bad))
                self.rejected()

    def test_directories_must_be_owned_by_the_owner_and_the_reader_group_with_0750(self):
        # 属主不是人侧账号（比如 sudo 手建之后忘了改属主）：程序绑 socket 时会拒绝启动。
        self.reported = {'st_uid': self.uid + 1}
        self.rejected()
        self.reported = {}
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
            moved = path.with_name(path.name + '-moved')
            path.rename(moved)
            with self.subTest(path=path.name, missing=True):
                self.rejected(ABSENT)
            # 同名、同属主属组、同权限的普通文件：不是目录也要拒。
            self.write(path, b'', 0o750)
            self.regroup(path)
            with self.subTest(path=path.name, regular_file=True):
                self.rejected()
            path.unlink()
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
        # 路径写成了别的类型：按核对没过拒绝，不让类型错误冒出来。
        for name in ('socket', 'staging_parent', 'ephemeral_root'):
            for bad in (7, ['/x'], True):
                with self.subTest(name=name, value=bad):
                    self.backup(self.producer(**{name: bad}))
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
        for bad in (str(hidden), str(self.private), str(linked), 'ephemeral'):
            with self.subTest(ephemeral=bad):
                self.backup(self.producer(ephemeral_root=bad))
                self.rejected()
        self.backup(self.producer(ephemeral_root=str(self.root / 'absent')))
        self.rejected(ABSENT)
        # 同一个目录，只要不算「服务里看不见」的就通过：上面那两条确实是因为落在私有临时目录里才被拒。
        self.backup(self.producer(ephemeral_root=str(hidden)))
        self.private = self.root / 'unrelated'
        self.ready()

    def test_freeze_budget_must_fit_the_program_and_the_stop_timeout(self):
        floor, limit = ops_module.FREEZE_BUDGET_FLOOR_MILLIS, assembly.BACKUP_FREEZE_BUDGET_SECONDS * 1000
        for bad in (limit + 1, 120000, floor - 1, 0, -1, str(limit), float(limit), True):
            with self.subTest(freeze=bad):
                self.backup(self.producer(freeze_budget_millis=bad))
                self.rejected()
        for good in (floor, limit):
            with self.subTest(freeze=good):
                self.backup(self.producer(freeze_budget_millis=good))
                self.ready()

    def test_bridge_stop_grace_must_fit_the_program_and_the_stop_timeout(self):
        floor, limit = ops_module.STOP_GRACE_FLOOR_SECONDS, assembly.BRIDGE_STOP_GRACE_SECONDS
        for bad in (limit + 1, floor - 1, -1, str(limit), float(limit), True):
            with self.subTest(grace=bad):
                self.approval(bad)
                self.rejected()
        for good in (floor, limit):
            with self.subTest(grace=good):
                self.approval(good)
                self.ready()
        # 审批文件形状不对（没有预算段、预算段不是对象、整份不是对象）：同样按核对没过拒绝。
        for document in ({}, {'budget': []}, {'budget': {}}, [{'budget': {'stop_grace_seconds': limit}}]):
            with self.subTest(document=document):
                self.replace('sync-transport.json', document)
                self.rejected()
        (self.config / 'sync-transport.json').unlink()
        self.rejected(ABSENT)
