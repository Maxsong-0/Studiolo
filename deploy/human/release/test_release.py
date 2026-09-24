#!/usr/bin/python3
"""虚构数据与注入式服务控制测试；不调用systemd、不访问网络或生产路径。"""
import json
import os
from pathlib import Path
import sys
import tempfile
import types
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
import activate
from activation_ops import Operations
import package
import release_common as c
import runtime_checks as runtime
from test_snapshot_worker import SnapshotTests


class PackageTests(unittest.TestCase):
    def setUp(self):
        self.mask = os.umask(0o077)
        self.temp = tempfile.TemporaryDirectory(prefix='.studiolo-r8-fictional-', dir=Path.home())
        self.root = Path(self.temp.name)
        self.args = types.SimpleNamespace(repo=str(self.root / 'repo'),
            example_vault=str(self.root / 'example'), local_server=str(self.root / 'local-server'),
            auth_admin=str(self.root / 'auth-admin'), sync_launcher=str(self.root / 'sync-launcher'),
            source_commit='1' * 40, output=str(self.root / 'release'))
        for name, data in {
            'repo/web/dist/index.html': b'fictional web', 'repo/web/dist/assets/main.js': b'void 0',
            'example/note.md': b'fictional note', 'local-server': b'fictional ELF',
            'auth-admin': b'fictional CLI', 'sync-launcher': b'fictional launcher',
            'repo/deploy/human/current-v2/launch-human.py': c.OLD_RELEASE.encode(),
            'repo/deploy/human/current-v2/studiolo-human.service': c.OLD_RELEASE.encode(),
        }.items():
            path = self.root / name
            path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
            c.new_file(path, data)

    def tearDown(self):
        self.temp.cleanup()
        os.umask(self.mask)

    def build(self):
        result = package.package(self.args)
        root = Path(self.args.output)
        return root, result['manifest_sha256']

    def test_exact_manifest_and_fixed_template_replacement(self):
        root, digest = self.build()
        doc = c.verify(root, digest, os.geteuid())
        self.assertEqual(set(doc), {'build', 'source_commit', 'assembly', 'files'})
        self.assertEqual((root / 'launch-human.py').read_bytes(), str(c.RELEASE).encode())
        self.assertEqual((root / 'bin/studiolo-auth-admin').stat().st_mode & 0o777, 0o755)
        self.assertEqual((root / 'bin/studiolo-sync-launcher').stat().st_mode & 0o777, 0o755)
        self.assertEqual((root / 'bin/studiolo-sync-launcher').read_bytes(), b'fictional launcher')
        self.assertIn('bin/studiolo-sync-launcher', doc['files'])
        self.assertEqual((root / 'web/index.html').stat().st_mode & 0o777, 0o600)

    def test_existing_output_never_overwritten(self):
        root, digest = self.build()
        with self.assertRaises(ValueError):
            package.package(self.args)
        c.verify(root, digest, os.geteuid())

    def test_extra_node_missing_file_and_tamper_rejected(self):
        root, digest = self.build()
        extra = root / 'empty'
        extra.mkdir(mode=0o700)
        with self.assertRaises(ValueError):
            c.verify(root, digest, os.geteuid())
        extra.rmdir()
        file = root / 'example-vault/note.md'
        original = file.read_bytes()
        file.unlink()
        with self.assertRaises(ValueError):
            c.verify(root, digest, os.geteuid())
        c.new_file(file, original + b'changed')
        with self.assertRaises(ValueError):
            c.verify(root, digest, os.geteuid())

    def test_links_and_unsafe_mode_rejected(self):
        path = self.root / 'example/note.md'
        linked = self.root / 'example/link'
        linked.symlink_to(path)
        with self.assertRaises(ValueError):
            package.package(self.args)
        linked.unlink()
        os.link(path, linked)
        with self.assertRaises(ValueError):
            package.package(self.args)
        linked.unlink()
        path.chmod(0o622)
        with self.assertRaises(ValueError):
            package.package(self.args)

    def test_writable_ancestor_rejected_without_source_change(self):
        parent = self.root / 'example'
        source = parent / 'note.md'
        content = source.read_bytes()
        parent.chmod(0o775)
        with self.assertRaises(ValueError):
            package.package(self.args)
        parent.chmod(0o755)
        self.build()
        self.assertEqual(source.read_bytes(), content)

    def test_input_change_keeps_incomplete_output(self):
        source = self.root / 'repo/web/dist/index.html'
        original, count = c.read, 0
        def changing(path, *args):
            nonlocal count
            data = original(path, *args)
            if path == source:
                count += 1
                if count == 2:
                    source.write_bytes(b'changed fictional input')
            return data
        with patch.object(c, 'read', side_effect=changing), self.assertRaises(ValueError):
            package.package(self.args)
        self.assertTrue((Path(self.args.output) / 'INCOMPLETE').is_file())

    def test_bad_manifest_permissions_and_duplicates(self):
        root, digest = self.build()
        (root / 'web/index.html').chmod(0o644)
        with self.assertRaises(ValueError):
            c.verify(root, digest, os.geteuid())
        with self.assertRaises(ValueError):
            c.document(b'{"files":{},"files":{}}')
        for value in ('../file', '/file', 'a//b', 'a/./b'):
            with self.assertRaises(ValueError):
                c.relative(value)


class FakeOperations:
    def __init__(self, failure=None):
        self.failure, self.events, self.record = failure, [], None
    def step(self, name):
        self.events.append(name)
        if name == self.failure:
            raise ValueError('fictional-sensitive-value')
    def begin(self):
        self.record = Path('/fictional-record')
        self.step('begin')
    def preflight(self): self.step('preflight')
    def stop(self): self.step('stop')
    def backup(self): self.step('backup')
    def install(self): self.step('install')
    def start(self): self.step('start')
    def accept(self):
        self.step('accept')
        return {'pid': 123}
    def recover(self, changed):
        self.events.append(('recover', changed))
        return 'old_program_restored'
    def receipt(self, name, value): self.events.append(('receipt', name))
    def close(self): self.events.append('close')


class ControlTests(unittest.TestCase):
    def test_closed_arguments_do_not_echo_sensitive_values(self):
        self.assertEqual(activate.options(['--manifest-sha256', 'a' * 64]).manifest_sha256, 'a' * 64)
        for argv in ([], ['--manifest-sha256', 'A' * 64], ['--secret', 'fictional-sensitive-value'],
                     ['--manifest-sha256', 'a' * 64, '--manifest-sha256', 'b' * 64]):
            with self.subTest(argv=argv), self.assertRaises(ValueError) as error:
                activate.options(argv)
            self.assertNotIn('fictional-sensitive-value', str(error.exception))
        with self.assertRaises(ValueError):
            package.options(['--repo', '/fictional'])
        # 少了 --sync-launcher 就是旧的 12 参形状，闭集参数表拒绝。
        twelve = ['--repo', '/f', '--example-vault', '/f', '--local-server', '/f', '--auth-admin', '/f',
                  '--source-commit', '1' * 40, '--output', '/f']
        with self.assertRaises(ValueError):
            package.options(twelve)
        self.assertEqual(package.options(twelve + ['--sync-launcher', '/f/launcher']).sync_launcher, '/f/launcher')

    def test_failure_boundaries_and_success(self):
        for failure, recover in [('preflight', None), ('backup', False), ('start', True),
                                 ('accept', True), (None, None)]:
            with self.subTest(failure=failure), patch.object(activate.signal, 'signal'):
                ops = FakeOperations(failure)
                code, receipt = activate.deploy(ops)
                self.assertEqual(code, int(failure is not None))
                self.assertNotIn('fictional-sensitive-value', json.dumps(receipt))
                if failure == 'preflight':
                    self.assertNotIn('stop', ops.events)
                if recover is not None:
                    self.assertIn(('recover', recover), ops.events)
                    self.assertEqual(receipt['program_recovery'], 'old_program_restored')
                self.assertIn(('receipt', 'failed' if failure else 'accepted'), ops.events)
                self.assertEqual(ops.events[-1], 'close')

    def test_actual_old_unit_mismatch_stops_before_service_actions(self):
        ops = Operations('0' * 64)
        with patch.object(c, 'verify', return_value={}), patch.object(c, 'read', return_value=b'wrong unit'), \
                patch.object(runtime, 'command') as command:
            with self.assertRaises(ValueError):
                ops.preflight()
            command.assert_not_called()

    def test_actual_recovery_restores_program_without_database_restore(self):
        ops = Operations('0' * 64)
        ops.old_unit, ops.new_sha = b'old unit', c.sha(b'candidate unit')
        events = []
        with patch.object(c, 'read', return_value=b'candidate unit'), \
                patch.object(c, 'atomic_unit', side_effect=lambda *a: events.append('old unit installed')), \
                patch.object(ops, 'stop', side_effect=lambda: events.append('candidate stopped')), \
                patch.object(ops, 'start', side_effect=lambda: events.append('old started')), \
                patch.object(ops, 'unchanged', side_effect=lambda: events.append('binding checked')), \
                patch.object(runtime, 'properties', return_value={'ActiveState': 'inactive'}), \
                patch.object(runtime, 'command'), patch.object(runtime, 'stopped'), \
                patch.object(runtime, 'ready'), patch.object(runtime, 'denials'):
            self.assertEqual(ops.recover(True), 'old_program_restored')
        self.assertLess(events.index('candidate stopped'), events.index('old unit installed'))
        self.assertLess(events.index('old unit installed'), events.index('old started'))
        self.assertEqual(events.count('binding checked'), 2)

    def test_launcher_active_is_not_product_ready(self):
        times, exes = [0], iter(['/usr/bin/python3', '/fictional/bin/server'])
        properties = lambda: dict(MainPID='123', ActiveState='active', SubState='running', NRestarts='0')
        result = runtime.wait_program(Path('/fictional/bin/server'), get_properties=properties,
            readlink=lambda _: next(exes), sleep=lambda n: times.append(times[-1] + n),
            monotonic=lambda: times[-1], socket_exists=lambda: True)
        self.assertEqual(result, 123)
        self.assertEqual(len(times), 2)

    def test_wait_is_bounded_and_restart_rejected(self):
        for restarts in ('0', '1'):
            times = [0]
            with self.subTest(restarts=restarts), self.assertRaises(ValueError):
                runtime.wait_program(Path('/fictional/bin/server'), deadline_seconds=0.2,
                    get_properties=lambda: dict(MainPID='123', ActiveState='active',
                        SubState='running', NRestarts=restarts), readlink=lambda _: '/usr/bin/python3',
                    sleep=lambda n: times.append(times[-1] + n), monotonic=lambda: times[-1],
                    socket_exists=lambda: True)
            self.assertLessEqual(times[-1], 0.2)


if __name__ == '__main__':
    unittest.main()
