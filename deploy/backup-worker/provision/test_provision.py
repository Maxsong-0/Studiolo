"""备份 worker 置备脚本的契约：两份配置的键和取值规则对得上程序，口令录入不覆盖、不外泄，NAS 账号行原子追加。

全部用虚构值和临时目录：不连任何机器，不碰 /etc、/run、真实账号。
"""
import ast
import fcntl
import importlib.util
import io
import os
from pathlib import Path
import re
import shutil
import stat
import subprocess
import sys
import tempfile
import types
import unittest
from unittest.mock import patch

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]


def load(name, path=None):
    spec = importlib.util.spec_from_file_location(name, path or HERE / (name + '.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


site, credentials, nas = load('prereq'), load('credentials'), load('nas_account')
assembly = load('assembly', ROOT / 'deploy' / 'blackboard' / 'assembly.py')
COMMAND = (HERE / 'nas-account.command').read_text(encoding='utf-8')
WORKER_UID, READER_GID = 2101, 2102


def source(path):
    return (ROOT / path).read_text(encoding='utf-8')


def fields(path, struct):
    body = re.search(r'struct ' + struct + r' \{\n(.*?)\n\}', source(path), re.S).group(1)
    return set(re.findall(r'^ {4}(\w+):', body, re.M))


def documents():
    backup = site.backup_document(WORKER_UID, READER_GID)
    return backup, site.worker_document(WORKER_UID, READER_GID)


class ConfigurationTests(unittest.TestCase):
    def test_keys_are_exactly_the_fields_the_programs_parse(self):
        backup, worker = documents()
        approval, producer = backup['backup'], backup['backup_producer']
        host = 'crates/host-config/src/backup/'
        self.assertEqual(set(backup), fields(host + 'file.rs', 'Document'))
        self.assertEqual(set(approval), fields(host + 'document.rs', 'Document'))
        self.assertEqual(set(producer), fields(host + 'producer.rs', 'Document'))
        self.assertEqual(set(worker), fields('crates/backup-worker/src/config.rs', 'Document'))
        for entries, path, struct in ((approval['credentials'], 'document.rs', 'CredentialDocument'),
                                      (producer['application_packages'], 'producer.rs', 'PackageDocument'),
                                      (producer['deployment'], 'producer.rs', 'UnitDocument')):
            for entry in entries:
                self.assertEqual(set(entry), fields(host + path, struct))

    def test_generated_documents_pass_the_copied_rules_and_agree_with_each_other(self):
        backup, worker = documents()
        site.check_backup_document(backup)
        site.check_worker_document(worker, backup)
        self.assertTrue(site.encode(backup).endswith(b'}\n'))
        self.assertLess(len(site.encode(backup)), 16 * 1024)
        # 批准的数字（2026-11-18 Q2）和审批里「间隔加完成预算不超过 RPO」这一条正好顶格。
        approval = backup['backup']
        self.assertEqual((approval['schedule_interval_millis'], approval['completion_budget_millis'],
                          approval['rpo_millis'], approval['keep_last'], approval['keep_daily']),
                         (600_000, 300_000, 900_000, 30, 14))
        self.assertEqual([entry['environment_variable'] for entry in approval['credentials']],
                         re.findall(r'"(RESTIC_[A-Z_]+)",', source('crates/backup/src/worker/restic.rs'))[:3])

    def test_fixtures_the_rust_parsers_read_are_exactly_what_the_script_generates(self):
        """抄来的规则只能证明「脚本自己觉得对」。两份夹具另由程序的真实解析读一遍，这里钉住夹具就是脚本的输出。"""
        backup = site.backup_document(*site.PLACEHOLDER_NUMBERS)
        worker = site.worker_document(*site.PLACEHOLDER_NUMBERS)
        self.assertEqual((HERE / 'fixtures' / 'backup.json').read_bytes(), site.encode(backup))
        self.assertEqual((HERE / 'fixtures' / 'backup-worker.json').read_bytes(), site.encode(worker))
        self.assertEqual(sorted(os.listdir(HERE / 'fixtures')), ['backup-worker.json', 'backup.json'])
        for test, name, number in (('crates/host-config/tests/support/backup_tests.rs', 'backup.json', 'worker_uid(), 64001'),
                                   ('crates/backup-worker/tests/support/config.rs', 'backup-worker.json', 'parse(raw, 64001)')):
            reader = source(test)
            self.assertIn('include_bytes!("../../../../deploy/backup-worker/provision/fixtures/' + name + '")', reader)
            self.assertIn(number, reader)
        # 预演在账号还没建时打出来的，也是这一组占位号码。
        with patch.object(site, 'account', return_value=None):
            self.assertEqual(site.Provision(False, run=self.fail, out=self.fail).numbers(),
                             site.PLACEHOLDER_NUMBERS + (False,))

    def test_each_rule_refuses_its_own_violation(self):
        def broken(change):
            backup, _ = documents()
            change(backup['backup'], backup['backup_producer'], backup)
            return backup
        cases = {
            'rpo': lambda a, p, d: a.update(rpo_millis=899_999),
            'zero interval': lambda a, p, d: a.update(schedule_interval_millis=0),
            'shape': lambda a, p, d: a.update(target_shape='plain_smb_or_nfs_directory'),
            'restic name': lambda a, p, d: a.update(restic_package='restic2'),
            'restic version': lambda a, p, d: a.update(restic_version='0.19.1-rc'),
            'keep': lambda a, p, d: a.update(keep_daily=0),
            'variable': lambda a, p, d: a['credentials'][0].update(environment_variable='restic_password'),
            'logical': lambda a, p, d: a['credentials'][0].update(logical_name='Restic'),
            'no credentials': lambda a, p, d: a.update(credentials=[]),
            'extra approval key': lambda a, p, d: a.update(note='x'),
            'extra top key': lambda a, p, d: d.update(owner={}),
            'top version': lambda a, p, d: d.update(version=2),
            'missing producer key': lambda a, p, d: p.pop('ephemeral_root'),
            'worker is root': lambda a, p, d: p.update(worker_uid=0),
            'worker is owner': lambda a, p, d: p.update(worker_uid=site.OWNER_UID),
            'reader max': lambda a, p, d: p.update(reader_gid=0xFFFF_FFFF),
            'retention low': lambda a, p, d: p.update(source_retention_millis=599_999),
            'retention high': lambda a, p, d: p.update(source_retention_millis=86_400_001),
            'freeze low': lambda a, p, d: p.update(freeze_budget_millis=999),
            'freeze high': lambda a, p, d: p.update(freeze_budget_millis=120_001),
            'digest case': lambda a, p, d: p.update(restic_digest='AB' * 32),
            'relative': lambda a, p, d: p.update(ephemeral_root='home/x'),
            'dotdot': lambda a, p, d: p.update(staging_parent='/var/lib/../lib/x'),
            'long socket': lambda a, p, d: p.update(socket='/run/' + 'x' * 96),
            'socket in staging': lambda a, p, d: p.update(socket=site.STAGING + '/producer.sock'),
            'ephemeral in auth': lambda a, p, d: p.update(ephemeral_root=site.PRIVATE[0] + '/tmp'),
            'audit in staging': lambda a, p, d: p.update(staging_parent=str(Path(site.PRIVATE[1]).parent)),
            'no packages': lambda a, p, d: p.update(application_packages=[]),
            'many units': lambda a, p, d: p.update(deployment=p['deployment'] * 9),
            'label': lambda a, p, d: p['deployment'][0].update(unit='studiolo human.service'),
            'package digest': lambda a, p, d: p['application_packages'][0].update(digest='0' * 63),
        }
        for name, change in cases.items():
            with self.subTest(name), self.assertRaises(site.Refused):
                site.check_backup_document(broken(change))

    def test_worker_rules_refuse_their_own_violations(self):
        backup, worker = documents()
        cases = {
            'schema': {'schema': 2},
            'same account': {'worker_uid': site.OWNER_UID},
            'other reader': {'reader_gid': READER_GID + 1},
            'other parent': {'sealed_parent': '/var/lib/other'},
            'own state': {'sealed_parent': '/var/lib/studiolo-backup-worker/sealed'},
            'credentials': {'producer_socket': '/run/credentials/x.sock'},
            'other restic': {'restic_sha256': 'ab' * 32},
            'ca digest': {'repository_ca_sha256': 'xyz'},
            'userinfo': {'repository': 'rest:https://user:pass@192.168.100.111:18443/a/b/'},
            'plain http': {'repository': 'rest:http://192.168.100.111:18443/a/b/'},
            'query': {'repository': 'rest:https://192.168.100.111:18443/a/b/?x=1'},
            'default port': {'repository': 'rest:https://192.168.100.111:443/a/b/'},
            'parent segment': {'repository': 'rest:https://192.168.100.111:18443/a/../b/'},
            'no slash': {'repository': 'rest:https://192.168.100.111:18443/a/b'},
            'upper host': {'repository': 'rest:https://NAS:18443/a/b/'},
        }
        for name, change in cases.items():
            with self.subTest(name), self.assertRaises(site.Refused):
                site.check_worker_document(dict(worker, **change), backup)
        extra = dict(worker, note='x')
        with self.assertRaises(site.Refused):
            site.check_worker_document(extra, backup)

    def test_site_values_match_the_assembly_the_units_and_the_worker_source(self):
        self.assertEqual((site.BACKUP_CONFIG, site.RUNTIME, site.STAGING, site.HUMAN_RELEASE),
                         (assembly.BACKUP_CONFIG, assembly.BACKUP_RUNTIME, assembly.BACKUP_STAGING, assembly.BUILD))
        self.assertLessEqual(site.FREEZE_MILLIS, assembly.BACKUP_FREEZE_BUDGET_SECONDS * 1000)
        operations = source('deploy/blackboard/activation_ops.py')
        for private in site.PRIVATE:
            self.assertIn("'" + private + "'", operations)
        self.assertIn('"' + site.RESTIC + '"', source('crates/backup-worker/src/package.rs'))
        reader = source('crates/backup-worker/src/config/input.rs')
        self.assertIn('"' + site.CA + '"', reader)
        # 配置文件名只写在打开处，目录写在同一个文件里：两段都要对得上。
        self.assertIn('"' + Path(site.WORKER_CONFIG).name + '"', reader)
        self.assertEqual(Path(site.WORKER_CONFIG).parent, Path(site.CA).parent)
        self.assertIn('RESTIC_PACKAGE_VERSION: &str = "0.19.1"', source('crates/backup/src/worker/restic.rs'))
        self.assertTrue(site.REPOSITORY.startswith('rest:https://192.168.100.111:18443/' + site.REST_USERNAME + '/'))
        worker_unit = (site.TEMPLATES / 'studiolo-backup-worker.service').read_text(encoding='utf-8')
        self.assertIn('ExecStart=' + site.WORKER_BINARY + ' run', worker_unit)
        self.assertIn('User=' + site.WORKER + '\n', worker_unit)
        self.assertIn('SupplementaryGroups=' + site.READERS + '\n', worker_unit)
        self.assertEqual(re.findall(r'^LoadCredential=(.*)$', worker_unit, re.M),
                         [name + ':' + credentials.DIRECTORY + '/' + name for name in credentials.NAMES])
        slots = source('crates/backup-worker/src/credentials/file.rs')
        for name in credentials.NAMES:
            self.assertIn('"' + name + '"', slots)
        for name in site.UNITS:
            self.assertTrue((site.TEMPLATES / name).is_file())
        self.assertEqual(credentials.SERVICE, site.UNITS[0])
        self.assertEqual(site.tmpfiles_text(),
                         'd /run/studiolo-backup 0750 maxsong studiolo-backup-read -\n'
                         'd /var/lib/studiolo-backup-staging 0750 maxsong studiolo-backup-read -\n')


class StepTests(unittest.TestCase):
    def provision(self, apply):
        self.ran, self.lines, self.options = [], [], None

        def run(argv, **options):
            self.ran.append(argv)
            self.options = options
        return site.Provision(apply, run=run, out=self.lines.append)

    def test_dry_run_prints_the_command_and_changes_nothing(self):
        provision = self.provision(False)
        provision.step('示例', lambda: None, lambda: provision.root('groupadd', '--system', 'x'), self.fail)
        with tempfile.TemporaryDirectory() as scratch, patch.dict(site.PLACED, {scratch + '/file': 0o644}):
            provision.root_file('文件', scratch + '/file', b'x')
            # 预演连要拷的来源都不打开（这里故意给一个不存在的）。
            provision.root_copy('拷贝', scratch + '/missing-source', scratch + '/file', 'ab' * 32)
            self.assertEqual(os.listdir(scratch), [])
            self.assertTrue(any('--place ' + scratch + '/file < ' in line for line in self.lines))
        self.assertEqual(self.ran, [])
        self.assertEqual(provision.pending, 3)
        self.assertIn('    sudo groupadd --system x', self.lines)

    def test_apply_places_root_files_through_the_create_only_entry(self):
        provision = self.provision(True)
        with tempfile.TemporaryDirectory() as scratch, patch.dict(site.PLACED, {scratch + '/file': 0o644}):
            path, origin = scratch + '/file', scratch + '/origin'
            Path(origin).write_bytes(b'binary')
            entry = ['sudo', '--', '/usr/bin/python3', '-I', '-B', str(HERE / 'prereq.py'), '--place', path]
            with self.assertRaises(site.Refused):  # 替身什么都没放：做完核对不过
                provision.root_file('文件', path, b'content')
            self.assertEqual((self.ran, self.options), ([entry], {'check': True, 'input': b'content'}))
            with self.assertRaises(site.Refused):
                provision.root_copy('拷贝', origin, path, 'ab' * 32)
            self.assertEqual(self.ran, [entry, entry])
            self.assertEqual((self.options['check'], self.options['stdin'].name), (True, origin))
            self.assertEqual(os.listdir(scratch), ['origin'])
        # 不用 install：它会盖掉已有的文件，-d 还会改掉已有目录的属主权限。
        self.assertNotIn("'install'", source('deploy/backup-worker/provision/prereq.py'))
        self.assertIn("self.root('mkdir', '-m', '0755', PROGRAMS)", source('deploy/backup-worker/provision/prereq.py'))

    def test_place_only_creates_and_leaves_nothing_behind_when_it_fails(self):
        with tempfile.TemporaryDirectory() as scratch:
            path = os.path.join(scratch, 'file')
            Path(path + '.probe').write_bytes(b'')
            probe = os.lstat(path + '.probe')
            os.unlink(path + '.probe')
            with patch.object(site, 'PLACED_OWNER', (probe.st_uid, probe.st_gid)):
                loosened = os.umask(0)  # 权限位不靠调用方的 umask
                try:
                    site.place(path, 0o555, io.BytesIO(b'content'))
                finally:
                    os.umask(loosened)
                made = os.lstat(path)
                self.assertEqual((stat.S_IMODE(made.st_mode), made.st_nlink, Path(path).read_bytes()),
                                 (0o555, 1, b'content'))
                with self.assertRaises(FileExistsError):
                    site.place(path, 0o644, io.BytesIO(b'other'))
                self.assertEqual((stat.S_IMODE(os.lstat(path).st_mode), Path(path).read_bytes()), (0o555, b'content'))
                os.symlink('/nonexistent/studiolo-test', path + '.link')  # 悬空链接也算「已经在」
                with self.assertRaises(FileExistsError):
                    site.place(path + '.link', 0o644, io.BytesIO(b'other'))

                def unreadable(size):
                    raise OSError('读到一半断了')
                with self.assertRaises(OSError):
                    site.place(path + '.second', 0o644, types.SimpleNamespace(read=unreadable))
            self.assertEqual(sorted(os.listdir(scratch)), ['file', 'file.link'])

    def test_place_entry_only_serves_root_and_the_files_this_script_installs(self):
        piped = types.SimpleNamespace(isatty=lambda: False, buffer=io.BytesIO(b'x'))
        terminal = types.SimpleNamespace(isatty=lambda: True, buffer=io.BytesIO(b'x'))
        cases = ((0, piped, ['--place', '/etc/passwd']), (1000, piped, ['--place', site.CA]),
                 (0, terminal, ['--place', site.CA]), (0, piped, ['--place', site.CA, 'extra']), (0, piped, ['--place']))
        for euid, stdin, argv in cases:
            with self.subTest(argv=argv, euid=euid), patch.object(site.os, 'geteuid', return_value=euid), \
                    patch.object(site.sys, 'stdin', stdin), patch.object(site, 'place', self.fail), \
                    self.assertRaises(site.Refused):
                site.main(argv)
        placed = []
        with patch.object(site.os, 'geteuid', return_value=0), patch.object(site.sys, 'stdin', piped), \
                patch.object(site, 'place', lambda *arguments: placed.append(arguments)):
            site.main(['--place', site.RESTIC])
        self.assertEqual(placed, [(site.RESTIC, 0o555, piped.buffer)])
        self.assertEqual(site.PLACED[site.WORKER_BINARY], 0o755)
        self.assertEqual({site.PLACED[site.UNIT_DIRECTORY + '/' + name] for name in site.UNITS}, {0o644})

    def test_chain_refuses_links_and_writable_directories_anywhere_above(self):
        with tempfile.TemporaryDirectory() as scratch:
            top, me = os.path.realpath(scratch), os.getuid()
            os.makedirs(top + '/a/b')
            for node in (top, top + '/a', top + '/a/b'):
                os.chmod(node, 0o755)
            site.chain(top + '/a/b/file', me, top=top)
            os.symlink(top + '/a', top + '/alias')
            with self.assertRaises(site.Refused):
                site.chain(top + '/alias/b/file', me, top=top)
            for mode in (0o775, 0o757, 0o1755):
                os.chmod(top + '/a', mode)
                with self.subTest(oct(mode)), self.assertRaises(site.Refused):
                    site.chain(top + '/a/b/file', me, top=top)
            os.chmod(top + '/a', 0o755)
            with self.assertRaises(site.Refused):
                site.chain(top + '/a/missing/file', me, top=top)
            Path(top + '/a/plain').write_bytes(b'')
            os.chmod(top + '/a/plain', 0o644)
            with self.assertRaises(site.Refused):  # 中间一级是普通文件
                site.chain(top + '/a/plain/file', me, top=top)
            if me != 0:  # 属主既不是 root 也不是指定账号
                with self.assertRaises(site.Refused):
                    site.chain(top + '/a/b/file', me + 1, top=top)
            site.chain(top + '/a/b/file', me, top=top)
        # 每个要建的东西都核了它上面的目录链。
        prereq = source('deploy/backup-worker/provision/prereq.py')
        self.assertIn('for path in (BACKUP_CONFIG, EPHEMERAL):\n            chain(path, OWNER_UID)', prereq)
        self.assertIn("for path in (WORKER_CONFIG, PROGRAMS, TMPFILES, UNIT_DIRECTORY + '/' + UNITS[0], RUNTIME, STAGING):\n"
                      '            chain(path, 0)', prereq)

    def test_apply_runs_through_sudo_and_refuses_a_failed_verification(self):
        provision = self.provision(True)
        provision.step('示例', lambda: None, lambda: provision.root('groupadd', '--system', 'x'), lambda: True)
        self.assertEqual(self.ran, [['sudo', '--', 'groupadd', '--system', 'x']])
        with self.assertRaises(site.Refused):
            provision.step('示例', lambda: None, lambda: None, lambda: False)

    def test_present_is_skipped_and_wrong_is_never_overwritten(self):
        for apply in (False, True):
            provision = self.provision(apply)
            provision.step('已有', lambda: True, self.fail, self.fail)
            with self.assertRaises(site.Refused):
                provision.step('不对', lambda: False, self.fail, self.fail)
            self.assertEqual((self.ran, provision.pending), ([], 0))

    def test_has_tells_missing_right_and_wrong_apart(self):
        with tempfile.TemporaryDirectory() as scratch:
            path = os.path.join(scratch, 'file')
            me, mine = os.getuid(), os.lstat(scratch).st_gid
            self.assertIsNone(site.has(path, stat.S_ISREG, me, mine, (0o600,)))
            descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
            os.close(descriptor)
            mine = os.lstat(path).st_gid
            self.assertTrue(site.has(path, stat.S_ISREG, me, mine, (0o600,)))
            self.assertFalse(site.has(path, stat.S_ISREG, me, mine, (0o644,)))
            self.assertFalse(site.has(path, stat.S_ISDIR, me, mine, (0o600,)))
            self.assertFalse(site.has(path, stat.S_ISREG, me + 1, mine, (0o600,)))
            os.link(path, path + '.second')
            self.assertFalse(site.has(path, stat.S_ISREG, me, mine, (0o600,)))


SECRETS = {'rest': 'fictional-rest-password', 'repository': 'fictional-repository-password'}


class CredentialTests(unittest.TestCase):
    def setUp(self):
        scratch = tempfile.TemporaryDirectory()
        self.addCleanup(scratch.cleanup)
        self.directory = os.path.join(scratch.name, 'credentials')
        for target, value in ((credentials, {'DIRECTORY': self.directory, 'FILE_OWNER': os.getuid()}),):
            for name, replacement in value.items():
                patcher = patch.object(target, name, replacement)
                patcher.start()
                self.addCleanup(patcher.stop)
        self.addCleanup(os.umask, os.umask(0o077))
        self.lines, self.ran = [], []

    def answers(self, *values):
        queue = list(values)
        return lambda prompt: queue.pop(0)

    def run_apply(self, results, ask=None):
        """results：依次是 is-active、之后每次 restic 调用的退出码。"""
        queue = list(results)

        def run(argv, **options):
            self.ran.append(argv)
            return types.SimpleNamespace(returncode=queue.pop(0))
        terminal = types.SimpleNamespace(isatty=lambda: True)
        streams = types.SimpleNamespace(stdin=terminal, stdout=terminal, stderr=terminal, platform='linux')
        ask = ask or self.answers(SECRETS['rest'], SECRETS['repository'], SECRETS['repository'])
        with patch.object(credentials.os, 'geteuid', return_value=0), patch.object(credentials, 'sys', streams):
            credentials.apply(self.lines.append, run=run, ask=ask)

    def test_secret_rules_match_what_the_worker_accepts(self):
        for value in ('', ' lead', 'trail ', 'tab\tinside', 'line\nbreak', 'x' * 4097):
            with self.subTest(value[:8]), self.assertRaises(credentials.Refused):
                credentials.validate_secret(value, 1, '口令')
        with self.assertRaises(credentials.Refused):
            credentials.validate_secret('fifteen-chars-x', credentials.REPOSITORY_PASSWORD_MINIMUM, '口令')
        self.assertEqual(credentials.validate_secret('内 含 空 格 也 行', 1, '口令'), '内 含 空 格 也 行')

    def test_prompt_refuses_mismatch_reuse_and_short_repository_password(self):
        good = credentials.prompt(self.answers('rest-secret', 'r' * 16, 'r' * 16))
        self.assertEqual(good, {'repository-password': 'r' * 16, 'rest-username': site.REST_USERNAME,
                                'rest-password': 'rest-secret'})
        for answers in (('rest-secret', 'r' * 16, 'r' * 15 + 'x'), ('r' * 16, 'r' * 16, 'r' * 16),
                        ('rest-secret', 'short', 'short')):
            with self.subTest(answers[1]), self.assertRaises(credentials.Refused):
                credentials.prompt(self.answers(*answers))

    def test_first_run_writes_three_private_files_and_initializes_as_the_worker(self):
        self.run_apply([3, 0, 0])
        self.assertEqual(stat.S_IMODE(os.lstat(self.directory).st_mode), 0o700)
        for name, value in (('repository-password', SECRETS['repository']), ('rest-username', site.REST_USERNAME),
                            ('rest-password', SECRETS['rest'])):
            path = os.path.join(self.directory, name)
            self.assertEqual(stat.S_IMODE(os.lstat(path).st_mode), 0o600)
            self.assertEqual(Path(path).read_bytes(), value.encode() + b'\n')
        self.assertEqual(self.lines[-1], 'backup_credentials=ready')
        self.assertEqual([argv[-1] for argv in self.ran[1:]], ['init', '--json'])
        said = ' '.join(self.lines) + ' '.join(' '.join(argv) for argv in self.ran)
        for secret in SECRETS.values():
            self.assertNotIn(secret, said)

    def test_restic_runs_as_the_worker_through_the_same_credential_chain(self):
        argv = credentials.restic_command('snapshots', '--json')
        self.assertEqual(argv[:5], ['systemd-run', '--quiet', '--wait', '--collect', '--pipe'])
        properties = [argv[index + 1] for index, word in enumerate(argv) if word == '-p']
        self.assertEqual(properties[:2], ['User=' + site.WORKER, 'Group=' + site.WORKER])
        self.assertEqual(properties[4:], ['LoadCredential=' + name + ':' + self.directory + '/' + name
                                          for name in credentials.NAMES])
        tail = argv[argv.index('/bin/sh'):]
        self.assertEqual(tail, ['/bin/sh', '-c', credentials.SHELL, 'sh', site.RESTIC, '--cacert',
                                              site.CA, '--no-cache', '--repo', site.REPOSITORY, 'snapshots', '--json'])
        # 口令只能从 systemd 给的凭据目录进环境变量：脚本里不许回显、不许开跟踪、不许写别的文件。
        for forbidden in ('echo', 'set -x', 'printf', '>', 'tee'):
            self.assertNotIn(forbidden, credentials.SHELL)
        self.assertIn('exec "$@"', credentials.SHELL)

    def test_second_run_keeps_the_files_and_asks_nothing(self):
        self.run_apply([3, 0, 0])
        before = {name: os.lstat(os.path.join(self.directory, name)).st_ino for name in credentials.NAMES}
        self.ran.clear()
        self.run_apply([3, 1, 0, 0], ask=self.fail)
        self.assertEqual(before, {name: os.lstat(os.path.join(self.directory, name)).st_ino for name in credentials.NAMES})
        self.assertIn('[已有] 仓库早就初始化过，这套凭据打得开，不重复初始化', self.lines)
        self.assertEqual([argv[-2:] for argv in self.ran[1:]],
                         [[site.REPOSITORY, 'init'], ['cat', 'config'], ['snapshots', '--json']])

    def test_failed_initialization_stops_and_leaves_the_files_for_a_person(self):
        with self.assertRaises(credentials.Refused):
            self.run_apply([3, 1, 1])
        self.assertEqual(credentials.credentials_state(self.directory), 'complete')
        self.assertNotIn('backup_credentials=ready', self.lines)
        with self.assertRaises(credentials.Refused):
            self.run_apply([3, 0, 1], ask=self.fail)

    def test_existing_credentials_the_worker_would_reject_stop_before_restic_runs(self):
        """restic 经 shell 读到的值会去掉结尾全部换行，多一个换行它照样连得上；worker 只去一个，会拒绝启动。"""
        good = {'repository-password': SECRETS['repository'], 'rest-username': site.REST_USERNAME,
                'rest-password': SECRETS['rest']}
        cases = {
            'two newlines': ('rest-password', SECRETS['rest'].encode() + b'\n\n'),
            'carriage return': ('rest-password', SECRETS['rest'].encode() + b'\r\n'),
            'trailing space': ('repository-password', SECRETS['repository'].encode() + b' \n'),
            'not text': ('repository-password', b'\xff\xfe-not-text\n'),
            'other account': ('rest-username', b'studiolo-backup\n'),
            'colon': ('rest-username', site.REST_USERNAME.encode() + b':x\n'),
        }
        for label, (name, raw) in cases.items():
            with self.subTest(label):
                shutil.rmtree(self.directory, ignore_errors=True)
                credentials.write_credentials(self.directory, good)
                Path(self.directory, name).write_bytes(raw)
                self.assertEqual(credentials.credentials_state(self.directory), 'complete')
                self.ran.clear()
                self.lines.clear()
                with self.assertRaises(credentials.Refused) as refusal:
                    self.run_apply([3], ask=self.fail)
                self.assertEqual(len(self.ran), 1)  # 只问过 worker 在不在跑，没到 restic
                said = str(refusal.exception) + ' '.join(self.lines)
                self.assertIn(name, said)
                for secret in SECRETS.values():
                    self.assertNotIn(secret, said)
        # 合规则的照旧接着用：结尾那一个换行有没有都行。
        shutil.rmtree(self.directory)
        credentials.write_credentials(self.directory, good)
        Path(self.directory, 'rest-password').write_bytes(SECRETS['rest'].encode())
        self.run_apply([3, 0, 0], ask=self.fail)
        self.assertEqual(self.lines[-1], 'backup_credentials=ready')

    def test_shell_snippet_exports_exactly_the_stored_values_and_stops_when_one_is_unreadable(self):
        """真的用 /bin/sh 跑一遍那段脚本：三份凭据原样进环境变量（不被 shell 再展开一次），读不到就不执行后面的命令。"""
        awkward = 'fic$HOME `id` "q" \\ \'s\' * end'
        credentials.write_credentials(self.directory, {'repository-password': SECRETS['repository'],
                                                       'rest-username': site.REST_USERNAME, 'rest-password': awkward})

        def handed_over():
            return subprocess.run(['/bin/sh', '-c', credentials.SHELL, 'sh', '/usr/bin/env'],
                                  env={'CREDENTIALS_DIRECTORY': self.directory, 'PATH': '/usr/bin:/bin'},
                                  stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
        done = handed_over()
        self.assertEqual(done.returncode, 0)
        environment = dict(line.split('=', 1) for line in done.stdout.decode().splitlines() if '=' in line)
        self.assertEqual((environment['RESTIC_PASSWORD_FILE'], environment['RESTIC_REST_USERNAME'],
                          environment['RESTIC_REST_PASSWORD']),
                         (self.directory + '/repository-password', site.REST_USERNAME, awkward))
        os.unlink(os.path.join(self.directory, 'rest-password'))
        done = handed_over()
        self.assertNotEqual(done.returncode, 0)
        self.assertEqual(done.stdout, b'')

    def test_partial_or_loose_credentials_are_never_touched(self):
        self.assertEqual(credentials.credentials_state(self.directory), 'absent')
        values = {name: 'value-of-' + name for name in credentials.NAMES}
        # 目录先被别人建好（哪怕是空的）也不接着用：只认自己这一次新建的。
        os.mkdir(self.directory, 0o755)
        with self.assertRaises(FileExistsError):
            credentials.write_credentials(self.directory, values)
        self.assertEqual(os.listdir(self.directory), [])
        os.rmdir(self.directory)
        # 权限位不靠调用方的 umask：放开 umask 写，目录仍是 0700、文件仍是 0600。
        loosened = os.umask(0)
        try:
            credentials.write_credentials(self.directory, values)
        finally:
            os.umask(loosened)
        self.assertEqual([stat.S_IMODE(os.lstat(self.directory).st_mode)]
                         + [stat.S_IMODE(os.lstat(os.path.join(self.directory, name)).st_mode)
                            for name in credentials.NAMES], [0o700, 0o600, 0o600, 0o600])
        self.assertEqual(credentials.credentials_state(self.directory), 'complete')
        # 目录被放开、多一个硬链接、或者有一份是空的，都不算完整。
        os.chmod(self.directory, 0o750)
        self.assertEqual(credentials.credentials_state(self.directory), 'partial')
        os.chmod(self.directory, 0o700)
        second = os.path.join(os.path.dirname(self.directory), 'second-name')
        os.link(os.path.join(self.directory, 'rest-username'), second)
        self.assertEqual(credentials.credentials_state(self.directory), 'partial')
        os.unlink(second)
        self.assertEqual(credentials.credentials_state(self.directory), 'complete')
        username = Path(self.directory, 'rest-username')
        kept = username.read_bytes()
        username.write_bytes(b'')
        self.assertEqual(credentials.credentials_state(self.directory), 'partial')
        username.write_bytes(kept)
        self.assertEqual(credentials.credentials_state(self.directory), 'complete')
        with self.assertRaises(FileExistsError):
            credentials.write_credentials(self.directory, {name: 'other' for name in credentials.NAMES})
        loose = os.path.join(self.directory, 'rest-password')
        os.chmod(loose, 0o644)
        self.assertEqual(credentials.credentials_state(self.directory), 'partial')
        with self.assertRaises(credentials.Refused):
            self.run_apply([3], ask=self.fail)
        os.chmod(loose, 0o600)
        Path(self.directory, 'extra').write_bytes(b'x')
        self.assertEqual(credentials.credentials_state(self.directory), 'partial')
        os.unlink(os.path.join(self.directory, 'extra'))
        os.unlink(loose)
        self.assertEqual(credentials.credentials_state(self.directory), 'partial')
        self.assertEqual(self.ran, [['systemctl', 'is-active', '--quiet', credentials.SERVICE]])

    def test_apply_needs_root_a_terminal_and_a_stopped_worker(self):
        pipe = types.SimpleNamespace(isatty=lambda: False)
        terminal = types.SimpleNamespace(isatty=lambda: True)
        running = lambda argv, **options: types.SimpleNamespace(returncode=0)
        for euid, stream, run in ((1000, terminal, self.fail), (0, pipe, self.fail), (0, terminal, running)):
            streams = types.SimpleNamespace(stdin=terminal, stdout=stream, stderr=terminal, platform='linux')
            with self.subTest(euid=euid), patch.object(credentials.os, 'geteuid', return_value=euid), \
                    patch.object(credentials, 'sys', streams), self.assertRaises(credentials.Refused):
                credentials.apply(self.lines.append, run=run, ask=self.fail)
        self.assertFalse(os.path.lexists(self.directory))


LINE = b'studiolo-backup-worker:$2y$12$' + b'B' * 53 + b'\n'
OLD = b'studiolo-backup:$2y$12$' + b'A' * 53 + b'\n'


class NasAccountTests(unittest.TestCase):
    def setUp(self):
        scratch = tempfile.TemporaryDirectory()
        self.addCleanup(scratch.cleanup)
        self.root = Path(scratch.name)
        self.addCleanup(os.umask, os.umask(0o077))
        self.target = self.root / 'backup.htpasswd'
        self.install(OLD)
        status = os.lstat(str(self.target))
        for name, value in (('ROOT', self.root), ('ACCOUNT_FILE_OWNER', (status.st_uid, status.st_gid))):
            patcher = patch.object(nas, name, value)
            patcher.start()
            self.addCleanup(patcher.stop)

    def install(self, raw, mode=0o640):
        self.target.write_bytes(raw)
        os.chmod(str(self.target), mode)

    def test_append_keeps_every_old_byte_and_leaves_a_copy_of_the_old_file(self):
        nas.append(LINE)
        self.assertEqual(self.target.read_bytes(), OLD + LINE)
        self.assertEqual(stat.S_IMODE(os.lstat(str(self.target)).st_mode), 0o640)
        self.assertEqual(sorted(os.listdir(str(self.root))),
                         ['backup.htpasswd', 'backup.htpasswd.before-studiolo-backup-worker'])
        self.assertEqual((self.root / 'backup.htpasswd.before-studiolo-backup-worker').read_bytes(), OLD)
        self.assertEqual(nas.accounts(self.target.read_bytes()), ['studiolo-backup', 'studiolo-backup-worker'])

    def test_missing_final_newline_does_not_glue_two_accounts_together(self):
        self.install(OLD[:-1])
        nas.append(LINE)
        self.assertEqual(self.target.read_bytes(), OLD + LINE)

    def test_only_one_exact_bcrypt_line_for_the_fixed_account_is_accepted(self):
        for raw in (b'', LINE[:-1], LINE + LINE, LINE.replace(b'$12$', b'$10$'), LINE.replace(b'-worker', b''),
                    b'studiolo-backup-worker:plain\n', LINE[:-2] + b'!\n', b' ' + LINE):
            with self.subTest(raw[:30]), self.assertRaises(ValueError):
                nas.append(raw)
        self.assertEqual(os.listdir(str(self.root)), ['backup.htpasswd'])
        self.assertEqual(self.target.read_bytes(), OLD)

    def test_existing_account_or_unexpected_file_is_refused_untouched(self):
        self.install(OLD + LINE)
        with self.assertRaises(ValueError):
            nas.append(LINE)
        self.install(OLD, mode=0o644)
        with self.assertRaises(ValueError):
            nas.append(LINE)
        self.install(OLD)
        with patch.object(nas, 'ACCOUNT_FILE_OWNER', (nas.ACCOUNT_FILE_OWNER[0] + 1, nas.ACCOUNT_FILE_OWNER[1])), \
                self.assertRaises(ValueError):
            nas.append(LINE)
        self.assertEqual(os.listdir(str(self.root)), ['backup.htpasswd'])
        self.assertEqual(self.target.read_bytes(), OLD)

    def test_a_rerun_after_an_interrupted_attempt_checks_the_leftover_copy(self):
        keep = self.root / 'backup.htpasswd.before-studiolo-backup-worker'
        keep.write_bytes(OLD)
        nas.append(LINE)
        self.assertEqual(self.target.read_bytes(), OLD + LINE)
        self.install(OLD)
        keep.write_bytes(b'somebody-else:x\n')
        with self.assertRaises(ValueError):
            nas.append(LINE)
        self.assertEqual(self.target.read_bytes(), OLD)

    def test_a_failed_write_or_replace_leaves_no_half_written_file(self):
        keep = 'backup.htpasswd.before-studiolo-backup-worker'
        with patch.object(nas.os, 'fsync', side_effect=OSError('盘满了')), self.assertRaises(OSError):
            nas.append(LINE)
        self.assertEqual(os.listdir(str(self.root)), ['backup.htpasswd'])
        with patch.object(nas.os, 'replace', side_effect=OSError('换不上')), self.assertRaises(OSError):
            nas.append(LINE)
        # 改之前的副本是完整的，留着；带新散列的那份不留。
        self.assertEqual(sorted(os.listdir(str(self.root))), ['backup.htpasswd', keep])
        self.assertEqual((self.target.read_bytes(), (self.root / keep).read_bytes()), (OLD, OLD))
        nas.append(LINE)
        self.assertEqual(self.target.read_bytes(), OLD + LINE)
        self.assertEqual(sorted(os.listdir(str(self.root))), ['backup.htpasswd', keep])

    def test_only_one_append_runs_at_a_time(self):
        """第一处读完旧内容、还没换上新文件，第二处进来：第二处被拒，第一处照常做完。"""
        real, seen = nas.append_locked, []

        def first_then_second(raw):
            if not seen:
                seen.append('第一处进来了')
                with self.assertRaises(ValueError) as refusal:
                    nas.append(LINE.replace(b'B', b'C'))
                seen.append(str(refusal.exception))
            real(raw)
        with patch.object(nas, 'append_locked', first_then_second):
            nas.append(LINE)
        self.assertEqual(seen, ['第一处进来了', '另一处正在改账号文件：等它结束再跑'])
        self.assertEqual(self.target.read_bytes(), OLD + LINE)
        # 别的进程拿着这个目录的锁（哪一种都算）时也一样；放开之后才做。
        self.install(OLD)
        os.unlink(str(self.root / 'backup.htpasswd.before-studiolo-backup-worker'))
        for kind in (fcntl.LOCK_EX, fcntl.LOCK_SH):
            other = os.open(str(self.root), os.O_RDONLY)
            try:
                fcntl.flock(other, kind)
                with self.assertRaises(ValueError):
                    nas.append(LINE)
                self.assertEqual((os.listdir(str(self.root)), self.target.read_bytes()), (['backup.htpasswd'], OLD))
            finally:
                os.close(other)
        nas.append(LINE)
        self.assertEqual(self.target.read_bytes(), OLD + LINE)

    def test_entry_needs_root_and_a_known_mode(self):
        for euid, argv in ((1000, ['check']), (0, []), (0, ['delete']), (0, ['check', 'append'])):
            with self.subTest(argv), patch.object(nas.os, 'geteuid', return_value=euid), self.assertRaises(ValueError):
                nas.main(argv, out=self.fail)

    def test_check_reports_names_and_modes_but_no_hash(self):
        lines = []
        with patch.object(nas, 'state', return_value='inactive'), patch.object(nas, 'listening', return_value=False):
            nas.check(lines.append)
        said = '\n'.join(lines)
        self.assertIn('现有账号：studiolo-backup', said)
        self.assertIn('没人监听', said)
        self.assertIn('backup.htpasswd：-rw-r-----', said)
        self.assertIn('tls.key：不在', said)
        self.assertIn('账号文件旁边的其他文件（只列名字）：没有', said)
        nas.append(LINE)
        lines.clear()
        with patch.object(nas, 'state', return_value='inactive'), patch.object(nas, 'listening', return_value=False):
            nas.check(lines.append)
        said += '\n'.join(lines)
        self.assertIn('账号文件旁边的其他文件（只列名字）：backup.htpasswd.before-studiolo-backup-worker', said)
        self.assertNotIn('$2y$', said)
        self.assertNotIn('A' * 53, said)
        self.assertNotIn('B' * 53, said)


class EntryTests(unittest.TestCase):
    def test_nas_helper_only_uses_syntax_the_nas_python_understands(self):
        ast.parse((HERE / 'nas_account.py').read_text(encoding='utf-8'), feature_version=(3, 8))

    def test_names_are_the_same_on_all_three_machines(self):
        self.assertEqual(nas.USER, site.REST_USERNAME)
        self.assertTrue(nas.LINE.pattern.startswith(nas.USER.encode() + b':'))
        self.assertIn('account=' + nas.USER + '\n', COMMAND)
        self.assertIn('helper="$here/nas_account.py"', COMMAND)
        self.assertEqual('rest:https://' + nas.ADDRESS[0] + ':' + str(nas.ADDRESS[1]) + '/' + nas.USER + '/main/',
                         site.REPOSITORY)
        recovery = source('deploy/sync-bridge/recovery/studiolo-rest-backup.service')
        self.assertIn('--listen ' + nas.ADDRESS[0] + ':' + str(nas.ADDRESS[1]) + ' ', recovery)
        self.assertIn('--userspec=' + str(nas.RUNTIME_ID) + ':' + str(nas.RUNTIME_ID), recovery)
        self.assertIn(str(nas.ROOT) + ' /rest-server', recovery)
        self.assertIn('--append-only --private-repos --htpasswd-file /backup.htpasswd', recovery)

    def test_command_is_valid_shell_asks_on_the_terminal_and_never_traces(self):
        subprocess.run(['bash', '-n', str(HERE / 'nas-account.command')], check=True)
        code = [line for line in COMMAND.splitlines() if not line.lstrip().startswith('#')]
        # 密码先在本机核规则，再只经管道给 htpasswd：不用 -b（那样密码会进命令行）。
        self.assertIn('  valid_password "$password" || { echo \'密码不合上面的要求。什么都没改。\'; exit 1; }', code)
        self.assertIn('  printf \'%s\\n\' "$password" | /usr/sbin/htpasswd -c -i -B -C 12 "$scratch/line" "$account"', code)
        self.assertLess(code.index('  valid_password "$password" || { echo \'密码不合上面的要求。什么都没改。\'; exit 1; }'),
                        code.index('  printf \'%s\\n\' "$password" | /usr/sbin/htpasswd -c -i -B -C 12 "$scratch/line" "$account"'))
        self.assertNotRegex(COMMAND, r'htpasswd[^\n]* -b\b')
        self.assertEqual(COMMAND.count('htpasswd -'), 1)
        self.assertIn("[[ -t 0 && -t 1 && -t 2 ]]", COMMAND)
        self.assertIn('trap cleanup EXIT', COMMAND)
        self.assertNotIn('set -x', COMMAND)
        # 每一次 ssh 都不要远端终端（有终端会把输入回显出来）；NAS 上不落文件，所以没有往远端写、从远端删的命令。
        calls = [line for line in code if re.search(r'\bssh ', line)]
        self.assertEqual(len(calls), 2)
        for line in calls:
            self.assertIn('ssh -T -o BatchMode=yes "${nas[@]}" "sudo -n python3 -I -B -', line)
        for forbidden in ('cat >', 'scp ', 'rm -f $'):
            self.assertNotIn(forbidden, COMMAND)
        done = subprocess.run(['bash', str(HERE / 'nas-account.command')], stdout=subprocess.PIPE)
        self.assertEqual(done.returncode, 2)
        self.assertIn('用法', done.stdout.decode())

    def test_password_rule_is_inside_what_the_worker_accepts(self):
        """NAS 上加了账号、169 却不认这个密码，就得去 NAS 上手工删行：所以本机这条规则只许比 worker 的更紧。"""
        def accepted(value):
            return subprocess.run(['bash', '-c', 'source "$1"; valid_password "$2"', 'probe',
                                   str(HERE / 'nas-account.command'), value]).returncode == 0
        # 按字节判断，不跟着本机的语言环境走（有的语言环境里字符范围按排序规则算）。
        self.assertIn('valid_password() {\n  local LC_ALL=C\n  [[ ${#1} -ge 16 && ${#1} -le 1024 && $1 =~ ^[!-~]+$ ]]\n}\n', COMMAND)
        for value in ('fictional-Password_123', 'a' * 16, '!~' * 8, 'x' * 1024, '$`"\\\'*?[]{}()<>|&;#=' + 'z' * 16):
            with self.subTest(value[:12]):
                self.assertTrue(accepted(value))
                self.assertEqual(credentials.validate_secret(value, 1, '口令'), value)
        for value in ('', 'a' * 15, 'x' * 1025, 'with space inside 123', 'trailing-space-12345 ', ' leading-space-12345',
                      'tab\tinside-123456789', 'unicode-密码-1234567890', 'nbsp-at-the-end-123\u00a0'):
            with self.subTest(value[:12]):
                self.assertFalse(accepted(value))

    def test_append_reads_the_helper_and_leaves_exactly_the_account_line_on_stdin(self):
        """追加那一步帮手脚本和账号行走同一条标准输入：先按字节数读出脚本来执行，剩下的正好是那一行。"""
        boot = re.search(r"^boot='(.*)'$", COMMAND, re.M).group(1)
        self.assertIn('  size=$(( $(wc -c < "$helper") ))\n  { cat "$helper"; cat "$scratch/line"; } \\\n'
                      '    | ssh -T -o BatchMode=yes "${nas[@]}" "sudo -n python3 -I -B -c \'$boot\' append $size"\n', COMMAND)
        stand_in = b'import sys\nsys.stdout.write(repr((sys.argv[1:], __name__, sys.stdin.buffer.read(256))))\n'
        done = subprocess.run([sys.executable, '-I', '-B', '-c', boot, 'append', str(len(stand_in))],
                              input=stand_in + LINE, stdout=subprocess.PIPE, check=True)
        self.assertEqual(done.stdout.decode(), repr((['append'], '__main__', LINE)))
        # 真的帮手脚本经这条路也是从自己的入口进去的（这里不是 root，或者没有 NAS 那个目录，都在入口被拒）。
        helper = (HERE / 'nas_account.py').read_bytes()
        done = subprocess.run([sys.executable, '-I', '-B', '-c', boot, 'append', str(len(helper))],
                              input=helper + LINE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        self.assertEqual((done.returncode, done.stdout), (1, b''))
        self.assertIn('nas_account=failed', done.stderr.decode())
        self.assertNotIn('B' * 53, done.stderr.decode())

    def test_scripts_refuse_unknown_arguments_before_touching_anything(self):
        for module in (site, credentials):
            with self.subTest(module.__name__), self.assertRaises(module.Refused):
                module.main(['--force'])


if __name__ == '__main__':
    unittest.main()
