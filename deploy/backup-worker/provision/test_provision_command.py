"""nas-account.command（用户在 Mac 终端里跑的入口脚本）真的跑起来：放进伪终端，按提示输入，看屏幕、退出码和它调了什么。

ssh 和 htpasswd 换成只记账的替身（bash 函数）：不连任何机器、不算任何散列。每一次 ssh 的参数和标准输入、htpasswd 的参数和
标准输入都记在临时目录里；可以指定哪一次 ssh 失败、htpasswd 给出什么。密码和散列全是虚构值。
这里跑的是测试机上的 /bin/bash；用户 Mac 上的 /bin/bash 是 3.2，不在这里跑。
由同目录的 test_provision.py 并进去一起跑，也可以单独跑。
"""
import base64
import hashlib
import importlib.util
import os
from pathlib import Path
import pty
import re
import select
import shlex
import shutil
import signal
import subprocess
import sys
import tempfile
import termios
import time
import types
import unittest

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]


def load(name):
    spec = importlib.util.spec_from_file_location(name, HERE / (name + '.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


nas = load('nas_account')
credentials = load('credentials')
COMMAND = HERE / 'nas-account.command'
TEXT = COMMAND.read_text(encoding='utf-8')
# 去掉注释行之后的脚本正文：静态断言只看会执行的部分。
CODE = '\n'.join(line for line in TEXT.splitlines() if not line.lstrip().startswith('#'))
HELPER = (HERE / 'nas_account.py').read_bytes()
UNIT = (ROOT / 'deploy/sync-bridge/recovery/studiolo-rest-backup.service').read_bytes()
KNOWN_HOSTS = HERE / 'nas_known_hosts'
README = ROOT / 'deploy/backup-worker/README.md'
# NAS 主机钥匙的指纹。NAS 换了钥匙要改四处：nas_known_hosts 里那一行和文件头注释、README「NAS 的主机钥匙」、这里。
FINGERPRINT = 'SHA256:Hkpz7ou4/hH99ZGBZH/vuP+Hm8ipHWpXEr+cLdVf1L8'
BASH = '/bin/bash'
TOOLS = '/usr/bin:/bin:/usr/sbin'
LIMIT = 30

BOOT = re.search(r"^boot='([^'\n]+)'$", TEXT, re.M).group(1)
REMOTE = 'sudo -n python3 -I -B '
PIN = ['-o', 'StrictHostKeyChecking=yes', '-o', 'UserKnownHostsFile="' + str(KNOWN_HOSTS) + '"',
       '-o', 'GlobalKnownHostsFile=/dev/null', '-o', 'HostKeyAlias=studiolo-nas', '-o', 'UpdateHostKeys=no']
NAS = ['-J', 'studiolo-108-ts', 'maxsong@192.168.100.111']

# 虚构值。散列那一段只是 53 个形状对的字符，不是任何密码的散列。
PASSWORD = 'fictional-Passw0rd!#%'
HASH = ('abcXYZ019./' * 5)[:53]
LINE = 'studiolo-backup-worker:$2y$12$' + HASH

GREETING = '登录 NAS（{}）。要是问密码，问的是 NAS 上 maxsong 的登录密码。'
ASKING = ['给 NAS 上的新账号 studiolo-backup-worker 定一个密码（只能追加备份，不能删改）。先存进密码管理器，再到这里输入。',
          '要求：16 到 72 个字符，只用英文字母、数字和常见符号，不含空格。']
FIRST = '密码（不回显）：'
SECOND = '再输一遍：'
ADDED = 'NAS 账号加好了。只回复「NAS 账号加好了」，不要发密码或账号文件的内容。'
NO_SUDO = 'NAS 上 maxsong 的 sudo 要密码，这个脚本做不了要 root 的那几步。什么都没改。'
USAGE = '用法：nas-account.command check | add | restart | unit'

# 替身。ssh：记下参数（以 NUL 分隔）和标准输入（不是终端才读），$LOG/fail 里的字样出现在参数里就失败；
# 登录那一次（-M）在 -S 给的路径上留一个文件，顶替真 ssh 建的控制口。
# stty：记一笔再照做。htpasswd：记下参数和标准输入，按 $LOG/htpasswd.says 决定给什么（没有这个文件就给形状对的一行）。
FAKES = r'''
ssh() {
  local n=1 want previous='' argument master='' socket=''
  while [[ -e "$LOG/$n.args" ]]; do n=$((n + 1)); done
  printf '%s\0' "$@" > "$LOG/$n.args"
  if [[ ! -t 0 ]]; then cat > "$LOG/$n.stdin"; fi
  if [[ -e "$LOG/fail" ]]; then
    want=$(< "$LOG/fail")
    if [[ "$*" == *"$want"* ]]; then return 1; fi
  fi
  for argument in "$@"; do
    if [[ "$argument" == -M ]]; then master=yes; fi
    if [[ "$previous" == -S ]]; then socket=$argument; fi
    previous=$argument
  done
  if [[ -n "$master" && -n "$socket" ]]; then : > "$socket"; fi
  return 0
}
stty() {
  printf '%s\n' "$*" >> "$LOG/stty"
  command stty "$@"
}
fake_htpasswd() {
  local say=''
  printf x >> "$LOG/htpasswd.calls"
  printf '%s\0' "$@" > "$LOG/htpasswd.args"
  cat > "$LOG/htpasswd.stdin"
  if [[ -e "$LOG/htpasswd.says" ]]; then say=$(< "$LOG/htpasswd.says"); fi
  case "$say" in
    fail) return 1 ;;
    nothing) return 0 ;;
    '') printf '%s\n\n' "$HASHLINE" ;;
    *) printf '%s\n' "$say" ;;
  esac
}
htpasswd=fake_htpasswd
'''
# 入口脚本被 source 时只给出函数：先 source 它，再换上替身，再照它自己的入口调 main。
DRIVER = 'source "$1"; source "$2"; shift 2; main "$@"'


def default_signals():
    """起入口脚本之前，把这三个信号的处置放回默认。bash 对「进来时就被忽略」的信号不认 trap，
    而这份用例是被 nohup 起的、还是被 shell 放在后台跑的（两样都会让子进程忽略其中几个），不该改变结果。"""
    for number in (signal.SIGINT, signal.SIGHUP, signal.SIGTERM):
        signal.signal(number, signal.SIG_DFL)


def named(calls, reach):
    """把记下的每一次 ssh 调用换成一个名字。参数或标准输入有一处和预期的写法不一样，就原样留着：断言时一眼看得出是哪里。
    返回（控制口的路径，名字的清单）。"""
    link, names = None, []
    for arguments, piped in calls:
        if link is None and arguments[:len(PIN) + 2] == PIN + ['-M', '-S'] and len(arguments) > len(PIN) + 2:
            link = arguments[len(PIN) + 2]
        shared = PIN + ['-T', '-S', str(link)] + reach
        control = PIN + ['-S', str(link), '-O']
        plain = {
            'login': (PIN + ['-M', '-S', str(link), '-o', 'ControlPersist=10m', '-f', '-N'] + reach, None),
            'sudo': (shared + ['sudo -n true'], None),
            'alive': (control + ['check'] + reach, None),
            'logout': (control + ['exit'] + reach, None),
            'check': (shared + [REMOTE + '- check'], HELPER),
            'restart': (shared + [REMOTE + '- restart'], HELPER),
        }
        name = [key for key, expected in plain.items() if (arguments, piped) == expected]
        for mode in ('append', 'unit'):
            sent = shared + [REMOTE + "-c '" + BOOT + "' " + mode + ' ' + str(len(HELPER))]
            if arguments == sent and piped is not None and piped.startswith(HELPER):
                # 帮手脚本后面跟着的内容：交给它的账号行，或者单元文件。
                name = [(mode, piped[len(HELPER):])]
        names.append(name[0] if name else (arguments, piped))
    return link, names


class Session:
    """入口脚本的一次运行。"""

    def __init__(self, case, process, master, slave, log, reach, says, plain):
        self.case, self.process, self.master, self.slave, self.log = case, process, master, slave, log
        self.reach, self.says, self.plain = reach, says, plain
        self.screen = b''
        self.seen = 0
        self.typed = []

    def echo(self):
        return bool(termios.tcgetattr(self.slave)[3] & termios.ECHO)

    def gather(self, wait):
        ready, _, _ = select.select([self.master], [], [], wait)
        if ready:
            self.screen += os.read(self.master, 65536)
        return bool(ready)

    def shown(self):
        return self.screen.decode('utf-8', 'replace').replace('\r\n', '\n')

    def expect(self, prompt):
        """等屏幕上出现这句提示，而且终端的回显已经关掉：这时候输入，终端自己才不会把它显示出来。"""
        wanted = prompt.encode()
        deadline = time.monotonic() + LIMIT
        while not (wanted in self.screen[self.seen:] and not self.echo()):
            self.gather(0.01)
            if self.process.poll() is not None or time.monotonic() > deadline:
                while self.gather(0.05):
                    pass
                self.case.fail('没等到「' + prompt + '」（或者回显没关）。退出码 ' + str(self.process.poll())
                               + '，屏幕上：\n' + self.shown())
        self.seen = len(self.screen)

    def asleep(self):
        """等 bash 真的睡在读输入上（进程状态 S）。提示出来、回显关掉之后，bash 还要走几步才进到读输入；信号落在这中间，
        bash 要等这次读输入返回才跑 trap。这是 bash 自己的做法：测试箱上绑在一个核里、提示一出来就发，3000 次里有 13 次，
        等到状态是 S 再发的 6000 次一次都没有。人按键落不到这几十微秒里，所以这里量的是「等输入时收到信号」。"""
        deadline = time.monotonic() + LIMIT
        while True:
            if self.process.poll() is not None or time.monotonic() > deadline:
                self.case.fail('bash 没有停在等输入上。退出码 ' + str(self.process.poll()) + '，屏幕上：\n' + self.shown())
            stat = Path('/proc', str(self.process.pid), 'stat').read_bytes()
            # 「进程号 (程序名) 状态 …」：程序名里可以有括号，状态是最后一个右括号后面的那一个字母。
            if stat[stat.rindex(b')') + 2:][:1] == b'S':
                return
            time.sleep(0.001)

    def type(self, line):
        self.typed.append(line)
        os.write(self.master, line.encode() + b'\n')

    def finish(self):
        """等它结束，把屏幕和替身记的账收上来。每一次运行都要守的几条在这里一并核。"""
        case = self.case
        deadline = time.monotonic() + LIMIT
        while self.process.poll() is None:
            self.gather(0.01)
            if time.monotonic() > deadline:
                self.process.kill()
                self.process.wait()
                case.fail('入口脚本没有结束。屏幕上：\n' + self.shown())
        while self.gather(0.05):
            pass
        calls, number = [], 1
        while os.path.exists(os.path.join(self.log, str(number) + '.args')):
            arguments = Path(self.log, str(number) + '.args').read_bytes().decode().split('\0')[:-1]
            piped = Path(self.log, str(number) + '.stdin')
            calls.append((arguments, piped.read_bytes() if piped.exists() else None))
            number += 1
        link, names = named(calls, self.reach)
        text = self.shown()
        files = {name: Path(self.log, name).read_bytes().decode() for name in self.plain if name != 'stdin'}
        htpasswd, used = None, Path(self.log, 'htpasswd.calls')
        if used.exists():
            case.assertEqual(used.read_text(), 'x', 'htpasswd 不止调了一次')
            htpasswd = (Path(self.log, 'htpasswd.args').read_bytes().decode().split('\0')[:-1],
                        Path(self.log, 'htpasswd.stdin').read_bytes())
        # 输过的密码、散列那一行：不上屏幕，不进任何一次 ssh 或 htpasswd 的命令行。
        secrets = [value for value in self.typed if value] + [HASH]
        secrets += [self.says] if self.says not in (None, 'fail', 'nothing') else []
        for secret in secrets:
            for shown in [text] + list(files.values()):
                case.assertNotIn(secret, shown)
            for arguments in [arguments for arguments, _ in calls] + ([htpasswd[0]] if htpasswd else []):
                for argument in arguments:
                    case.assertNotIn(secret, argument)
        # 登录过就一定收尾：临时目录（连同控制口）删掉，终端的回显开回来。
        if link is not None:
            case.assertRegex(link, r'\A/tmp/studiolo-nas\.[A-Za-z0-9]{8}/ssh\Z')
            case.assertFalse(os.path.lexists(os.path.dirname(link)), '临时目录没删：' + os.path.dirname(link))
            case.assertEqual(names.count('logout'), 1, names)
        case.assertEqual(names[-1:], ['logout'] if calls else [], names)
        restored = Path(self.log, 'stty')
        case.assertEqual(restored.read_text() if restored.exists() else '',
                         'echo\n' if calls and 'stdin' not in self.plain else '')
        case.assertTrue(self.echo(), '终端的回显没开回来')
        return types.SimpleNamespace(code=self.process.returncode, lines=text.splitlines(), names=names,
                                     htpasswd=htpasswd, files=files)


class Driving:
    """在伪终端里起入口脚本。"""

    def setUp(self):
        scratch = tempfile.TemporaryDirectory()
        self.addCleanup(scratch.cleanup)
        self.scratch = scratch.name
        self.fakes = os.path.join(self.scratch, 'fakes.sh')
        Path(self.fakes).write_text(FAKES, encoding='ascii')

    def start(self, *arguments, reach=None, failing=None, says=None, plain=(), direct=False):
        """reach：STUDIOLO_NAS_SSH 的值（不给就用脚本默认的连法）。failing：参数里带这段字样的那一次 ssh 失败。
        says：htpasswd 给出什么（'fail' 退出码 1，'nothing' 什么都不给，别的原样当输出）。
        plain：这几个流不接伪终端。direct：不经 source，照用户的用法把这个文件直接交给 bash（替身经 BASH_ENV 给进去）。"""
        log = tempfile.mkdtemp(dir=self.scratch)
        if failing is not None:
            Path(log, 'fail').write_text(failing, encoding='ascii')
        if says is not None:
            Path(log, 'htpasswd.says').write_text(says, encoding='ascii')
        environment = {'PATH': TOOLS, 'LOG': log, 'HASHLINE': LINE, 'TERM': 'dumb'}
        if reach is not None:
            environment['STUDIOLO_NAS_SSH'] = reach
        if direct:
            environment['BASH_ENV'] = self.fakes
            argv = [BASH, str(COMMAND)] + list(arguments)
        else:
            argv = [BASH, '-c', DRIVER, 'probe', str(COMMAND), self.fakes] + list(arguments)
        master, slave = pty.openpty()
        self.addCleanup(os.close, master)
        self.addCleanup(os.close, slave)
        streams, opened = {}, []
        for name in ('stdin', 'stdout', 'stderr'):
            if name not in plain:
                streams[name] = slave
            elif name == 'stdin':
                streams[name] = subprocess.DEVNULL
            else:
                streams[name] = open(os.path.join(log, name), 'wb')
                opened.append(streams[name])
        # 伪终端的另一头在这里一直开着：脚本结束之后还要看终端的回显是不是开回来了。
        process = subprocess.Popen(argv, env=environment, start_new_session=True, preexec_fn=default_signals, **streams)
        for stream in opened:
            stream.close()

        def reap():
            if process.poll() is None:
                process.kill()
            process.wait()
        self.addCleanup(reap)
        return Session(self, process, master, slave, log, NAS if reach is None else reach.split(), says, plain)


class CommandFlowTests(Driving, unittest.TestCase):
    """四个子命令各走哪几步、停在哪一步、屏幕上说了什么。"""

    greeting = GREETING.format(' '.join(NAS))
    sent = ('append', LINE.encode() + b'\n')

    def add(self, first=PASSWORD, second=None, **options):
        session = self.start('add', **options)
        session.expect(FIRST)
        session.type(first)
        session.expect(SECOND)
        session.type(first if second is None else second)
        return session.finish()

    def test_add_asks_twice_hashes_locally_and_sends_only_the_account_line(self):
        done = self.add()
        self.assertEqual(done.code, 0)
        self.assertEqual(done.lines, [self.greeting] + ASKING + [FIRST, SECOND, ADDED])
        # 登录一回 → 探 sudo → 问密码、本机算散列 → 确认连接还在 → 送那一行 → 重启 → 再看现状 → 退出登录。
        self.assertEqual(done.names, ['login', 'sudo', 'alive', self.sent, 'restart', 'check', 'logout'])
        self.assertEqual(done.htpasswd, (['-n', '-i', '-B', '-C', '12', nas.USER], PASSWORD.encode() + b'\n'))
        # 送过去的那一行，NAS 上的帮手脚本认。
        self.assertTrue(nas.LINE.fullmatch(self.sent[1]))
        # 退出登录那一步没成（主连接已经自己断了）不算失败。
        self.assertEqual(self.add(failing='-O exit').code, 0)

    def test_the_password_reaches_htpasswd_byte_for_byte_and_only_within_the_rule(self):
        # 以 -n 开头，带反斜杠、引号、美元符和通配符：一个字节都不能变。
        odd = '-n\\$a"b\'c*d?e[f]g{h}i~!#%&'
        # 整个密码长得像 echo 的选项（用 echo 往管道里送的话会被吞掉，送出去的是空的）。
        flags = '-' + 'neE' * 5
        for password in ('a' * 16, 'Z' * 72, odd, flags):
            with self.subTest(accepted=password):
                done = self.add(password)
                self.assertEqual((done.code, done.htpasswd[1], done.names[3]), (0, password.encode() + b'\n', self.sent))
        refused = ('a' * 15, 'a' * 73, 'fictional pass word 1', ' ' + 'a' * 16, 'a' * 16 + ' ', 'a' * 8 + '\t' + 'a' * 8,
                   'é' * 16, '')
        for password in refused:
            with self.subTest(refused=password):
                done = self.add(password)
                self.assertEqual((done.code, done.lines[-1], done.names, done.htpasswd),
                                 (1, '密码不合上面的要求。什么都没改。', ['login', 'sudo', 'logout'], None))

    def test_two_different_entries_change_nothing(self):
        pairs = ((PASSWORD, PASSWORD + 'x'), (PASSWORD, PASSWORD[:-1]), (PASSWORD, PASSWORD.upper()), (PASSWORD, ''),
                 ('', PASSWORD))
        for first, second in pairs:
            with self.subTest(first=first, second=second):
                done = self.add(first, second)
                self.assertEqual((done.code, done.lines[-1], done.names, done.htpasswd),
                                 (1, '两次输入不一样。什么都没改。', ['login', 'sudo', 'logout'], None))

    def test_add_stops_before_asking_when_it_cannot_log_in_or_cannot_sudo(self):
        for failing, said, reached in (('-M -S', [], ['login', 'logout']),
                                       ('sudo -n true', [NO_SUDO], ['login', 'sudo', 'logout'])):
            with self.subTest(failing=failing):
                done = self.start('add', failing=failing).finish()
                self.assertEqual((done.code, done.lines, done.names, done.htpasswd),
                                 (1, [self.greeting] + said, reached, None))

    def test_every_later_failure_stops_there_and_says_what_was_changed(self):
        nothing = 'htpasswd 给出的不是预期的那一行。什么都没改。'
        other = 'studiolo-backup:$2y$12$' + HASH
        before = ['login', 'sudo']
        cases = (
            # （哪一次 ssh 失败，htpasswd 给什么，屏幕上最后一句，ssh 走到了哪几步）
            (None, 'fail', 'htpasswd 没成功。什么都没改。', before),
            (None, 'nothing', nothing, before),
            (None, 'studiolo-backup-worker:plain', nothing, before),
            (None, 'studiolo-backup-worker:$2y$10$' + HASH, nothing, before),
            (None, 'studiolo-backup-worker:$2y$12$' + HASH[:-1], nothing, before),
            (None, 'studiolo-backup-worker:$2y$12$' + HASH + 'a', nothing, before),
            (None, other, nothing, before),
            # 两行：照收的话会一次加进两个账号。
            (None, LINE + '\n' + other, nothing, before),
            (None, other + '\n' + LINE, nothing, before),
            ('-O check', None, '登录 NAS 的那条连接已经断了（闲着超过 10 分钟会自己断）。什么都没改，重跑一遍。', before + ['alive']),
            # 账号行没送成：不重启。
            ("' append ", None, SECOND, before + ['alive', self.sent]),
            ('- restart', None, '账号已经加上了，但备份服务重启之后没起来。不要再跑 add（账号已经在，会被拒绝）：'
             '查明原因之后跑 nas-account.command restart', before + ['alive', self.sent, 'restart']),
            ('- check', None, SECOND, before + ['alive', self.sent, 'restart', 'check']),
        )
        for failing, says, last, reached in cases:
            with self.subTest(failing=failing, says=says):
                done = self.add(failing=failing, says=says)
                self.assertEqual((done.code, done.lines[-1], done.names), (1, last, reached + ['logout']))
                self.assertNotIn(ADDED, done.lines)
        # htpasswd 给的是另外两种 bcrypt 前缀：帮手脚本认，照送。
        for prefix in ('$2a$', '$2b$'):
            line = 'studiolo-backup-worker:' + prefix + '12$' + HASH[::-1]
            with self.subTest(prefix=prefix):
                done = self.add(says=line)
                self.assertEqual((done.code, done.names[3]), (0, ('append', line.encode() + b'\n')))

    def test_a_signal_at_either_prompt_logs_out_cleans_up_and_turns_the_echo_back_on(self):
        for number, code in ((signal.SIGINT, 130), (signal.SIGHUP, 129), (signal.SIGTERM, 143)):
            for stage in (FIRST, SECOND):
                with self.subTest(signal=number.name, stage=stage):
                    session = self.start('add')
                    session.expect(FIRST)
                    if stage is SECOND:
                        session.type(PASSWORD)
                        session.expect(SECOND)
                    self.assertFalse(session.echo())
                    session.asleep()
                    session.process.send_signal(number)
                    # 收尾（退出登录、删临时目录、把回显开回来）由 finish 里的那几条核。
                    done = session.finish()
                    self.assertEqual((done.code, done.names, done.htpasswd), (code, ['login', 'sudo', 'logout'], None))

    def test_add_needs_a_real_terminal_on_all_three_streams(self):
        for stream in ('stdin', 'stdout', 'stderr'):
            with self.subTest(stream=stream):
                done = self.start('add', plain=(stream,)).finish()
                said = done.files['stdout'].splitlines() if stream == 'stdout' else done.lines
                self.assertEqual((done.code, said, done.names, done.htpasswd), (1, ['需要用户自己的终端'], [], None))

    def test_check_restart_and_unit_run_their_steps_in_order_and_stop_at_the_first_failure(self):
        self.assertEqual(hashlib.sha256(UNIT).hexdigest(), nas.UNIT_SHA256)
        flows = (
            ('check', ['check'], []),
            ('restart', ['restart', 'check'], ['备份服务起来了。']),
            # 换单元：帮手脚本后面跟着的正好是仓库里这一版单元文件。
            ('unit', [('unit', UNIT), 'check'],
             ['服务单元换好了。下一次重启 NAS 之后再跑一遍 check：「现在」应该是 active，端口有人监听。']),
        )
        fails = {'check': '- check', 'restart': '- restart', ('unit', UNIT): "' unit "}
        for subcommand, steps, said in flows:
            with self.subTest(subcommand=subcommand):
                done = self.start(subcommand).finish()
                self.assertEqual((done.code, done.lines, done.names, done.htpasswd),
                                 (0, [self.greeting] + said, ['login', 'sudo'] + steps + ['logout'], None))
            for position, step in enumerate(steps):
                with self.subTest(subcommand=subcommand, failing=fails[step]):
                    done = self.start(subcommand, failing=fails[step]).finish()
                    self.assertEqual((done.code, done.lines, done.names),
                                     (1, [self.greeting], ['login', 'sudo'] + steps[:position + 1] + ['logout']))
            for failing, last, reached in (('-M -S', [], ['login']), ('sudo -n true', [NO_SUDO], ['login', 'sudo'])):
                with self.subTest(subcommand=subcommand, failing=failing):
                    done = self.start(subcommand, failing=failing).finish()
                    self.assertEqual((done.code, done.lines, done.names),
                                     (1, [self.greeting] + last, reached + ['logout']))

    def test_the_way_to_reach_the_nas_can_be_overridden_and_is_used_for_every_call(self):
        direct = 'maxsong@192.168.100.111'
        done = self.start('check', reach=direct).finish()
        # 名字都认得出，就是每一次调用末尾的连法都是给的这一个（named 是按它对的）。
        self.assertEqual((done.code, done.lines, done.names),
                         (0, [GREETING.format(direct)], ['login', 'sudo', 'check', 'logout']))
        done = self.add(reach='-p 2222 ' + direct)
        self.assertEqual((done.code, done.lines[0], done.names),
                         (0, GREETING.format('-p 2222 ' + direct),
                          ['login', 'sudo', 'alive', self.sent, 'restart', 'check', 'logout']))

    def test_run_as_a_file_it_dispatches_the_same_way_and_unknown_words_touch_nothing(self):
        for arguments in ([], ['bogus'], ['--apply'], ['ADD'], ['']):
            with self.subTest(arguments=arguments):
                done = self.start(*arguments, direct=True).finish()
                self.assertEqual((done.code, done.lines, done.names, done.htpasswd), (2, [USAGE], [], None))
        done = self.start('check', direct=True).finish()
        self.assertEqual((done.code, done.lines, done.names),
                         (0, [self.greeting], ['login', 'sudo', 'check', 'logout']))
        # 照用户的用法跑，登录没成也是马上停（不接着去探 sudo）。
        done = self.start('check', direct=True, failing='-M -S').finish()
        self.assertEqual((done.code, done.lines, done.names), (1, [self.greeting], ['login', 'logout']))


class CommandRuleTests(unittest.TestCase):
    """入口脚本里的两条规则和它交给 NAS 的那段引导代码，各自单独核。"""

    def verdicts(self, function, values):
        """一个 bash 进程里把这些值挨个交给入口脚本里的这个函数；返回每个值放过没有。"""
        loop = 'source "$1"; check=$2; shift 2; for value in "$@"; do if "$check" "$value"; then printf 1; else printf 0; fi; done'
        done = subprocess.run([BASH, '-c', loop, 'probe', str(COMMAND), function] + list(values), env={'PATH': TOOLS},
                              stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        self.assertEqual((done.returncode, done.stderr, len(done.stdout)), (0, b'', len(values)))
        return [flag == '1' for flag in done.stdout.decode()]

    def test_the_password_rule_is_sixteen_to_seventy_two_printable_ascii_without_spaces(self):
        every = ''.join(chr(code) for code in range(0x21, 0x7f))
        accepted = ['a' * 16, 'a' * 72, every[:47], every[47:] + 'a' * 16, '-n' + 'a' * 14, '-' + 'neE' * 5, PASSWORD]
        refused = ['', 'a' * 15, 'a' * 73, 'a' * 8 + ' ' + 'a' * 8, ' ' + 'a' * 16, 'a' * 16 + ' ', 'a' * 8 + '\t' + 'a' * 8,
                   'a' * 16 + '\n', 'a' * 8 + '\n' + 'a' * 8, 'a' * 16 + '\x7f', 'a' * 16 + '\x1b', 'é' * 16, 'a' * 15 + 'é',
                   '密码' * 8]
        self.assertEqual(self.verdicts('valid_password', accepted + refused), [True] * len(accepted) + [False] * len(refused))
        self.assertEqual(len(every), 94)
        for value in accepted:
            # 这里放过的，169 那边读凭据时也认（credentials.py 和 worker 同一条规则）；bcrypt 只用前 72 个字节。
            self.assertEqual(credentials.validate_secret(value, 1, '账号密码'), value)
            self.assertLessEqual(len(value.encode()), 72)

    def test_the_account_line_rule_agrees_with_the_helper_on_the_nas(self):
        accepted = [LINE, LINE.replace('$2y$', '$2a$'), LINE.replace('$2y$', '$2b$'),
                    'studiolo-backup-worker:$2y$12$' + './' + 'A' * 51]
        refused = ['', LINE.replace('$2y$', '$2x$'), LINE.replace('$12$', '$10$'), LINE.replace('$12$', '$13$'), LINE[:-1],
                   LINE + 'a', 'x' + LINE, LINE + ' ', ' ' + LINE, LINE.replace('-worker', ''), LINE.replace(':', '::'),
                   LINE[:-1] + '!', LINE[:-1] + '=', LINE + '\n' + LINE, LINE + '\nstudiolo-backup:$2y$12$' + HASH,
                   'studiolo-backup-worker:' + HASH]
        got = self.verdicts('account_line', accepted + refused)
        self.assertEqual(got, [True] * len(accepted) + [False] * len(refused))
        # NAS 上的帮手脚本对「这一行加一个换行」的判定，和这里对每一个值都一样。
        self.assertEqual(got, [bool(nas.LINE.fullmatch(value.encode() + b'\n')) for value in accepted + refused])

    def test_the_remote_side_runs_exactly_the_helper_bytes_and_leaves_the_rest_on_stdin(self):
        self.assertNotEqual(os.geteuid(), 0, '这条用例不以 root 跑')
        # 引导代码夹在单引号里交给 NAS 上的 shell：它自己不能带单引号。
        self.assertNotIn("'", BOOT)
        remote = REMOTE + "-c '" + BOOT + "' append " + str(len(HELPER))
        self.assertEqual(shlex.split(remote),
                         ['sudo', '-n', 'python3', '-I', '-B', '-c', BOOT, 'append', str(len(HELPER))])
        rest = LINE.encode() + b'\n'

        def boot(script):
            return subprocess.run([sys.executable, '-I', '-B', '-c', BOOT, 'append', str(len(script))],
                                  input=script + rest, env={'PATH': TOOLS, 'LC_ALL': 'C.UTF-8'},
                                  stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        # 用一段会报告自己的小脚本顶替帮手脚本：它被完整执行（长度按字节数，里面有非 ASCII 的字），
        # 参数里只剩模式，后面跟着的内容原样留在标准输入里。
        probe = ('# 按字节数，不是按字符数\nimport sys\n'
                 'sys.stdout.buffer.write(repr((__name__, sys.argv[1:])).encode() + b"|" + sys.stdin.buffer.read())\n').encode()
        self.assertGreater(len(probe), len(probe.decode()))
        done = boot(probe)
        self.assertEqual((done.returncode, done.stdout, done.stderr), (0, b"('__main__', ['append'])|" + rest, b''))
        # 真的帮手脚本经这条路也跑得起来：这里不是 root，它在读账号行之前就照自己的规矩拒绝。
        self.assertGreater(len(HELPER), len(HELPER.decode()))
        done = boot(HELPER)
        self.assertEqual((done.returncode, done.stdout, done.stderr.decode()),
                         (1, b'', 'nas_account=failed：用法见文件开头；要经 sudo 跑\n'))

    def test_the_script_is_valid_shell_and_has_no_way_around_its_own_rules(self):
        checked = subprocess.run([BASH, '-n', str(COMMAND)], stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        self.assertEqual((checked.returncode, checked.stdout), (0, b''))
        # 每一次 ssh 都先带上钉住主机钥匙的那几项。
        self.assertEqual(re.findall(r'(?:^|[\s|;({])ssh\s+(\S+)', CODE), ['"${pin[@]}"'] * 6)
        # 密码只经管道到 htpasswd（-i 从标准输入读、-n 不写文件），而且只这一处。
        self.assertEqual(CODE.count('"$htpasswd"'), 1)
        self.assertIn('line=$(printf \'%s\\n\' "$password" | "$htpasswd" -n -i -B -C 12 "$account")', CODE)
        for banned in ('set -x', 'xtrace', 'StrictHostKeyChecking=no', 'accept-new', 'BatchMode',
                       'UserKnownHostsFile=/dev/null', ' -b ', '-tt', 'eval '):
            self.assertNotIn(banned, CODE)


class HostKeyTests(unittest.TestCase):
    """NAS 的主机钥匙只认仓库里钉住的那一把。"""

    def tool(self, name):
        found = shutil.which(name, path=TOOLS)
        self.assertIsNotNone(found, '这台机器上没有 ' + name)
        return found

    def test_the_pinned_key_is_one_ed25519_key_with_the_documented_fingerprint(self):
        text = KNOWN_HOSTS.read_text(encoding='utf-8')
        keys = [line for line in text.splitlines() if line and not line.startswith('#')]
        self.assertEqual(len(keys), 1)
        host, kind, blob = keys[0].split(' ')
        self.assertEqual((host, kind), ('studiolo-nas', 'ssh-ed25519'))
        raw = base64.b64decode(blob, validate=True)
        self.assertEqual((raw[:19], len(raw)), (b'\x00\x00\x00\x0bssh-ed25519\x00\x00\x00\x20', 51))
        self.assertEqual('SHA256:' + base64.b64encode(hashlib.sha256(raw).digest()).decode().rstrip('='), FINGERPRINT)
        # 写着指纹的两处（文件头注释、README）都是这一把，没有别的指纹留着。
        for written in (text, README.read_text(encoding='utf-8')):
            self.assertEqual(set(re.findall(r'SHA256:[A-Za-z0-9+/]{43}', written)), {FINGERPRINT})
        # ssh 自己的工具读这个文件，认到的也是这一把；按别名找得到，按 NAS 的地址找不到（所以脚本要带 HostKeyAlias）。
        keygen = self.tool('ssh-keygen')
        listed = subprocess.run([keygen, '-l', '-f', str(KNOWN_HOSTS)], stdin=subprocess.DEVNULL,
                                stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        self.assertEqual((listed.returncode, listed.stdout.decode().split()),
                         (0, ['256', FINGERPRINT, 'studiolo-nas', '(ED25519)']), listed.stderr.decode())
        for host, code in (('studiolo-nas', 0), ('192.168.100.111', 1)):
            found = subprocess.run([keygen, '-F', host, '-f', str(KNOWN_HOSTS)], stdin=subprocess.DEVNULL,
                                   stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            self.assertEqual(found.returncode, code, found.stderr.decode())
            self.assertEqual(keys[0] in found.stdout.decode(), code == 0)

    def test_ssh_takes_the_pinned_options_over_whatever_its_own_configuration_says(self):
        """这台机器自己的 ssh 配置写得再松，命令行上钉的那几项照样作数（用 ssh -G 看它实际会用的设置，不连任何机器）。"""
        with tempfile.TemporaryDirectory() as scratch:
            loose = os.path.join(scratch, 'config')
            Path(loose).write_text('Host *\n  StrictHostKeyChecking no\n  UserKnownHostsFile /dev/null\n'
                                   '  GlobalKnownHostsFile /etc/ssh/ssh_known_hosts\n  HostKeyAlias somebody-else\n'
                                   '  UpdateHostKeys yes\n', encoding='ascii')

            def settings(options):
                done = subprocess.run([self.tool('ssh'), '-G', '-F', loose] + options + NAS, stdin=subprocess.DEVNULL,
                                      stdout=subprocess.PIPE, stderr=subprocess.PIPE)
                self.assertEqual(done.returncode, 0, done.stderr.decode())
                found = dict(line.split(' ', 1) for line in done.stdout.decode().splitlines() if ' ' in line)
                return [found.get(key) for key in ('stricthostkeychecking', 'userknownhostsfile', 'globalknownhostsfile',
                                                   'hostkeyalias', 'updatehostkeys', 'hostname', 'user', 'proxyjump')]
            where = ['192.168.100.111', 'maxsong', 'studiolo-108-ts']
            strict, *pinned = settings(PIN)
            self.assertIn(strict, ('true', 'yes'))
            self.assertEqual(pinned[:3], [str(KNOWN_HOSTS), '/dev/null', 'studiolo-nas'])
            self.assertIn(pinned[3], ('false', 'no'))
            self.assertEqual(pinned[4:], where)
            # 对照：不带那几项，这份松的配置就真的生效（说明上面看的确实是会被配置改掉的设置）。
            strict, *loosened = settings([])
            self.assertIn(strict, ('false', 'no', 'off'))
            self.assertEqual(loosened[:3], ['/dev/null', '/etc/ssh/ssh_known_hosts', 'somebody-else'])
            self.assertIn(loosened[3], ('true', 'yes'))
            self.assertEqual(loosened[4:], where)


if __name__ == '__main__':
    unittest.main()
