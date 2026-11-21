"""credentials.py 的三件事：录口令之前把前提核全、只认批准的那一张证书、三份凭据要么都在要么一份不留。

证书那几条真的在 127.0.0.1 上起一个 TLS 服务端，证书是用例当场用 openssl 生成的虚构证书；不连任何别的机器。
凭据写在临时目录里，口令全是虚构值。由同目录的 test_provision.py 并进去一起跑，也可以单独跑。
"""
import contextlib
import hashlib
import importlib.util
import io
import os
from pathlib import Path
import signal
import socket
import ssl
import subprocess
import tempfile
import threading
import types
import unittest
from unittest.mock import patch

HERE = Path(__file__).resolve().parent


def load(name):
    spec = importlib.util.spec_from_file_location(name, HERE / (name + '.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


credentials = load('credentials')
# credentials.py 自己加载了一份 prereq.py：它调的、读的都是这一份，替身也要换在这一份上。
prereq = credentials.prereq
VALUES = {'repository-password': 'fictional-repository-password', 'rest-username': prereq.REST_USERNAME,
          'rest-password': 'fictional-rest-password'}
SETTLED = 16


class PreconditionTests(unittest.TestCase):
    """预演和照做都先过前提：在 169 上、prereq.py 的产物一样不少、NAS 连得上而且证书就是批准的那一张。"""

    def setUp(self):
        scratch = tempfile.TemporaryDirectory()
        self.addCleanup(scratch.cleanup)
        self.directory = os.path.join(scratch.name, 'credentials')

    def main(self, argv, settled=SETTLED, endpoint='TLSv1.3', platform='linux', euid=1000):
        """返回（打出来的行，走到了哪几步）。settled、endpoint 给异常就是那一步失败。"""
        lines, calls = [], []

        def step(name, outcome):
            def call(*arguments):
                calls.append(name)
                if isinstance(outcome, BaseException):
                    raise outcome
                return outcome
            return call
        streams = types.SimpleNamespace(platform=platform)
        with patch.object(prereq, 'settled', step('settled', settled)), \
                patch.object(credentials, 'endpoint', step('endpoint', endpoint)), \
                patch.object(credentials, 'apply', step('apply', None)), patch.object(credentials, 'sys', streams), \
                patch.object(credentials.os, 'geteuid', return_value=euid), \
                patch.object(credentials, 'DIRECTORY', self.directory):
            try:
                credentials.main(argv, out=lines.append)
            except credentials.Refused as refusal:
                lines.append(refusal)
        return lines, calls

    def test_a_preview_checks_everything_and_then_stops_without_asking_or_writing(self):
        lines, calls = self.main([])
        self.assertEqual(calls, ['settled', 'endpoint'])
        self.assertEqual(lines, [
            '[已核] prereq.py 的 16 样产物都在、都对（账号和组、目录、程序和证书、两份配置、三个单元）',
            '[已核] NAS 备份服务连得上（TLSv1.3），证书就是批准的那一张',
            '凭据目录现在：还没有',
            '预演结束：没问口令、没写任何东西。带 sudo 和 --apply 再跑一遍才录入。'])
        self.assertFalse(os.path.lexists(self.directory))
        # 不是 root 时凭据目录里看不到东西：只说在不在。
        os.mkdir(self.directory)
        self.assertEqual(self.main([])[0][2], '凭据目录现在：已经在（里面要 root 才看得到）')
        self.assertEqual(self.main([], euid=0)[0][2], '凭据目录现在：在，但不完整（要人看）')
        os.rmdir(self.directory)
        self.assertEqual(self.main([], euid=0)[0][2], '凭据目录现在：还没有')

    def test_apply_is_reached_only_after_every_precondition_holds(self):
        self.assertEqual(self.main(['--apply']), ([
            '[已核] prereq.py 的 16 样产物都在、都对（账号和组、目录、程序和证书、两份配置、三个单元）',
            '[已核] NAS 备份服务连得上（TLSv1.3），证书就是批准的那一张'], ['settled', 'endpoint', 'apply']))

    def test_each_failed_precondition_stops_before_the_next_step_in_both_modes(self):
        unsettled = credentials.Refused('worker 配置 /etc/studiolo/backup-worker.json：已经在，但不是预期的样子。不覆盖，先由人看过')
        pinned = credentials.Refused('NAS 现在用的证书不是批准的那一张（指纹 ' + 'ab' * 32 + '）')
        nas = '。先在 NAS 那边看 nas-account.command check'
        cases = (
            ({'platform': 'darwin'}, [], '只在 169 上跑'),
            # prereq.py 停在半路、或者它的某样产物后来被改了：不问口令、不连 NAS。
            ({'settled': unsettled}, ['settled'], str(unsettled)),
            ({'endpoint': ConnectionRefusedError(111, 'Connection refused')}, ['settled', 'endpoint'],
             'NAS 的备份服务连不上或证书验不过（ConnectionRefusedError：[Errno 111] Connection refused）' + nas),
            ({'endpoint': ssl.SSLCertVerificationError(1, 'certificate verify failed')}, ['settled', 'endpoint'],
             'NAS 的备份服务连不上或证书验不过（SSLCertVerificationError：certificate verify failed）' + nas),
            ({'endpoint': TimeoutError('timed out')}, ['settled', 'endpoint'],
             'NAS 的备份服务连不上或证书验不过（TimeoutError：timed out）' + nas),
            # 握手过了、但不是批准的那一张：原话照传，不改成「连不上」。
            ({'endpoint': pinned}, ['settled', 'endpoint'], str(pinned)),
            # 错误的原话只取开头一段（里面是对端给的东西，不让它刷屏）。
            ({'endpoint': OSError('x' * 500)}, ['settled', 'endpoint'],
             'NAS 的备份服务连不上或证书验不过（OSError：' + 'x' * 120 + '）' + nas),
        )
        for argv in ([], ['--apply']):
            for failing, reached, told in cases:
                with self.subTest(argv=argv, failing=sorted(failing)):
                    lines, calls = self.main(argv, **failing)
                    self.assertEqual((calls, str(lines[-1])), (reached, told))
                    self.assertIsInstance(lines[-1], credentials.Refused)
                    self.assertFalse(os.path.lexists(self.directory))
        # 没预料到的错不吞。
        with self.assertRaises(ZeroDivisionError):
            self.main([], endpoint=ZeroDivisionError())


class EndpointTests(unittest.TestCase):
    """证书钉扎，对着真的 TLS 握手：验得过、而且就是批准的那一张才放过。"""

    @classmethod
    def setUpClass(cls):
        scratch = tempfile.TemporaryDirectory()
        cls.addClassCleanup(scratch.cleanup)
        cls.scratch = scratch.name

        def openssl(*argv):
            done = subprocess.run(['/usr/bin/openssl'] + list(argv), cwd=cls.scratch, stdin=subprocess.DEVNULL,
                                  stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
            assert done.returncode == 0, done.stdout.decode(errors='replace')
        # 不靠这台机器自己的 openssl.cnf（有的机器没有，有的会自己往证书里加东西）：要什么扩展在这里写明。
        Path(cls.scratch, 'request.cnf').write_text('[req]\ndistinguished_name = name\n[name]\n', encoding='ascii')
        key = ('-config', 'request.cnf', '-newkey', 'ec', '-pkeyopt', 'ec_paramgen_curve:prime256v1', '-nodes')
        address = 'subjectAltName=IP:127.0.0.1'
        # approved、other：两张互不相干的自签证书（NAS 上的就是自签：服务端证书和 CA 是同一张）。
        # elsewhere：也是自签，但发给的是另一个地址。
        for name, issued in (('approved', address), ('other', address), ('elsewhere', 'subjectAltName=IP:127.0.0.2')):
            openssl('req', '-x509', *key, '-keyout', name + '.key', '-out', name + '.crt', '-days', '2',
                    '-subj', '/CN=fictional-' + name, '-addext', issued)
        # authority 签出来的 leaf：用 authority 验得过，却不是 authority 那一张。
        openssl('req', '-x509', *key, '-keyout', 'authority.key', '-out', 'authority.crt', '-days', '2',
                '-subj', '/CN=fictional-authority', '-addext', 'basicConstraints=critical,CA:TRUE')
        openssl('req', '-new', *key, '-keyout', 'leaf.key', '-out', 'leaf.csr', '-subj', '/CN=fictional-leaf')
        Path(cls.scratch, 'leaf.ext').write_text(address + '\n', encoding='ascii')
        openssl('x509', '-req', '-in', 'leaf.csr', '-CA', 'authority.crt', '-CAkey', 'authority.key', '-CAcreateserial',
                '-out', 'leaf.crt', '-days', '2', '-extfile', 'leaf.ext')

    def path(self, name):
        return os.path.join(self.scratch, name)

    def fingerprint(self, name):
        return hashlib.sha256(ssl.PEM_cert_to_DER_cert(Path(self.path(name + '.crt')).read_text(encoding='ascii'))).hexdigest()

    def serve(self, name):
        """在 127.0.0.1 的一个空闲端口上用这张证书应答 TLS 握手；返回端口。用例结束时关掉。"""
        context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        context.load_cert_chain(self.path(name + '.crt'), self.path(name + '.key'))
        listener = socket.create_server(('127.0.0.1', 0))
        listener.settimeout(0.1)
        stopping = threading.Event()

        def answer():
            while not stopping.is_set():
                try:
                    raw, _ = listener.accept()
                except socket.timeout:
                    continue
                except OSError:
                    return
                raw.settimeout(5)
                try:
                    with context.wrap_socket(raw, server_side=True) as secured:
                        secured.recv(1)  # 等对端关
                except OSError:
                    raw.close()
        thread = threading.Thread(target=answer, daemon=True)
        thread.start()

        def stop():
            stopping.set()
            listener.close()
            thread.join(10)
            assert not thread.is_alive()
        self.addCleanup(stop)
        return listener.getsockname()[1]

    @contextlib.contextmanager
    def approving(self, name, port):
        repository = 'rest:https://127.0.0.1:' + str(port) + '/' + prereq.REST_USERNAME + '/main/'
        with patch.object(prereq, 'CA', self.path(name + '.crt')), patch.object(prereq, 'REPOSITORY', repository):
            yield

    def test_the_approved_self_signed_certificate_is_accepted(self):
        with self.approving('approved', self.serve('approved')):
            self.assertIn(credentials.endpoint(), ('TLSv1.3', 'TLSv1.2'))
            lines = []
            with patch.object(prereq, 'settled', return_value=SETTLED), \
                    patch.object(credentials, 'sys', types.SimpleNamespace(platform='linux')):
                credentials.preconditions(lines.append)
            self.assertIn(lines[-1], ['[已核] NAS 备份服务连得上（' + version + '），证书就是批准的那一张'
                                      for version in ('TLSv1.3', 'TLSv1.2')])

    def test_another_certificate_fails_the_handshake(self):
        with self.approving('approved', self.serve('other')):
            with self.assertRaises(ssl.SSLCertVerificationError):
                credentials.endpoint()
            with patch.object(prereq, 'settled', return_value=SETTLED), \
                    patch.object(credentials, 'sys', types.SimpleNamespace(platform='linux')), \
                    self.assertRaises(credentials.Refused) as refusal:
                credentials.preconditions(lambda line: None)
            self.assertIn('NAS 的备份服务连不上或证书验不过（SSLCertVerificationError：', str(refusal.exception))

    def test_the_approved_certificate_must_also_be_issued_for_this_address(self):
        """就是批准的那一张，但不是发给这个地址的：restic 连的时候会拒绝，这里先拦下（不然口令录完才发现连不上）。"""
        with self.approving('elsewhere', self.serve('elsewhere')), \
                self.assertRaises(ssl.SSLCertVerificationError) as failure:
            credentials.endpoint()
        self.assertIn('IP address mismatch', str(failure.exception))

    def test_a_certificate_that_verifies_but_is_not_the_approved_one_is_refused(self):
        """握手验得过（它是批准的那张 CA 签出来的），但 NAS 给的不是批准的那一张本身：只有指纹比对拦得住。"""
        with self.approving('authority', self.serve('leaf')):
            with self.assertRaises(credentials.Refused) as refusal:
                credentials.endpoint()
        self.assertEqual(str(refusal.exception), 'NAS 现在用的证书不是批准的那一张（指纹 ' + self.fingerprint('leaf') + '）')
        self.assertNotEqual(self.fingerprint('leaf'), self.fingerprint('authority'))

    def test_nothing_listening_or_an_unreadable_certificate_is_an_error_not_a_pass(self):
        spare = socket.create_server(('127.0.0.1', 0))
        port = spare.getsockname()[1]
        spare.close()
        with self.approving('approved', port), self.assertRaises(ConnectionRefusedError):
            credentials.endpoint()
        with self.approving('missing', self.serve('approved')), self.assertRaises(FileNotFoundError):
            credentials.endpoint()


class AtomicWriteTests(unittest.TestCase):
    """三份凭据要么都在、要么一份都不留：每一步上出错或被打断都一样；别人的东西不碰。"""

    def setUp(self):
        scratch = tempfile.TemporaryDirectory()
        self.addCleanup(scratch.cleanup)
        self.scratch = scratch.name
        self.directory = os.path.join(self.scratch, 'credentials')
        self.writing = self.directory + credentials.WRITING
        for name, value in (('DIRECTORY', self.directory), ('FILE_OWNER', os.getuid())):
            patcher = patch.object(credentials, name, value)
            patcher.start()
            self.addCleanup(patcher.stop)
        self.addCleanup(os.umask, os.umask(0o077))

    def left(self):
        return sorted(os.listdir(self.scratch))

    def nth(self, name, number, failure=None, before=None):
        """把 credentials.py 用的 os.<name> 换成：第 number 次调用时，先做 before（有的话），
        照常做完，然后（给了 failure 的话）抛出它。别的次数照常。"""
        real, count = getattr(os, name), [0]

        def call(*arguments, **options):
            count[0] += 1
            if count[0] == number and before is not None:
                before(*arguments)
            result = real(*arguments, **options)
            if count[0] == number and failure is not None:
                raise failure
            return result
        return patch.object(credentials.os, name, call)

    def test_a_complete_write_leaves_exactly_the_three_files_and_no_temporary_directory(self):
        self.assertEqual(self.writing, self.directory + '.writing')
        credentials.write_credentials(self.directory, VALUES)
        self.assertEqual((self.left(), credentials.credentials_state(self.directory)), (['credentials'], 'complete'))
        self.assertEqual({name: Path(self.directory, name).read_bytes() for name in credentials.NAMES},
                         {name: value.encode() + b'\n' for name, value in VALUES.items()})
        self.assertEqual(credentials.read_credentials(self.directory), VALUES)

    def test_an_error_or_an_interruption_at_any_step_leaves_nothing_behind(self):
        interrupted = prereq.Interrupted('SIGHUP')
        # （哪个调用，第几次，那一次做完之后抛什么）：建临时目录、建三份文件、各自落盘、临时目录落盘、建正式目录。
        steps = [('mkdir', 1, interrupted), ('mkdir', 2, interrupted), ('mkdir', 2, OSError(28, '盘满了'))]
        steps += [('open', number, failure) for number in (1, 2, 3) for failure in (interrupted, OSError(5, '写不了'))]
        steps += [('fsync', number, failure) for number in (1, 2, 3, 4) for failure in (interrupted, OSError(5, '落不了盘'))]
        for name, number, failure in steps:
            with self.subTest(step=name, number=number, failure=type(failure).__name__):
                with self.nth(name, number, failure), self.assertRaises(type(failure)):
                    credentials.write_credentials(self.directory, VALUES)
                self.assertEqual(self.left(), [])
        # 改名已经做成之后才被打断或出错（父目录落盘是最后一步）：留下的是完整的三份，下次跑直接接着用。
        for name, number, failure in (('rename', 1, interrupted), ('fsync', 5, OSError(5, '落不了盘'))):
            with self.subTest(step=name, number=number):
                with self.nth(name, number, failure), self.assertRaises(type(failure)):
                    credentials.write_credentials(self.directory, VALUES)
                self.assertEqual((self.left(), credentials.credentials_state(self.directory)), (['credentials'], 'complete'))
                for entry in credentials.NAMES:
                    os.unlink(os.path.join(self.directory, entry))
                os.rmdir(self.directory)

    def test_what_somebody_else_created_is_left_exactly_as_it_was(self):
        def theirs(path, *mode, keep=True):
            os.mkdir(path, 0o700)
            if keep:
                Path(path, 'theirs').write_bytes(b'x')
        for keep in (True, False):
            found = ['theirs'] if keep else []
            with self.subTest(keep=keep):
                # 正式目录先在（空的也算）：什么都不建，口令连临时目录都不进。
                theirs(self.directory, keep=keep)
                with self.nth('mkdir', 1, AssertionError('正式目录已经在，不该再去建临时目录')), \
                        self.assertRaises(FileExistsError):
                    credentials.write_credentials(self.directory, VALUES)
                self.assertEqual((self.left(), os.listdir(self.directory)), (['credentials'], found))
                os.rename(self.directory, self.writing)
                # 临时目录先在（上一次留下的，或者另一处正在录）：不接着用，也不删。
                with self.assertRaises(FileExistsError):
                    credentials.write_credentials(self.directory, VALUES)
                self.assertEqual((self.left(), os.listdir(self.writing)), (['credentials.writing'], found))
                for name in found:
                    os.unlink(os.path.join(self.writing, name))
                os.rmdir(self.writing)
                # 三份都写齐了、正要占正式的名字，别人抢先建了：这一次写的全删掉，别人建的原样留着（空的也不删）。
                with self.nth('mkdir', 2, before=lambda path, *mode: theirs(path, keep=keep)), \
                        self.assertRaises(FileExistsError):
                    credentials.write_credentials(self.directory, VALUES)
                self.assertEqual((self.left(), os.listdir(self.directory)), (['credentials'], found))
                for name in found:
                    os.unlink(os.path.join(self.directory, name))
                os.rmdir(self.directory)

    def apply(self, results, ask):
        ran, lines = [], []
        queue = list(results)

        def run(argv, **options):
            ran.append(argv)
            return types.SimpleNamespace(returncode=queue.pop(0))
        terminal = types.SimpleNamespace(isatty=lambda: True)
        streams = types.SimpleNamespace(stdin=terminal, stdout=terminal, stderr=terminal, platform='linux')
        with patch.object(credentials.os, 'geteuid', return_value=0), patch.object(credentials, 'sys', streams):
            credentials.apply(lines.append, run=run, ask=ask)
        return ran, lines

    def test_a_leftover_temporary_directory_stops_the_run_before_any_password_is_asked(self):
        os.mkdir(self.writing, 0o700)
        Path(self.writing, 'rest-password').write_bytes(b'left-by-a-killed-run\n')
        with self.assertRaises(credentials.Refused) as refusal:
            self.apply([3], ask=self.fail)
        self.assertEqual(str(refusal.exception), self.writing + ' 还在：上一次录到一半被强杀或断电留下的，里面可能有那一次输的口令。'
                         '确认没有别的 credentials.py 在跑，sudo rm -r 它之后重跑')
        # 留下的原样不动（不自动删：里面是什么得由人看）；正式目录没有出现。
        self.assertEqual((self.left(), os.listdir(self.writing)), (['credentials.writing'], ['rest-password']))
        # 悬空链接占着这个名字也一样。
        os.unlink(os.path.join(self.writing, 'rest-password'))
        os.rmdir(self.writing)
        os.symlink('/nonexistent/studiolo-test', self.writing)
        with self.assertRaises(credentials.Refused):
            self.apply([3], ask=self.fail)
        os.unlink(self.writing)
        # 没有残留就照常录：先问口令，再写。
        answers = [VALUES['rest-password'], VALUES['repository-password'], VALUES['repository-password']]
        ran, lines = self.apply([3, 0, 0], ask=lambda prompt: answers.pop(0))
        self.assertEqual((self.left(), lines[-1]), (['credentials'], 'backup_credentials=ready'))

    def test_an_interruption_while_the_passwords_are_being_typed_writes_nothing(self):
        for position in (0, 1, 2):
            answers = [VALUES['rest-password'], VALUES['repository-password'], VALUES['repository-password']]

            def ask(prompt, position=position):
                if len(answers) == 3 - position:
                    raise KeyboardInterrupt()
                return answers.pop(0)
            with self.subTest(position=position), self.assertRaises(KeyboardInterrupt):
                self.apply([3], ask=ask)
            self.assertEqual(self.left(), [])


class CredentialEntryTests(unittest.TestCase):
    def setUp(self):
        for number in (signal.SIGHUP, signal.SIGTERM, signal.SIGINT):
            self.addCleanup(signal.signal, number, signal.getsignal(number))

    def outcome(self, raised):
        def main(argv):
            if raised is not None:
                raise raised
        said = io.StringIO()
        with patch.object(credentials, 'main', main), contextlib.redirect_stderr(said):
            try:
                code = credentials.entry([])
            except KeyboardInterrupt:
                # 不让它一路传上去：那样整套用例会被当成「用户按了 Ctrl-C」直接中断，看不到是哪一条没过。
                self.fail('入口没有接住「被打断」')
        return code, said.getvalue()

    def test_every_expected_stop_is_one_line_and_exit_code_one(self):
        interrupted = '\n停下：被打断。三份凭据要么都写好了、要么一份都没留；重跑会从现状接着做（写好的不会再问）。\n'
        cases = (
            (credentials.Refused('worker 正在跑：先停掉再动凭据'), '停下：worker 正在跑：先停掉再动凭据\n'),
            (FileExistsError(17, '已经在', '/x'), "停下：FileExistsError：[Errno 17] 已经在: '/x'\n"),
            (PermissionError(13, '不让写'), '停下：PermissionError：[Errno 13] 不让写\n'),
            (KeyboardInterrupt(), interrupted), (prereq.Interrupted('SIGTERM'), interrupted),
        )
        for raised, told in cases:
            with self.subTest(type(raised).__name__):
                self.assertEqual(self.outcome(raised), (1, told))
        self.assertEqual(self.outcome(None), (0, ''))
        with self.assertRaises(ZeroDivisionError):
            self.outcome(ZeroDivisionError())
        # 入口先把终端断开、被要求结束接成「被打断」：不然收尾不跑，写到一半的临时目录会留下。
        self.assertNotIn(signal.getsignal(signal.SIGHUP), (signal.SIG_DFL, signal.SIG_IGN))
        self.assertNotIn(signal.getsignal(signal.SIGTERM), (signal.SIG_DFL, signal.SIG_IGN))


if __name__ == '__main__':
    unittest.main()
