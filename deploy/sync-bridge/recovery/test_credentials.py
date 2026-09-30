"""用户配置入口的虚拟秘密夹具；本测试不能接入任何生产端点。"""
import contextlib
import importlib.util
import io
import os
from pathlib import Path
import stat
import tempfile
import unittest
from unittest.mock import patch


def load(name):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(name + '.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


nas, client = load('nas_credentials'), load('client_credentials')


class CredentialTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.old_umask = os.umask(0o077)
        self.addCleanup(os.umask, self.old_umask)

    def nas_install(self, raw):
        stdin = io.TextIOWrapper(io.BytesIO(raw))
        self.addCleanup(stdin.close)
        with patch.object(nas, 'ROOT', self.root), patch.object(nas.os, 'geteuid', return_value=0), \
             patch.object(nas.os, 'fchown'), patch.object(nas.sys, 'argv', ['fixture', 'install-auth']), \
             patch.object(nas.sys, 'stdin', stdin), contextlib.redirect_stdout(io.StringIO()) as out:
            nas.main()
        return out.getvalue()

    def test_bcrypt_closed_input_mode_and_no_overwrite(self):
        raw = b'studiolo-backup:$2y$12$' + b'A' * 53 + b'\n'
        self.assertEqual(self.nas_install(raw), 'nas_backup_setup=ready\n')
        target = self.root / 'backup.htpasswd'
        self.assertEqual(stat.S_IMODE(target.stat().st_mode), 0o640)
        with self.assertRaises(FileExistsError):
            self.nas_install(raw)
        self.assertEqual(target.read_bytes(), raw)

    def test_weak_or_duplicate_credentials_rejected(self):
        for raw in (b'x:{SHA}abc\n', b'studiolo-backup:$2y$04$' + b'A' * 53 + b'\n',
                    b'studiolo-backup:$2y$12$' + b'A' * 53 + b'\nsecond:bad\n'):
            with self.subTest(raw=raw[:3]), self.assertRaises(ValueError):
                self.nas_install(raw)
        self.assertFalse((self.root / 'backup.htpasswd').exists())

    def client_setup(self, answers, run):
        (self.root / 'ca.pem').touch(exist_ok=True)
        with patch.object(client, 'ROOT', self.root), patch.object(client.os, 'getuid', return_value=1000), \
             patch.object(client.sys, 'argv', ['fixture']), patch.object(client.sys.stdin, 'isatty', return_value=True), \
             patch.object(client.sys.stdout, 'isatty', return_value=True), patch.object(client.sys.stderr, 'isatty', return_value=True), \
             patch.object(client.getpass, 'getpass', side_effect=answers), \
             patch.object(client.subprocess, 'run', side_effect=run), patch('builtins.print') as printed:
            client.main()
        return printed.call_args_list

    def test_client_keeps_fixture_passwords_out_of_argv_and_output(self):
        calls = []
        def run(argv, **kwargs):
            calls.append((argv, kwargs))
            return type('Completed', (), {'returncode': 0})()
        printed = self.client_setup(['http-fixture', 'repository-fixture-secret', 'repository-fixture-secret'], run)
        self.assertEqual(calls[0][0], [client.RESTIC, 'init', '--json'])
        self.assertEqual(calls[0][1]['env']['RESTIC_REST_PASSWORD'], 'http-fixture')
        self.assertEqual(str(printed), "[call('backup_repository_initialized=true')]")
        for name in ('credentials.json', 'repository-password'):
            self.assertEqual(stat.S_IMODE((self.root / name).stat().st_mode), 0o600)

    def test_unconfirmed_or_same_password_leaves_no_config(self):
        for answers in (['http-fixture', 'repository-fixture-secret', 'different'],
                        ['same-fixture-password', 'same-fixture-password', 'same-fixture-password']):
            with self.subTest(), self.assertRaises(ValueError):
                self.client_setup(answers, lambda *a, **kw: self.fail('must not run'))
        self.assertFalse((self.root / 'credentials.json').exists())


if __name__ == '__main__':
    unittest.main()
