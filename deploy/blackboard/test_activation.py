#!/usr/bin/python3
"""虚构文件与注入服务替身；不访问生产路径、网络、凭据或 systemd。"""
import importlib.util
import json
import os
from pathlib import Path
import signal
import sys
import tempfile
import types
import unittest
from unittest.mock import Mock, patch

sys.dont_write_bytecode = True
HERE = Path(__file__).resolve().parent


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


c = load('release_common', HERE.parent / 'human/release/release_common.py')
runtime = load('runtime_checks', HERE.parent / 'human/release/runtime_checks.py')
legacy = load('test_board_legacy', HERE.parent / 'human/release/activation_ops.py')
assembly = load('test_board_assembly', HERE / 'assembly.py')
activation = load('test_board_activate', HERE / 'activate.py')
ops_module = load('test_board_ops', HERE / 'activation_ops.py')
prepare = load('test_board_prepare', HERE / 'prepare.py')
Operations = ops_module.Operations


class Files(unittest.TestCase):
    def setUp(self):
        self.mask = os.umask(0o077)
        self.temp = tempfile.TemporaryDirectory(prefix='studiolo-board-fictional-')
        self.root = Path(self.temp.name).resolve()
        self.uid = os.geteuid()
        self.uid_patch = patch.object(c, 'UID', self.uid)
        self.uid_patch.start()
        # 临时沙盒的 /tmp 祖先不同于生产：只替换祖先遍历，保留文件/节点安全检查。
        def ancestors(path, uid):
            self.assertTrue(path.is_relative_to(self.root))
            for parent in path.parents:
                if parent == self.root.parent:
                    break
                s = parent.lstat()
                c.require(parent.is_dir() and not parent.is_symlink()
                          and s.st_uid == uid and s.st_mode & 0o7022 == 0)
        self.ancestor_patch = patch.object(c, 'ancestors', side_effect=ancestors)
        self.ancestor_patch.start()

    def tearDown(self):
        self.ancestor_patch.stop()
        self.uid_patch.stop()
        self.temp.cleanup()
        os.umask(self.mask)

    def write(self, path, raw, mode=0o600):
        path.parent.mkdir(parents=True, mode=0o700, exist_ok=True)
        c.new_file(path, raw, mode)

    def begin_record(self, directory):
        ops = Operations('0' * 64, assembly, c, Mock(), legacy)
        inspect = c.inspect
        # 不需要 root 跑单测：只将预期 root UID 映射为临时文件的真实主人。
        # 原 inspect 对真实 lstat 的类型、mode、链接和所有权检查全部保留。
        def inspect_fixture(path, uid, *args, **kwargs):
            self.assertEqual(uid, 0)
            return inspect(path, self.uid, *args, **kwargs)
        try:
            with patch.object(ops_module, 'RECORDS', directory), patch.object(c, 'UID', 1000), \
                    patch.object(c, 'inspect', side_effect=inspect_fixture), \
                    patch.object(ops_module.sys, 'platform', 'linux'), \
                    patch.object(os, 'getuid', return_value=0), patch.object(os, 'geteuid', return_value=0), \
                    patch.object(ops_module.pwd, 'getpwnam', return_value=types.SimpleNamespace(pw_uid=1000)):
                ops.begin()
        except BaseException:
            ops.close()
            raise
        return ops

    def test_record_parent_0750_keeps_private_record_and_lock(self):
        directory = self.root / 'records'
        directory.mkdir(mode=0o700)
        directory.chmod(0o750)
        prior = directory / 'prior-group-visible'
        prior.mkdir(mode=0o700)
        prior.chmod(0o750)
        self.write(prior / 'receipt', b'preserve previous deployment')
        before = (prior.stat().st_mode, (prior / 'receipt').read_bytes())
        ops = self.begin_record(directory)
        try:
            self.assertEqual(directory.stat().st_mode & 0o7777, 0o750)
            self.assertEqual(ops.record.stat().st_mode & 0o7777, 0o700)
            self.assertEqual((directory / '.board-r1.lock').stat().st_mode & 0o7777, 0o600)
            self.assertEqual(ops.record.stat().st_uid, self.uid)
            self.assertEqual((prior.stat().st_mode, (prior / 'receipt').read_bytes()), before)
        finally:
            ops.close()

    def test_record_parent_group_or_other_write_refused_before_creation(self):
        for mode in (0o770, 0o752):
            with self.subTest(mode=oct(mode)):
                directory = self.root / ('records-' + oct(mode))
                directory.mkdir(mode=0o700)
                directory.chmod(mode)
                with self.assertRaises(ValueError):
                    self.begin_record(directory)
                self.assertEqual(list(directory.iterdir()), [])
                self.assertEqual(directory.stat().st_mode & 0o7777, mode)

    def test_record_parent_link_refused_without_touching_target(self):
        target = self.root / 'records-target'
        target.mkdir(mode=0o700)
        link = self.root / 'records-link'
        link.symlink_to(target, target_is_directory=True)
        with self.assertRaises(ValueError):
            self.begin_record(link)
        self.assertEqual(list(target.iterdir()), [])
        self.assertTrue(link.is_symlink())

    def test_existing_group_readable_lock_remains_rejected(self):
        directory = self.root / 'records'
        directory.mkdir(mode=0o700)
        directory.chmod(0o750)
        lock = directory / '.board-r1.lock'
        self.write(lock, b'', 0o640)
        with self.assertRaises(ValueError):
            self.begin_record(directory)
        self.assertEqual(list(directory.iterdir()), [lock])
        self.assertEqual(lock.stat().st_mode & 0o7777, 0o640)

    def candidate(self):
        """上一发行（r2 形状）与候选（r3）：只有 web/ 与启动器、人侧单元不同。"""
        old, new = self.root / 'old', self.root / 'new'
        def mode(name):
            return 0o755 if name in c.BINARIES | {'bin/studiolo-vault-admin'} else 0o600
        shared = {'bin/studiolo-local-server': b'\x7fELFfictional human',
                  'bin/studiolo-auth-admin': b'fictional admin',
                  'bin/studiolo-vault-admin': b'\x7fELFfictional bootstrap',
                  'studiolo-agent.service': b'old agent',
                  'example-vault/note.md': b'fictional note'}
        old_content = dict(shared, **{'launch-human.py': b'old launcher',
                                      'studiolo-human.service': b'old human',
                                      'web/index.html': b'stale web',
                                      'web/assets/stale.js': b'stale bundle'})
        for name, raw in old_content.items():
            self.write(old / name, raw, mode(name))
        files, _ = c.scan(old, self.uid)
        old_doc = dict(build='m5m6-board-20260920-r2', bootstrap_source_commit='2' * 40,
            human_program_source_commit='3' * 40, r8_manifest_sha256='0' * 64,
            assembly=assembly.ASSEMBLY,
            before_units={'studiolo-human': c.sha(b'older human'), 'studiolo-agent': c.sha(b'old agent')},
            files=files)
        raw = json.dumps(old_doc).encode()
        self.write(old / 'manifest.json', raw)
        a = types.SimpleNamespace(OLD_RELEASE=str(old), RELEASE=str(new), BUILD=assembly.BUILD,
            R8_MANIFEST_SHA=c.sha(raw), HUMAN_SHA=c.sha(b'old human'), AGENT_SHA=c.sha(b'old agent'),
            WEB_PREFIX=assembly.WEB_PREFIX, ASSEMBLY=assembly.ASSEMBLY)
        new_content = dict(shared, **{'launch-human.py': b'new launcher',
                                      'studiolo-human.service': b'new human',
                                      # agent 单元以现场为准：两次发行之间读取范围可能被改过。
                                      'studiolo-agent.service': b'current agent',
                                      'web/index.html': b'fresh web',
                                      'web/assets/fresh.js': b'fresh bundle'})
        for name, raw in new_content.items():
            self.write(new / name, raw, mode(name))
        files, _ = c.scan(new, self.uid)
        doc = dict(build=a.BUILD, bootstrap_source_commit='2' * 40, human_program_source_commit='3' * 40,
            web_source_commit='4' * 40, r8_manifest_sha256=a.R8_MANIFEST_SHA, assembly=assembly.ASSEMBLY,
            before_units={'studiolo-human': a.HUMAN_SHA, 'studiolo-agent': a.AGENT_SHA}, files=files)
        raw = json.dumps(doc).encode()
        self.write(new / 'manifest.json', raw)
        return a, c.sha(raw)

    def check(self, a, digest):
        verify = c.verify
        with patch.object(c, 'verify', side_effect=lambda root, sha: verify(root, sha, self.uid)):
            return ops_module.verify_candidate(a, c, digest)

    def test_exact_package_swaps_web_and_reuses_everything_else(self):
        a, digest = self.candidate()
        doc = self.check(a, digest)
        self.assertEqual(doc['build'], assembly.BUILD)
        self.assertEqual(set(doc['before_units']), set(ops_module.NAMES))
        self.assertEqual(doc['bootstrap_source_commit'], '2' * 40)
        self.assertEqual(doc['human_program_source_commit'], '3' * 40)
        self.assertEqual(doc['web_source_commit'], '4' * 40)
        # 网页整块换掉：旧资源不再出现，新资源必须在。
        self.assertEqual({name for name in doc['files'] if name.startswith('web/')},
                         {'web/index.html', 'web/assets/fresh.js'})
        self.assertEqual((Path(a.RELEASE) / 'studiolo-agent.service').read_bytes(), b'current agent')
        for name in ('bin/studiolo-local-server', 'bin/studiolo-auth-admin',
                     'bin/studiolo-vault-admin', 'example-vault/note.md'):
            self.assertEqual((Path(a.OLD_RELEASE) / name).read_bytes(),
                             (Path(a.RELEASE) / name).read_bytes())
        self.assertNotEqual((Path(a.OLD_RELEASE) / 'web/index.html').read_bytes(),
                            (Path(a.RELEASE) / 'web/index.html').read_bytes())

    def test_tamper_extra_file_directory_missing_file_and_link_rejected(self):
        a, digest = self.candidate()
        root = Path(a.RELEASE)
        for name, kind in [('extra', 'file'), ('extra', 'directory'), ('linked', 'link')]:
            with self.subTest(kind=kind):
                path = root / name
                if kind == 'file':
                    self.write(path, b'unapproved')
                elif kind == 'directory':
                    path.mkdir(mode=0o700)
                else:
                    path.symlink_to(root / 'web/index.html')
                with self.assertRaises(ValueError):
                    self.check(a, digest)
                path.rmdir() if kind == 'directory' else path.unlink()
        target = root / 'web/index.html'
        target.unlink()
        with self.assertRaises(ValueError):
            self.check(a, digest)
        self.write(target, b'tampered')
        with self.assertRaises(ValueError):
            self.check(a, digest)

    def test_manifest_hash_and_closed_schema_rejected(self):
        a, digest = self.candidate()
        with self.assertRaises(ValueError):
            self.check(a, '0' * 64)
        path = Path(a.RELEASE) / 'manifest.json'
        doc = json.loads(path.read_bytes())
        doc['unrecognized'] = True
        raw = json.dumps(doc).encode()
        path.write_bytes(raw)
        with self.assertRaises(ValueError):
            self.check(a, c.sha(raw))

    def test_new_manifest_cannot_replace_binaries_or_example_vault(self):
        a, digest = self.candidate()
        root = Path(a.RELEASE)
        path = root / 'manifest.json'
        original = path.read_bytes()
        for name in ('bin/studiolo-auth-admin', 'bin/studiolo-local-server', 'example-vault/note.md'):
            with self.subTest(name=name):
                prior = (root / name).read_bytes()
                raw = b'changed file'
                (root / name).write_bytes(raw)
                doc = json.loads(original)
                doc['files'][name] = {'bytes': len(raw), 'sha256': c.sha(raw)}
                raw = json.dumps(doc).encode()
                path.write_bytes(raw)
                with self.assertRaises(ValueError):
                    self.check(a, c.sha(raw))
                (root / name).write_bytes(prior)
                path.write_bytes(original)
                self.check(a, digest)
        # 网页可以整块换，但首页必须在，来源提交也必须是规范的 40 位十六进制。
        (root / 'web/index.html').unlink()
        doc = json.loads(original)
        del doc['files']['web/index.html']
        raw = json.dumps(doc).encode()
        path.write_bytes(raw)
        with self.assertRaises(ValueError):
            self.check(a, c.sha(raw))
        self.write(root / 'web/index.html', b'fresh web')
        path.write_bytes(original)
        for bad in ('4' * 39, 'z' * 40, None):
            doc = json.loads(original)
            doc['web_source_commit'] = bad
            raw = json.dumps(doc).encode()
            path.write_bytes(raw)
            with self.subTest(bad=bad), self.assertRaises(ValueError):
                self.check(a, c.sha(raw))
        path.write_bytes(original)

    def test_new_human_nonelf_invalid_source_or_digest_rejected(self):
        a, digest = self.candidate()
        root = Path(a.RELEASE)
        path = root / 'manifest.json'
        original = path.read_bytes()
        human = root / 'bin/studiolo-local-server'
        human.write_bytes(b'not ELF')
        doc = json.loads(original)
        doc['files']['bin/studiolo-local-server'] = {'bytes': 7, 'sha256': c.sha(b'not ELF')}
        raw = json.dumps(doc).encode()
        path.write_bytes(raw)
        with self.assertRaises(ValueError):
            self.check(a, c.sha(raw))
        path.write_bytes(original)
        human.write_bytes(b'\x7fELFtampered human')
        with self.assertRaises(ValueError):
            self.check(a, digest)
        human.write_bytes(b'\x7fELFnew human')
        for source in ('3' * 39, 'z' * 40, None):
            doc = json.loads(original)
            doc['human_program_source_commit'] = source
            raw = json.dumps(doc).encode()
            path.write_bytes(raw)
            with self.assertRaises(ValueError):
                self.check(a, c.sha(raw))

    def test_agent_read_accepts_only_approved_modes(self):
        binary = self.root / 'studiolo-agent'
        self.write(binary, b'fictional immutable agent', 0o550)
        ops = Operations('0' * 64, assembly, c, Mock(), legacy)
        ops.agent_binary = binary
        # r2 时代 root 冻结的 0550，与 T-101 起 UID1000 私有的 0700。
        for mode in (0o550, 0o700):
            with self.subTest(mode=oct(mode)):
                binary.chmod(mode)
                self.assertEqual(ops.read_agent(), b'fictional immutable agent')
        for mode in (0o755, 0o750, 0o555, 0o500, 0o440, 0o570, 0o770, 0o707, 0o600):
            with self.subTest(mode=oct(mode)):
                binary.chmod(mode)
                with self.assertRaises(ValueError):
                    ops.read_agent()

    def preparation(self):
        a, _ = self.candidate()
        a.RELEASE = str(self.root / 'prepared')
        a.launcher = lambda text: text.replace('old', 'new')
        a.human_unit = lambda text: text.replace('old', 'new')
        a.agent_unit = lambda text: text
        for name, raw in (('studiolo-human', b'old human'), ('studiolo-agent', b'old agent')):
            self.write(self.root / 'units' / (name + '.service'), raw, 0o644)
        web = self.root / 'web-dist'
        self.write(web / 'index.html', b'brand new web')
        self.write(web / 'assets' / 'brand-new.js', b'brand new bundle')
        args = types.SimpleNamespace(web=str(web), web_source_commit='5' * 40)
        return a, args

    def run_prepare(self, a, args):
        read, verify = c.read, c.verify
        def fixture_read(path, uid, mode=None):
            if path.parent == Path('/etc/systemd/system'):
                self.assertEqual(uid, 0)
                return read(self.root / 'units' / path.name, self.uid, mode)
            return read(path, uid, mode)
        with patch.object(c, 'read', side_effect=fixture_read), \
                patch.object(c, 'verify', side_effect=lambda root, sha: verify(root, sha, self.uid)):
            return prepare.prepare(args, a, c, ops_module)

    def test_prepare_swaps_web_and_carries_the_previous_program_sources(self):
        a, args = self.preparation()
        result = self.run_prepare(a, args)
        doc = self.check(a, result['manifest_sha256'])
        self.assertTrue(result['human_binary_reused'])
        self.assertEqual(result['web_files'], 2)
        # 程序来源照抄上一发行，不冒称新构建；网页来源来自本次参数。
        self.assertEqual(doc['bootstrap_source_commit'], '2' * 40)
        self.assertEqual(doc['human_program_source_commit'], '3' * 40)
        self.assertEqual(doc['web_source_commit'], args.web_source_commit)
        root = Path(a.RELEASE)
        self.assertEqual((root / 'bin/studiolo-local-server').read_bytes(), b'\x7fELFfictional human')
        self.assertEqual((root / 'bin/studiolo-auth-admin').read_bytes(), b'fictional admin')
        self.assertEqual((root / 'web/index.html').read_bytes(), b'brand new web')
        self.assertEqual((root / 'web/assets/brand-new.js').read_bytes(), b'brand new bundle')
        self.assertFalse((root / 'web/assets/stale.js').exists())
        with self.assertRaises(ValueError):
            self.run_prepare(a, args)

    def test_prepare_rejects_bad_web_source_or_missing_index_before_creating_output(self):
        a, args = self.preparation()
        for bad in ('a0acdde', 'z' * 40, '5' * 39):
            prior = args.web_source_commit
            args.web_source_commit = bad
            with self.subTest(bad=bad), self.assertRaises(ValueError):
                self.run_prepare(a, args)
            self.assertFalse(Path(a.RELEASE).exists())
            args.web_source_commit = prior
        (Path(args.web) / 'index.html').unlink()
        with self.assertRaises(ValueError):
            self.run_prepare(a, args)
        self.assertFalse(Path(a.RELEASE).exists())
        self.write(Path(args.web) / 'index.html', b'brand new web')
        # 现场单元与预期摘要不符时，同样在建目录之前就停。
        self.write(self.root / 'units' / 'studiolo-human.service.tmp', b'x')
        (self.root / 'units' / 'studiolo-human.service').write_bytes(b'drifted human')
        with self.assertRaises(ValueError):
            self.run_prepare(a, args)
        self.assertFalse(Path(a.RELEASE).exists())

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

    def broker(self):
        a = types.SimpleNamespace(VAULT=str(self.root / 'vault'), WRITER=str(self.root / 'writer'),
                                  BOARD=str(self.root / 'board'))
        vault, writer, board = [Path(p) for p in (a.VAULT, a.WRITER, a.BOARD)]
        paths = {'canonical': vault, 'data': writer, 'control': vault / '.workbench',
            'marker': vault / '.workbench/vault.json', 'staging': vault / '.workbench/staging',
            **{name: writer / name for name in ('history', 'idempotency', 'snapshots')}}
        for key, path in paths.items():
            if key != 'marker':
                path.mkdir(mode=0o700, parents=True, exist_ok=True)
        board.mkdir(mode=0o700)
        # 起点是已在跑的黑板：目录里有控制库（r2 起的现场样子）。
        self.write(board / 'control.sqlite3', b'fictional board')
        identifier = 'f0a2d4b6-5588-4411-9922-66778899aabb'
        self.write(paths['marker'], json.dumps({'format_version': 1, 'vault_uuid': identifier}).encode())
        doc = dict(format_version=1, pair_id=identifier, vault_uuid=identifier)
        for key, path in paths.items():
            s = path.stat()
            doc[key] = {'dev': s.st_dev, 'ino': s.st_ino}
        for path in (vault / '.workbench/broker-binding.json', writer / 'broker-binding.json'):
            self.write(path, json.dumps(doc).encode())
        self.write(writer / 'writer-state.json', b'fictional journal')
        return a

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


class FakeOps:
    build = assembly.BUILD

    def __init__(self, fail=None):
        self.fail, self.record, self.events = fail, None, []

    def event(self, name):
        self.events.append(name)
        if name == self.fail:
            raise ValueError('secret exception must not be emitted')

    def begin(self):
        self.event('record')
        self.record = Path('/fictional/record')

    def preflight(self):
        self.event('preflight')

    def stop(self):
        self.event('stop-agent')
        self.event('stop-human')

    def backup(self):
        self.event('backup')

    def install(self):
        self.event('install')

    def start_human(self):
        self.event('start-human')

    def start_agent(self):
        self.event('start-agent')

    def accept(self):
        self.event('accept')
        return {'board_authorization_accepted': False}

    def receipt(self, name, value):
        self.event('receipt-' + name)

    def recover(self):
        self.event('recover')
        return 'old_programs_restored'

    def close(self):
        self.event('close')


class DeploymentTests(unittest.TestCase):
    def test_success_order_and_limited_acceptance(self):
        ops = FakeOps()
        code, result = activation.deploy(ops)
        self.assertEqual(code, 0)
        self.assertEqual(ops.events, ['record', 'preflight', 'stop-agent', 'stop-human', 'backup',
            'install', 'start-human', 'start-agent', 'accept', 'receipt-accepted', 'close'])
        self.assertFalse(result['database_rollback'])
        self.assertFalse(result['board_authorization_accepted'])

    def test_preflight_failure_never_stops_services(self):
        ops = FakeOps('preflight')
        with patch.object(signal, 'signal'):
            code, result = activation.deploy(ops)
        self.assertEqual(code, 1)
        self.assertEqual(result['program_recovery'], 'not_needed')
        self.assertNotIn('stop-agent', ops.events)
        self.assertNotIn('recover', ops.events)
        self.assertNotIn('secret', json.dumps(result))

    def test_backup_human_agent_install_and_stop_failure_all_recover(self):
        for fail in ('stop-agent', 'stop-human', 'backup', 'install', 'start-human', 'start-agent', 'accept'):
            with self.subTest(fail=fail), patch.object(signal, 'signal') as signals:
                ops = FakeOps(fail)
                code, result = activation.deploy(ops)
                self.assertEqual(code, 1)
                self.assertEqual(result['program_recovery'], 'old_programs_restored')
                self.assertEqual(ops.events[-3:], ['recover', 'receipt-failed', 'close'])
                self.assertEqual(signals.call_count, 3)

    def test_failed_recovery_reports_operator_boundary(self):
        ops = FakeOps('start-agent')
        ops.recover = Mock(side_effect=ValueError('do not expose'))
        with patch.object(signal, 'signal'):
            code, result = activation.deploy(ops)
        self.assertEqual(result['program_recovery'], 'blocked_requires_operator')
        self.assertFalse(result['database_rollback'])

    def test_interrupt_after_stop_enters_bounded_recovery(self):
        ops = FakeOps()
        ops.backup = Mock(side_effect=InterruptedError('interrupted'))
        with patch.object(signal, 'signal') as signals:
            code, result = activation.deploy(ops)
        self.assertEqual(code, 1)
        self.assertEqual(result['stage'], 'backup')
        self.assertEqual(result['program_recovery'], 'old_programs_restored')
        self.assertEqual([call.args for call in signals.call_args_list], [
            (signal.SIGINT, signal.SIG_IGN), (signal.SIGTERM, signal.SIG_IGN), (signal.SIGHUP, signal.SIG_IGN)])

    def test_helpers_all_validated_before_any_is_executed(self):
        looked = []
        def source(path):
            looked.append(path.name)
            if path.name == 'snapshot_worker.py':
                raise ValueError('unsafe worker')
            return b'raise AssertionError("helper executed before full validation")'
        with patch.object(activation, 'trusted_source', side_effect=source):
            with self.assertRaises(ValueError):
                activation.trusted_modules()
        self.assertEqual(looked[-1], 'snapshot_worker.py')
        self.assertEqual(len(looked), 6)

    def test_cli_is_closed_and_does_not_echo_values(self):
        for args in ([], ['--manifest-sha256', 'secret'], ['--manifest', '0' * 64],
                     ['--manifest-sha256', '0' * 64, '--unsafe', 'secret']):
            with self.subTest(args=args), self.assertRaises(ValueError) as caught:
                activation.options(args, c)
            self.assertNotIn('secret', str(caught.exception))
        self.assertEqual(activation.options(['--manifest-sha256', '0' * 64], c).manifest_sha256, '0' * 64)


class OperationTests(unittest.TestCase):
    def operation(self):
        result = Operations('0' * 64, assembly, c, Mock(), legacy)
        result.old_units = {'studiolo-human': b'old human', 'studiolo-agent': b'old agent'}
        result.new_units = {'studiolo-human': b'new human', 'studiolo-agent': b'new agent'}
        result.old_shas = {n: c.sha(v) for n, v in result.old_units.items()}
        result.new_shas = {n: c.sha(v) for n, v in result.new_units.items()}
        result.human_sha = '2' * 64
        result.old_human_sha = '3' * 64
        return result

    def test_agent_probe_is_fixed_anonymous_head_without_proxy_redirect_or_body(self):
        ops = self.operation()
        ops.runtime.command.return_value.stdout = '401'
        with patch.object(ops_module.time, 'monotonic', return_value=10):
            self.assertTrue(ops.agent_http_ready(20))
        self.assertEqual(ops.runtime.command.call_args.args, ([
            '/usr/bin/curl', '--disable', '--head', '--silent',
            '--output', '/dev/null', '--write-out', '%{http_code}',
            '--noproxy', '*', '--proxy', '', '--proto', '=http',
            '--max-redirs', '0', '--max-time', '1.0',
            '--url', 'http://127.0.0.1:4400/mcp'], 1.0))

    def test_agent_probe_refuses_non401_and_unready_connections(self):
        ops = self.operation()
        with patch.object(ops_module.time, 'monotonic', return_value=10):
            for status in ('200', '301', '302', '403', '500', '401\n', '000'):
                with self.subTest(status=status):
                    ops.runtime.command.return_value.stdout = status
                    self.assertFalse(ops.agent_http_ready(20))
            for error in (ConnectionRefusedError(), ops_module.subprocess.TimeoutExpired('curl', 1),
                          ops_module.subprocess.CalledProcessError(7, 'curl')):
                with self.subTest(error=type(error).__name__):
                    ops.runtime.command.side_effect = error
                    self.assertFalse(ops.agent_http_ready(20))

    def test_agent_probe_never_runs_after_deadline_and_bounds_last_attempt(self):
        ops = self.operation()
        with patch.object(ops_module.time, 'monotonic', return_value=20):
            self.assertFalse(ops.agent_http_ready(20))
        ops.runtime.command.assert_not_called()
        ops.runtime.command.return_value.stdout = '401'
        with patch.object(ops_module.time, 'monotonic', side_effect=[19.75, 20.01]):
            self.assertFalse(ops.agent_http_ready(20))
        self.assertEqual(ops.runtime.command.call_args.args[1], 0.25)
        args = ops.runtime.command.call_args.args[0]
        self.assertEqual(args[args.index('--max-time') + 1], '0.25')

    def agent_poll(self, ops, statuses, *, never_ready=False):
        ticks = [0.0]
        ops.agent_binary = Path('/fictional/bin/studiolo-agent')
        ops.agent_sha = c.sha(b'agent')
        ops.runtime.wait_program.side_effect = lambda binary, **kwargs: runtime.wait_program(
            binary, readlink=lambda _: str(binary), **kwargs)
        attempts = iter(statuses)
        def probe(*_args):
            status = '503' if never_ready else next(attempts)
            if isinstance(status, BaseException):
                raise status
            return types.SimpleNamespace(stdout=status)
        ops.runtime.command.side_effect = probe
        state = dict(MainPID='123', ActiveState='active', SubState='running', NRestarts='0')
        with patch.object(ops_module.time, 'monotonic', side_effect=lambda: ticks[0]), \
                patch.object(ops_module.time, 'sleep', side_effect=lambda n: ticks.__setitem__(0, ticks[0] + n)), \
                patch.object(ops, 'properties', return_value=state), patch.object(ops, 'read_agent', return_value=b'agent'), \
                patch.object(os, 'stat', return_value=types.SimpleNamespace(st_dev=1, st_ino=2)):
            result = ops.agent_ready()
        return result, ticks[0]

    def test_agent_process_waits_for_actual_401_after_connection_and_status_failures(self):
        ops = self.operation()
        result, elapsed = self.agent_poll(ops, [ConnectionRefusedError(), '503', '401'])
        self.assertEqual(result, 123)
        self.assertEqual(ops.runtime.command.call_count, 3)
        self.assertGreaterEqual(elapsed, 0.2)

    def test_agent_process_without_http_ready_hits_deadline(self):
        ops = self.operation()
        with self.assertRaises(ValueError):
            self.agent_poll(ops, [], never_ready=True)
        self.assertGreater(ops.runtime.command.call_count, 1)
        self.assertLessEqual(ops.runtime.command.call_count, 201)

    def test_actual_stop_order_and_no_socket_removal(self):
        ops = self.operation()
        with patch.object(ops, 'known_units'), patch.object(ops, 'stopped'):
            ops.stop()
        self.assertEqual([call.args[0] for call in ops.runtime.command.call_args_list], [
            ['systemctl', 'stop', 'studiolo-agent.service'], ['systemctl', 'stop', 'studiolo-human.service']])

    def test_worker_uses_original_code_as_unprivileged_isolated_stdin(self):
        ops = self.operation()
        summary = {'epoch': 1, 'factor_ref': 'studiolo-human-totp-v2', 'code_count': 10, 'restore_event_count': 0}
        with patch.object(ops_module.pwd, 'getpwnam', return_value=types.SimpleNamespace(pw_uid=1000, pw_gid=1000)), \
                patch.object(ops, 'trusted_read', return_value=b'# fictional worker') as read, \
                patch.object(ops_module.subprocess, 'run', return_value=types.SimpleNamespace(
                    stdout=json.dumps(summary).encode())) as run:
            self.assertEqual(ops.worker('binding'), summary)
        read.assert_called_once_with(legacy.WORKER)
        self.assertEqual(run.call_args.args[0], ['/usr/bin/python3', '-I', '-B', '-S', '-', 'binding'])
        self.assertEqual(run.call_args.kwargs['user'], 1000)
        self.assertEqual(run.call_args.kwargs['extra_groups'], [])
        self.assertEqual(run.call_args.kwargs['timeout'], 40)
        self.assertEqual(run.call_args.kwargs['pass_fds'], ())

    def test_human_must_be_ready_with_board_socket_before_agent_can_start(self):
        ops = self.operation()
        events = []
        ops.runtime.command.side_effect = lambda args, *rest: events.append(args[-1])
        ops.runtime.ready.side_effect = lambda *args: events.append('human-ready')
        with patch.object(ops, 'board_ready', side_effect=lambda: events.append('board-ready')), \
                patch.object(ops, 'agent_ready', side_effect=lambda: events.append('agent-ready')):
            ops.start_human()
            ops.start_agent()
        self.assertEqual(events, ['studiolo-human.service', 'human-ready', 'board-ready',
                                  'studiolo-agent.service', 'agent-ready'])

    def test_actual_recover_partial_install_restores_only_changed_unit(self):
        ops = self.operation()
        current = {'studiolo-human': ops.new_shas['studiolo-human'], 'studiolo-agent': ops.old_shas['studiolo-agent']}
        events = []
        ops.runtime.command.side_effect = lambda args, *rest: events.append(args)
        with patch.object(ops, 'known_units', return_value=current), \
                patch.object(ops, 'stop', side_effect=lambda: events.append('stop')) , \
                patch.object(ops, 'unchanged', side_effect=lambda: events.append('binding')), \
                patch.object(c, 'atomic_unit', side_effect=lambda *args: events.append('restore-human')), \
                patch.object(ops, 'agent_ready'), patch.object(ops, 'units', return_value=ops.old_units):
            self.assertEqual(ops.recover(), 'old_programs_restored')
        self.assertEqual(events, ['stop', 'binding', 'restore-human', ['systemctl', 'daemon-reload'],
            ['systemctl', 'start', 'studiolo-human.service'], ['systemctl', 'start', 'studiolo-agent.service'], 'binding'])

    def test_unknown_unit_refuses_recovery_without_service_actions(self):
        ops = self.operation()
        with patch.object(ops, 'units', return_value={**ops.new_units, 'studiolo-agent': b'unknown'}), \
                patch.object(c, 'atomic_unit') as replace:
            with self.assertRaises(ValueError):
                ops.recover()
        replace.assert_not_called()
        ops.runtime.command.assert_not_called()

    def test_changed_auth_binding_stops_but_does_not_restore_or_restart(self):
        ops = self.operation()
        with patch.object(ops, 'known_units', return_value=ops.new_shas), patch.object(ops, 'stop') as stop, \
                patch.object(ops, 'unchanged', side_effect=ValueError('changed')), \
                patch.object(c, 'atomic_unit') as replace:
            with self.assertRaises(ValueError):
                ops.recover()
        stop.assert_called_once()
        replace.assert_not_called()
        ops.runtime.command.assert_not_called()

    def test_preflight_rejects_wrong_live_unit_before_service_command(self):
        ops = self.operation()
        with patch.object(ops_module.sys, 'platform', 'linux'), patch.object(os, 'getuid', return_value=0), \
                patch.object(os, 'geteuid', return_value=0), patch.object(ops_module.pwd, 'getpwnam',
                return_value=types.SimpleNamespace(pw_uid=1000)), \
                patch.object(ops_module, 'verify_candidate', return_value={}), \
                patch.object(ops, 'units', return_value={'studiolo-human': b'wrong', 'studiolo-agent': b'wrong'}):
            with self.assertRaises(ValueError):
                ops.preflight()
        ops.runtime.command.assert_not_called()

    def test_stopped_refuses_lingering_socket_without_removing_it(self):
        ops = self.operation()
        with patch.object(ops, 'properties', return_value={'ActiveState': 'inactive', 'MainPID': '0'}), \
                patch.object(os.path, 'lexists', return_value=True), patch.object(os, 'unlink') as unlink:
            with self.assertRaises(ValueError):
                ops.stopped()
        unlink.assert_not_called()


if __name__ == '__main__':
    unittest.main()
