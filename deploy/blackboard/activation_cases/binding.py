"""binding 发行契约。"""
from .fixture import *


class BindingTests(FilesBase):
    def test_failed_new_human_start_restores_old_binary_with_its_actual_file_digest(self):
        a, _ = self.candidate()
        ops = Operations('0' * 64, a, c, Mock(), legacy)
        old_binary = Path(a.OLD_RELEASE) / 'bin/studiolo-local-server'
        new_binary = Path(a.RELEASE) / 'bin/studiolo-local-server'
        ops.old_human_sha = c.sha(old_binary.read_bytes())
        ops.human_sha = c.sha(new_binary.read_bytes())
        # r3 沿用 r2 的程序，两份摘要相同；回滚路径仍须按实际文件摘要恢复。
        self.assertEqual(ops.old_human_sha, ops.human_sha)
        ops.old_units = {'studiolo-human': b'old human', 'studiolo-agent': b'old agent'}
        ops.new_units = {'studiolo-human': b'new human', 'studiolo-agent': b'new agent'}
        ops.old_shas = {name: c.sha(raw) for name, raw in ops.old_units.items()}
        ops.new_shas = {name: c.sha(raw) for name, raw in ops.new_units.items()}
        current = dict(ops.new_units)
        verified = []
        def ready(binary, digest):
            # 只替身服务状态；摘要必须与真实临时二进制逐字节匹配。
            c.require(c.sha(c.read(binary, self.uid, 0o755)) == digest)
            verified.append(binary)
            if binary == new_binary:
                raise ValueError('fictional new human startup failure')
            return 123
        def replace(path, raw, expected):
            name = path.stem
            c.require(c.sha(current[name]) == expected)
            current[name] = raw
        ops.record = self.root
        ops.runtime.ready.side_effect = ready
        with patch.object(ops, 'begin'), patch.object(ops, 'preflight'), patch.object(ops, 'stop'), \
                patch.object(ops, 'backup'), patch.object(ops, 'install'), patch.object(ops, 'unchanged'), \
                patch.object(ops, 'properties'), patch.object(ops, 'units', side_effect=lambda: dict(current)), \
                patch.object(c, 'atomic_unit', side_effect=replace), patch.object(ops, 'agent_ready'), \
                patch.object(ops, 'receipt'), patch.object(signal, 'signal'):
            code, result = activation.deploy(ops)
        self.assertEqual(code, 1)
        self.assertEqual(result['stage'], 'start-human')
        self.assertEqual(result['program_recovery'], 'old_programs_restored')
        self.assertEqual(verified, [new_binary, old_binary])
        self.assertEqual(current, ops.old_units)

    def test_running_board_with_only_its_control_database_accepted(self):
        a = self.broker()
        board = Path(a.BOARD)
        self.assertEqual(ops_module.binding(c, a)['format_version'], 1)
        # 在跑时带 WAL/SHM，停服后 SQLite 可能把它们收掉：两种样子都得认。
        for name in ('control.sqlite3-wal', 'control.sqlite3-shm'):
            self.write(board / name, b'fictional journal')
        self.assertEqual(ops_module.binding(c, a)['format_version'], 1)

    def test_board_without_control_database_or_with_strangers_rejected(self):
        a = self.broker()
        board = Path(a.BOARD)
        (board / 'control.sqlite3').unlink()
        with self.subTest(case='空目录（首建的样子，已不是本次起点）'), self.assertRaises(ValueError):
            ops_module.binding(c, a)
        self.write(board / 'control.sqlite3', b'fictional board')
        self.write(board / 'existing', b'preserve')
        with self.subTest(case='多出陌生文件'), self.assertRaises(ValueError):
            ops_module.binding(c, a)
        (board / 'existing').unlink()
        (board / 'control.sqlite3').chmod(0o644)
        with self.subTest(case='控制库权限变宽'), self.assertRaises(ValueError):
            ops_module.binding(c, a)

    def test_missing_writer_binding_rejected_without_creating_it(self):
        a = self.broker()
        path = Path(a.WRITER) / 'broker-binding.json'
        path.unlink()
        with self.assertRaises(FileNotFoundError):
            ops_module.binding(c, a)
        self.assertFalse(path.exists())

    def test_wrong_inode_rejected(self):
        a = self.broker()
        for path in (Path(a.VAULT) / '.workbench/broker-binding.json', Path(a.WRITER) / 'broker-binding.json'):
            doc = json.loads(path.read_bytes())
            doc['canonical']['ino'] += 1
            path.write_bytes(json.dumps(doc).encode())
        with self.assertRaises(ValueError):
            ops_module.binding(c, a)


