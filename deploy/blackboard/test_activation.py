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
        old, new = self.root / 'old', self.root / 'new'
        content = {'bin/studiolo-local-server': b'fictional human',
                   'bin/studiolo-auth-admin': b'fictional admin', 'launch-human.py': b'old launcher',
                   'studiolo-human.service': b'old human', 'web/index.html': b'fictional web',
                   'example-vault/note.md': b'fictional note'}
        for name, raw in content.items():
            self.write(old / name, raw, c.mode_for(name))
        files, _ = c.scan(old, self.uid)
        old_doc = dict(build=c.BUILD, source_commit='1' * 40, assembly='human-v2-r8', files=files)
        raw = json.dumps(old_doc).encode()
        self.write(old / 'manifest.json', raw)
        a = types.SimpleNamespace(OLD_RELEASE=str(old), RELEASE=str(new), BUILD=assembly.BUILD,
            R8_MANIFEST_SHA=c.sha(raw), HUMAN_SHA=c.sha(b'old human'), AGENT_SHA=c.sha(b'old agent'))
        content.update({'launch-human.py': b'new launcher', 'studiolo-human.service': b'new human',
                        'studiolo-agent.service': b'new agent', 'bin/studiolo-vault-admin': b'\x7fELFfictional'})
        for name, raw in content.items():
            self.write(new / name, raw, 0o755 if name in c.BINARIES | {'bin/studiolo-vault-admin'} else 0o600)
        files, _ = c.scan(new, self.uid)
        doc = dict(build=a.BUILD, bootstrap_source_commit='2' * 40, human_program_source_commit='1' * 40,
            r8_manifest_sha256=a.R8_MANIFEST_SHA, assembly='canonical-board-consumer-only',
            before_units={'studiolo-human': a.HUMAN_SHA, 'studiolo-agent': a.AGENT_SHA}, files=files)
        raw = json.dumps(doc).encode()
        self.write(new / 'manifest.json', raw)
        return a, c.sha(raw)

    def check(self, a, digest):
        verify = c.verify
        with patch.object(c, 'verify', side_effect=lambda root, sha: verify(root, sha, self.uid)):
            return ops_module.verify_candidate(a, c, digest)

    def test_exact_package_reuses_both_old_binaries(self):
        a, digest = self.candidate()
        doc = self.check(a, digest)
        self.assertEqual(doc['build'], assembly.BUILD)
        self.assertEqual(set(doc['before_units']), set(ops_module.NAMES))

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

    def test_new_manifest_cannot_replace_reused_binary(self):
        a, digest = self.candidate()
        root = Path(a.RELEASE)
        raw = b'changed binary'
        (root / 'bin/studiolo-local-server').write_bytes(raw)
        path = root / 'manifest.json'
        doc = json.loads(path.read_bytes())
        doc['files']['bin/studiolo-local-server'] = {'bytes': len(raw), 'sha256': c.sha(raw)}
        raw = json.dumps(doc).encode()
        path.write_bytes(raw)
        with self.assertRaises(ValueError):
            self.check(a, c.sha(raw))

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

    def test_initialized_binding_and_empty_board_accepted(self):
        a = self.broker()
        self.assertEqual(ops_module.binding(c, a)['format_version'], 1)

    def test_missing_writer_binding_rejected_without_creating_it(self):
        a = self.broker()
        path = Path(a.WRITER) / 'broker-binding.json'
        path.unlink()
        with self.assertRaises(FileNotFoundError):
            ops_module.binding(c, a)
        self.assertFalse(path.exists())

    def test_wrong_inode_or_nonempty_board_rejected(self):
        a = self.broker()
        self.write(Path(a.BOARD) / 'existing', b'preserve')
        with self.assertRaises(ValueError):
            ops_module.binding(c, a)
        (Path(a.BOARD) / 'existing').unlink()
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
