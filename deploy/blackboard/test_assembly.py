"""黑板形状发行（当前 r27→r28）的固定变更面；夹具不接触生产或凭据。"""
import ast
import collections
import contextlib
import hashlib
import importlib.util
import os
from pathlib import Path
import re
import socket
import tempfile
import types
import unittest
import assembly as a

ROOT = Path(__file__).resolve().parents[2]


def activated(name):
    """169 上现装的原文（上一发行激活写下的），逐字节取回放在 activated/，摘要钉在装配里。"""
    return (Path(__file__).resolve().parent / 'activated' / name).read_bytes().decode()


class RootedOs:
    """给启动器用的 os：绝对路径都落到假根目录里，相对路径（带目录描述符）照走真的系统调用。
    只放行 preflight 只看元数据用得到的几样，别的（打开来写、删、改名、建……）一律算错：启动器什么都不改。"""

    def __init__(self, root):
        self.root, self.opened = root, set()
        # 这些名字的节点按「别人的」「看不了的」报：跑用例的账号改不了属主，要是 root 又什么都看得了。
        self.foreign, self.denied = set(), set()
        self.path = types.SimpleNamespace(lexists=lambda path: os.path.lexists(self.at(path)))

    def __getattr__(self, name):
        if name in ('fstat', 'geteuid') or name.startswith('O_'):
            return getattr(os, name)
        raise AssertionError('preflight must not use os.' + name)

    def at(self, path):
        return self.root + path if path.startswith('/') else path

    def open(self, path, flags, *, dir_fd=None):
        if flags & (os.O_WRONLY | os.O_RDWR | os.O_CREAT | os.O_TRUNC | os.O_APPEND):
            raise AssertionError('preflight must not open for writing')
        descriptor = os.open(self.at(path), flags, dir_fd=dir_fd)
        self.opened.add(descriptor)
        return descriptor

    def close(self, descriptor):
        os.close(descriptor)
        self.opened.remove(descriptor)

    def stat(self, path, *, dir_fd=None, follow_symlinks=True):
        if path in self.denied:
            raise PermissionError(path)
        fields = list(os.stat(self.at(path), dir_fd=dir_fd, follow_symlinks=follow_symlinks))
        fields[4] += path in self.foreign
        return os.stat_result(fields)


def dead_socket(path):
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as listener:
        listener.bind(str(path))
        listener.listen()


class Installed:
    """假根目录里照 169 的样子摆好启动器要看的全部目录和文件（路径取启动器自己的常量，权限和现场一样），
    启动器整份用真的字节，只把它的 os 换成 RootedOs：preflight 的每一步都真的走一遍。"""

    def __init__(self, root, text):
        self.launcher = {'__name__': 'launcher_under_test'}
        exec(compile(text, 'launch-human.py', 'exec'), self.launcher)
        self.os = self.launcher['os'] = RootedOs(root)

        def at(name):
            return Path(self.os.at(self.launcher[name]))
        for directory in (at('AUTH'), at('AUDIT'), at('POLICY'), at('VAULT') / '.workbench', at('WRITER'), at('BOARD'),
                          at('AGENT'), at('CONFIG').parent, at('BINARY').parent, at('RELEASE') / 'web',
                          at('SOCKET').parent, at('BOARD_SOCKET').parent):
            directory.mkdir(parents=True)
        for path in (at('CONFIG'), at('AUTH') / 'auth.sqlite3', at('AUDIT') / 'audit.sqlite3', at('POLICY') / 'policy.sqlite3',
                     at('AGENT') / 'agent.sqlite3', at('VAULT') / '.workbench/broker-binding.json',
                     at('WRITER') / 'broker-binding.json', at('BINARY')):
            path.touch()
            path.chmod(0o755 if path == at('BINARY') else 0o600)
        # 人侧在跑时的三个入口；备份那个启动器不看，摆上是为了和被强杀之后的现场一样。
        self.leftovers = [at('SOCKET'), at('BOARD_SOCKET'), Path(root) / 'run/studiolo-backup/producer.sock']
        self.leftovers[2].parent.mkdir()

    def preflight(self):
        try:
            self.launcher['preflight']()
        finally:
            # 放行还是拒绝，打开过的都关了：后面换成产品程序时不带着走。
            assert not self.os.opened, self.os.opened


@contextlib.contextmanager
def installed(text):
    mask = os.umask(0o077)
    try:
        with tempfile.TemporaryDirectory(prefix='stu-l-', dir='/tmp') as root:
            yield Installed(root, text)
    finally:
        os.umask(mask)


def rust_seconds(relative, name):
    """从 Rust 源码读一个 `const NAME: Duration = Duration::from_secs(N);`，读不到或不止一处就失败。"""
    found = re.findall(r'^const ' + name + r': Duration = Duration::from_secs\((\d+)\);$',
                       (ROOT / relative).read_text(), re.MULTILINE)
    assert len(found) == 1, (relative, name, found)
    return int(found[0])


METHOD_HEAD = re.compile(r'^    (?:pub(?:\([a-z]+\))? )?(?:(?:async|const|unsafe) )*fn (\w+)\b', re.MULTILINE)


def rust_code(path):
    """一份 Rust 源码的非测试部分，去掉行注释（注释里提到的调用不算数）。
    只切掉写在文件里的测试模块（`#[cfg(test)]` 后面跟着 `mod 名字 {`）；`mod tests;` 这种声明不切，后面的代码照样看。"""
    return re.sub(r'//[^\n]*', '', re.split(r'\n#\[cfg\(test\)\]\nmod \w+ \{', path.read_text(), maxsplit=1)[0])


def rust_calls(paths, receiver, method=r'\w+'):
    """这些源码里 `<receiver>.<method>(` 写在哪些方法里：[(所在方法, 被调的名字), …]，按出现顺序。
    receiver 给 None 就不限接收者。点号前后允许换行和空白（rustfmt 会把长链拆成几行）。
    只认 impl 块里缩进四格的方法；写在方法以外就失败（那是这个助手不认识的写法，要人来看）。"""
    chain = '' if receiver is None else r'\b' + r'\s*\.\s*'.join(re.escape(part) for part in receiver.split('.')) + r'\s*'
    pattern = re.compile(chain + r'\.\s*(' + method + r')\s*\(')
    found = []
    for path in paths:
        text = rust_code(path)
        heads = [(match.start(), match.group(1)) for match in METHOD_HEAD.finditer(text)]
        ends = [match.start() for match in re.finditer(r'^}', text, re.MULTILINE)]
        for call in pattern.finditer(text):
            inside = [(at, name) for at, name in heads if at < call.start()]
            assert inside and not any(inside[-1][0] < end < call.start() for end in ends), (str(path), call.group(0))
            found.append((inside[-1][1], call.group(1)))
    return found


class AssemblyTests(unittest.TestCase):
    def test_r28_upgrades_the_activated_r27(self):
        self.assertEqual(a.BUILD, 'm5m6-board-20260920-r28')
        self.assertEqual(a.RELEASE.rsplit('/', 1)[1], a.BUILD)
        self.assertTrue(a.OLD_RELEASE.endswith('/m5m6-board-20260920-r27'))
        self.assertEqual(a.WEB_PREFIX, 'web/')

    def test_fixtures_are_the_bytes_installed_on_169(self):
        """BUG-058：以前的夹具拿仓库里更早的模板拼，少了线上那几行。夹具就是现场原文，换发行时忘了换这里就红。"""
        for name, pinned in (('launch-human.py', a.LAUNCHER_SHA), ('studiolo-human.service', a.HUMAN_SHA)):
            self.assertEqual(hashlib.sha256(activated(name).encode()).hexdigest(), pinned, name)

    def test_launcher_preserves_the_approved_bridge_wiring(self):
        old = activated('launch-human.py')
        new = a.launcher(old)
        backup = ' "--backup-config", "' + a.BACKUP_CONFIG + '",'
        # 本版只换发行路径：换回去就是上一发行的那一份，一个字不差。发行路径恰好一处指到新发行
        # （只看还原后相等的话，忘了换路径的变换也能过）。
        self.assertEqual(new.replace(a.RELEASE, a.OLD_RELEASE), old)
        self.assertEqual(new.count(a.RELEASE), 1)
        self.assertNotIn(a.OLD_RELEASE, new)
        # 看见残留就拒绝的写法一处不剩（r27 起）。
        self.assertNotIn('exists(', new)
        self.assertEqual(new.count('"--backup-config"'), 1)
        self.assertEqual(new.count('"--agent-data"'), 1)
        self.assertEqual(new.count('"--sync-transport"'), 1)
        # 备份配置紧跟在同步审批后面，路径是写死的那一份，和主人配置同目录。
        self.assertIn('"--sync-transport", "' + a.SYNC_APPROVAL + '",' + backup, new)
        self.assertEqual(Path(a.BACKUP_CONFIG).parent, Path(a.SYNC_APPROVAL).parent)

        # 语法树上只有模块级的发行路径那一句不同，函数（含 preflight、inspect_leftover、main）一个不变。
        before, after = ([ast.dump(node) for node in ast.parse(source).body] for source in (old, new))
        release = [ast.dump(ast.parse('RELEASE = ' + repr(path)).body[0]) for path in (a.OLD_RELEASE, a.RELEASE)]
        self.assertEqual([[x, y] for x, y in zip(before, after, strict=True) if x != y], [release])
        self.assertIn('"--canonical-writer-data", WRITER', new)
        self.assertIn('"STUDIOLO_TOTP_SEED_HEX": credential()', new)
        # 已转换的启动器不能再过一遍；接线或标签漂移也拒绝；还没带只读身份目录参数的（r15 形状）、
        # 还没带备份配置参数的（r16 形状）、备份配置指到别处或多出一处的也拒绝。
        for bad in (new, old + '\n# "--backup-config"\n', old + '\n# "--agent-data"\n',
                    old.replace(backup, ''), old.replace(a.BACKUP_CONFIG, '/foreign/backup.json'),
                    old.replace(backup, '').replace('"--board-socket", BOARD_SOCKET,',
                                                    '"--board-socket", BOARD_SOCKET,' + backup),
                    old.replace(' "--agent-data", AGENT,', ''),
                    old.replace('"--board-socket", BOARD_SOCKET,', ''),
                    old.replace('"--sync-transport"', '"--foreign"'),
                    # 两个残留入口的核对少一处、多一处、被改过，或者又写回「见到就拒绝」，都不是上一发行的那一份。
                    old.replace(a.LEFTOVER_CHECKS, ''), old + '\n\ndef extra():\n' + a.LEFTOVER_CHECKS,
                    old.replace('(BOARD_SOCKET, SOCKET):', '(SOCKET,):'),
                    old.replace(a.INSPECT_LEFTOVER, ''), old.replace(' and node.st_uid == os.geteuid()', ''),
                    old.replace('            and node.st_nlink == 1)', '            )'),
                    old.replace('def preflight():', 'def preflight(strict=True):'),
                    old.replace(a.LEFTOVER_CHECKS, a.LEFTOVER_CHECKS + '    inspect_leftover(BOARD_SOCKET)\n'),
                    old.replace(a.LEFTOVER_CHECKS,
                                a.LEFTOVER_CHECKS + '    require(not os.path.lexists(BOARD_SOCKET))\n'),
                    # 核对一个字没改，但挪出了 preflight：启动前就没有谁看残留了。
                    old.replace(a.LEFTOVER_CHECKS, '').replace('def main():\n', 'def main():\n' + a.LEFTOVER_CHECKS),
                    # 两个入口的常量指到别处：核对会看错节点。
                    old.replace(a.HUMAN_SOCKET, '/foreign/http.sock'),
                    old.replace(a.BOARD_SOCKET, '/foreign/board.sock'),
                    old.replace(a.SYNC_APPROVAL, '/foreign/approval.json'),
                    old + '\n# "--sync-transport"\n',
                    # 别处还提到上一发行的路径：只换了一处，旧路径会留下。
                    old + '\n# ' + a.OLD_RELEASE + '\n',
                    old.replace('正式黑板库 · Obsidian 同步试跑', '未知标签')):
            with self.assertRaises(ValueError):
                a.launcher(bad)

    def test_launcher_lets_the_human_side_come_back_after_a_kill(self):
        """BUG-058：被强杀之后三个死 socket 都在，整份 preflight 照样放行，一样不动。"""
        with installed(a.launcher(activated('launch-human.py'))) as site:
            # 正常停下之后（一个残留都没有）照常起。
            site.preflight()
            for path in site.leftovers:
                dead_socket(path)
            before = [path.lstat() for path in site.leftovers]
            site.preflight()
            self.assertEqual([path.lstat() for path in site.leftovers], before)

    def test_launcher_hands_only_our_own_single_name_sockets_to_the_product(self):
        """T-231、BUG-058：两个残留入口同样严——不是自己的、不止一个名字的、不是 socket 的一律拒绝，原样留着。"""
        for index in (0, 1):
            with installed(a.launcher(activated('launch-human.py'))) as site:
                node = site.leftovers[index]
                other = node.parent / 'other'

                def refused():
                    with self.assertRaises(RuntimeError):
                        site.preflight()
                dead_socket(node)
                site.preflight()
                site.os.foreign.add(node.name)
                refused()
                site.os.foreign.clear()
                # 看不了不等于不在。
                site.os.denied.add(node.name)
                with self.assertRaises(PermissionError):
                    site.preflight()
                site.os.denied.clear()
                site.preflight()
                os.link(node, other)
                refused()
                self.assertTrue(node.is_socket())
                node.unlink()
                # 另一个名字还在、入口这个名字不在：不是残留。
                site.preflight()
                # 链接（指着一个合格的 socket 也不行；指着不存在的也不行）、普通文件、目录、管道。
                for make, remove in ((lambda: node.symlink_to(other), node.unlink),
                                     (lambda: node.symlink_to('missing'), node.unlink), (node.touch, node.unlink),
                                     (node.mkdir, node.rmdir), (lambda: os.mkfifo(node), node.unlink)):
                    make()
                    refused()
                    remove()
                    site.preflight()
                # 入口所在的目录别人进得来、不在了：照旧拒绝。
                node.parent.chmod(0o750)
                refused()
                node.parent.chmod(0o700)
                site.preflight()
                other.unlink()
                node.parent.rmdir()
                with self.assertRaises(FileNotFoundError):
                    site.preflight()

    def test_launcher_still_refuses_missing_or_open_stores(self):
        """摆好的现场少一样、权限放宽一样都拒绝：证明上面两条跑的是整份 preflight，不是空的。"""
        text = a.launcher(activated('launch-human.py'))
        files = ('CONFIG', ('AUTH', 'auth.sqlite3'), ('AUDIT', 'audit.sqlite3'), ('POLICY', 'policy.sqlite3'),
                 ('AGENT', 'agent.sqlite3'), ('VAULT', '.workbench/broker-binding.json'), ('WRITER', 'broker-binding.json'),
                 'BINARY')
        for name in files:
            with self.subTest(name=name), installed(text) as site:
                base, rest = name if isinstance(name, tuple) else (name, '')
                path = Path(site.os.at(site.launcher[base])) / rest
                site.preflight()
                path.chmod(0o640 if base != 'BINARY' else 0o775)
                with self.assertRaises(RuntimeError):
                    site.preflight()
                path.unlink()
                with self.assertRaises(FileNotFoundError):
                    site.preflight()
        for base in ('AUTH', 'AUDIT', 'POLICY', 'VAULT', 'WRITER', 'BOARD', 'AGENT'):
            with self.subTest(base=base), installed(text) as site:
                Path(site.os.at(site.launcher[base])).chmod(0o750)
                with self.assertRaises(RuntimeError):
                    site.preflight()

    def test_human_unit_only_repoints_the_release(self):
        old = activated('studiolo-human.service')
        new_stop = a.stop_timeout_line(a.STOP_TIMEOUT_SECONDS)
        self.assertEqual(old.count(new_stop), 1)
        new = a.human_unit(old)
        added = ''.join('ReadWritePaths=' + path + '\n' for path in (a.BACKUP_RUNTIME, a.BACKUP_STAGING))
        # 本版只换发行路径：换回去就是上一发行的那一份，一个字不差。
        self.assertEqual(new.replace(a.RELEASE, a.OLD_RELEASE), old)
        # 内存上限原样沿用 r24 的 2G：恰好一行、落在 [Service] 节里。
        self.assertEqual(a.MEMORY_LINE, 'MemoryMax=2G\n')
        self.assertEqual(new.count('MemoryMax='), 1)
        self.assertEqual(new.count('\n' + a.MEMORY_LINE), 1)
        self.assertLess(new.index('[Service]'), new.index(a.MEMORY_LINE))
        self.assertLess(new.index(a.MEMORY_LINE), new.index('[Install]'))
        # 发行路径恰好两处（启动器、工作目录）都指到新发行，旧发行名一处不剩：只看「还原后相等」的话，
        # 忘了换路径的变换也能过（还原是空操作）。
        self.assertEqual(new.count(a.RELEASE), 2)
        self.assertNotIn(a.OLD_RELEASE, new)
        self.assertEqual(new.count(added), 1)
        # 新写根要落在 [Service] 节里，写到 [Install] 节 systemd 不认。
        self.assertLess(new.index(added), new.index('[Install]'))
        self.assertIn('LoadCredentialEncrypted=totp-next:', new)
        self.assertIn('RuntimeDirectory=studiolo-human studiolo-board\n', new)
        # 备份的两个写根紧跟在 Bridge 传输根后面，路径和 worker 部署模板里 tmpfiles 建的两个目录是同一组。
        self.assertIn('ReadWritePaths=' + a.TRANSPORT_ROOT + '\n' + added, new)
        tmpfiles = (ROOT / 'deploy/backup-worker/systemd/studiolo-backup.tmpfiles.conf').read_text()
        for path in (a.BACKUP_RUNTIME, a.BACKUP_STAGING):
            self.assertIn('d ' + path + ' 0750 ', tmpfiles)
        self.assertEqual(new.count('TimeoutStopSec='), 1)
        self.assertEqual(new.count(new_stop), 1)
        self.assertEqual(new_stop, 'TimeoutStopSec=300\n')
        # 拒绝：已转换过的；停止超时不是算出来的那个值（含还没接备份的 90 秒）、多出一行（不管值是多少）、没有；
        # 备份的写根缺一个、多一处、不在 Bridge 传输根后面、别处又提到备份路径；黑板或 Bridge 接线缺了；发行路径不是两处；
        # 内存上限不是恰好一行 2G（退回 512M、别的值、没有、多一行、写在注释里、前面带别的字）。
        for bad in (new, old.replace(new_stop, 'TimeoutStopSec=120\n'), old.replace(new_stop, 'TimeoutStopSec=90\n'),
                    old + new_stop, old + 'TimeoutStopSec=120\n', old.replace(new_stop, ''),
                    old.replace(added, ''), old.replace('ReadWritePaths=' + a.BACKUP_STAGING + '\n', ''),
                    old.replace(added, '').replace('ReadWritePaths=' + a.BOARD + '\n',
                                                   'ReadWritePaths=' + a.BOARD + '\n' + added),
                    old + 'ReadWritePaths=' + a.BACKUP_RUNTIME + '\n', old + '# studiolo-backup\n',
                    old.replace('ReadWritePaths=' + a.BOARD + '\n', ''),
                    old.replace('ReadWritePaths=' + a.BRIDGE_ROOT + '\n', ''),
                    old.replace('ReadWritePaths=' + a.TRANSPORT_ROOT + '\n', ''),
                    old.replace(a.OLD_RELEASE + '/launch-human.py', '/foreign/launch.py'),
                    old.replace('RuntimeDirectory=studiolo-human studiolo-board\n',
                                'RuntimeDirectory=studiolo-human\n'),
                    old.replace(a.MEMORY_LINE, 'MemoryMax=512M\n'), old.replace(a.MEMORY_LINE, 'MemoryMax=1G\n'),
                    old.replace(a.MEMORY_LINE, ''), old + a.MEMORY_LINE, old + 'MemoryMax=512M\n',
                    old.replace(a.MEMORY_LINE, '#' + a.MEMORY_LINE),
                    old.replace(a.MEMORY_LINE, 'X' + a.MEMORY_LINE)):
            with self.assertRaises(ValueError):
                a.human_unit(bad)

    def test_stop_timeout_covers_the_serial_backup_and_bridge_shutdown(self):
        # 关停是串行的两段：备份收尾做完才通知 Bridge 停（crates/local-server/src/startup/serving.rs），所以逐项相加；
        # 哪天改成并行，相加仍然够用，只是偏保守。
        # 激活脚本停服务的命令时限按这个值加余量算，在 test_activation 的 OperationTests 里钉。
        request = a.BACKUP_SCAN_BUDGET_SECONDS * a.BACKUP_REQUEST_SCANS
        backup = (a.BACKUP_CONNECTION_WINDOW_SECONDS + request
                  + a.BACKUP_RECLAIM_SECONDS + a.BACKUP_FREEZE_BUDGET_SECONDS)
        bridge = a.BRIDGE_STOP_GRACE_SECONDS + a.BRIDGE_STOP_MARGIN_SECONDS
        margins = a.UNBUDGETED_STEPS_SECONDS + a.STOP_SAFETY_MARGIN_SECONDS
        self.assertEqual(a.STOP_TIMEOUT_SECONDS, backup + bridge + margins)
        # 结果本身也钉住：任一项改了，这里先红，提醒把文档里的数字和算法一起看。
        # 预算（各项按上限相加）和总数分开钉：拿余量去补一项漏掉的预算，总数不变也会红。
        self.assertEqual(backup + bridge, 185)
        self.assertEqual(a.STOP_TIMEOUT_SECONDS, 300)
        self.assertGreaterEqual(a.UNBUDGETED_STEPS_SECONDS, 10)
        # 保险余量（用户 2026-11-20 定）：超时被强杀要人处理，所以它不能悄悄变小。
        self.assertEqual(a.STOP_SAFETY_MARGIN_SECONDS, 105)
        # 程序里的三个常量：程序改了这里先红，免得装配还按旧值算。
        producer = 'crates/local-server/src/backup/producer.rs'
        self.assertEqual(a.BACKUP_CONNECTION_WINDOW_SECONDS,
                         rust_seconds('crates/local-server/src/backup/service.rs', 'WINDOW'))
        self.assertEqual(a.BACKUP_RECLAIM_SECONDS, rust_seconds(producer, 'RECLAIM_BUDGET'))
        self.assertEqual(a.BACKUP_SCAN_BUDGET_SECONDS, rust_seconds(producer, 'SCAN_BUDGET'))

    def test_request_scans_count_every_ledger_read_in_the_program(self):
        # 「处理请求」按读几次账本算。这里把生产者源码（producer.rs 和 producer/ 下全部文件：refresh 和状态锁是私有的，
        # 只有它们调得到）里读账本、拿状态锁的每一处按所在方法点名，再钉请求怎么分派、谁调健康页的两个探针。
        # 对不上就先红，提醒把 assembly.py 里的算法重看一遍：四轮复查各发现过一条当时没数到的路径。
        # 钉的是源码文本，有它认不出的：把读账本放进循环或递归（字面次数不变，执行次数变多）、宏生成的调用、
        # 拿到锁之后新加的别的慢操作。所以它只是提醒，不是证明：改这几个文件的人仍要自己看一眼关停要等多久。
        backup = ROOT / 'crates/local-server/src/backup'
        sources = [backup / 'producer.rs'] + sorted((backup / 'producer').rglob('*.rs'))

        def where(receiver, method):
            return dict(collections.Counter(name for name, _ in rust_calls(sources, receiver, method)))
        reads = where('self', 'refresh')
        self.assertEqual(reads, {'next_pending': 1, 'schedule_with': 2, 'decide': 1, 'jobs': 1,
                                 'repository_check_with': 3, 'report_check': 1, 'repository_checks_at': 1})
        # 拿状态锁的每一处：新加一个拿着锁干活的地方（哪怕不读账本），请求和后台线程都会跟着等。
        # 后台线程收尾复位 busy 也拿这把锁（Idle 的 drop），assembly.py 把它列在没算进去的那几样里。
        # 后台读账本的线程（producer/catch_up.rs 的 catch_up_by，T-219）每页拿两次：取读到哪儿、把读回的一页记进队列。
        # 它读账本在锁外，拿着锁只做内存里的记账，所以不算进「等锁时别人读账本」。scanned_offset 只在测试构建里有。
        self.assertEqual(where('self.state', 'lock'), dict.fromkeys(
            ('next_pending', 'schedule_with', 'reclaim', 'launch', 'jobs', 'repository_check_with',
             'report_check', 'repository_checks_at', 'scanned_offset'), 1) | {'catch_up_by': 2})
        # 不经 refresh 直接读账本的每一处：refresh 自己；后台读账本的线程（一次一页、在状态锁之外，关停最多等这一页）；
        # 抽查上报核原意图（按事件键取，最多两条）。新加一处要先想清楚它拿没拿着状态锁、关停要不要等它。
        self.assertEqual(where('self.ledger', 'query'),
                         {'refresh': 1, 'catch_up_by': 1, 'check_intent_record': 1})
        self.assertEqual(where('self.0', 'lock'), {'drop': 1})
        # 请求怎么分派：四种请求各走一个方法、各调一次，dispatch 里没有别的 self 方法调用。
        dispatched = sorted(called for name, called in rust_calls(sources, 'self') if name == 'dispatch')
        self.assertEqual(dispatched, ['next_pending', 'report_check', 'repository_check', 'schedule'])
        for method in dispatched:
            self.assertEqual(where('self', method), {'dispatch': 1}, method)
        # 一个请求自己最多读几次：四种请求里最多的那个。decide 只被 schedule_with 调一次，schedule_with 又只被 schedule
        # 调一次（schedule 只多给一个真实时钟），所以都算在安排封存里；安排抽查同样写法。
        self.assertEqual(where('self', 'decide'), {'schedule_with': 1})
        self.assertEqual(where('self', 'schedule_with'), {'schedule': 1})
        self.assertEqual(where('self', 'repository_check_with'), {'repository_check': 1})
        requests = (reads['next_pending'], reads['schedule_with'] + reads['decide'],
                    reads['repository_check_with'], reads['report_check'])
        self.assertEqual(a.BACKUP_REQUEST_OWN_SCANS, max(requests))
        # 等状态锁时别人读几次：健康页一次快照调两个探针（备份作业、仓库抽查），各拿一次锁、各读一次。
        # 整个 crate 的非测试代码里，调这两个探针的只有那一处（不限接收者叫什么；别的类型起了同名方法也会红，来看一眼）。
        crate = ROOT / 'crates/local-server/src'
        code = [path for path in sorted(crate.rglob('*.rs'))
                if 'tests' not in path.relative_to(crate).parts and path.stem != 'tests'
                and not path.name.endswith('_tests.rs')]
        for probe in ('jobs', 'repository_checks'):
            callers = [(str(path.relative_to(crate)), name)
                       for path in code for name, _ in rust_calls([path], None, probe)]
            self.assertEqual(callers, [('startup/health_input.rs', 'snapshot')], probe)
        self.assertEqual(where('self', 'repository_checks_at'), {'repository_checks': 1})
        self.assertEqual(a.BACKUP_SNAPSHOT_SCANS, reads['jobs'] + reads['repository_checks_at'])
        self.assertEqual(a.BACKUP_REQUEST_SCANS, a.BACKUP_REQUEST_OWN_SCANS + a.BACKUP_SNAPSHOT_SCANS)

    def test_bridge_stop_margin_matches_the_sync_deployment_document(self):
        # Bridge 段的 60 秒是同步部署文档的要求，文档改了这里先红。
        bridge_readme = (ROOT / 'deploy/sync-bridge/README.md').read_text()
        self.assertEqual(bridge_readme.count(
            '审批 `budget.stop_grace_seconds` 加 %d s' % a.BRIDGE_STOP_MARGIN_SECONDS), 1)

    def test_this_release_requires_the_program_with_the_connector(self):
        # 新东西在程序里：出候选和激活预检都要求换程序，而且程序里有这三段字节。
        self.assertIs(a.REQUIRES_NEW_HUMAN_PROGRAM, True)
        # 每段字节：在哪份源码里、原样写成什么样。都是一整个字符串字面量，所以程序里是连续的字节；
        # 上一发行的程序里都没有，在 169 上只读核过。
        pinned = (
            ('/internal/connector', 'crates/local-server/src/board/http.rs',
             '.route("/internal/connector", post(super::connector::command))'),
            ('/api/v1/board/presence', 'crates/local-server/src/board/http.rs',
             '.route("/api/v1/board/presence", get(super::connector::presence))'),
            ('studiolo_bridge component=transport outcome=unlaunched_run_cancelled',
             'crates/local-server/src/sync_plane/driver/diagnostics.rs',
             '"studiolo_bridge component=transport outcome=unlaunched_run_cancelled"'),
        )
        self.assertEqual(a.NEW_HUMAN_PROGRAM_LITERALS, tuple(literal.encode() for literal, _, _ in pinned))
        for literal, path, written in pinned:
            self.assertEqual(rust_code(ROOT / path).count(written), 1, literal)

    def test_program_takes_over_every_leftover_the_launcher_lets_through(self):
        # 启动器放过去的死 socket 要靠程序接管：三个入口在程序里都走同一处接管（r26 起的程序都有）。
        for path, written in (('crates/local-server/src/startup/factor_input.rs', '"HTTP 入口"'),
                              ('crates/local-server/src/startup/plane_input.rs', '"黑板入口"'),
                              ('crates/local-server/src/backup/listener.rs', 'label: "备份入口"')):
            self.assertEqual(rust_code(ROOT / path).count(written), 1, path)

    def test_agent_unit_is_carried_over_byte_for_byte(self):
        for quoted in (False, True):
            scope = (lambda s: '"' + s + '"') if quoted else (lambda s: s)
            old = ('[Service]\nExecStart=/binary serve --allow ' + scope('项目/Studiolo-Docs')
                   + ' --allow ' + scope('项目/Tillerwright-Docs') + ' --allow ' + scope('_规则')
                   + ' --port 4400 --board-socket ' + a.BOARD_SOCKET + '\nProtectSystem=strict\n')
            with self.subTest(quoted=quoted):
                # 线上单元的范围是带引号的；带不带引号都必须认得出同一组范围。
                self.assertEqual(a.agent_unit(old), old)
                # 还是改名前的旧单元（agent 没先按新名发布）也要拒，见 AGENT_READ_SCOPES 的注释。
                renamed_back = old.replace('-Docs', '')
                for bad in (old.replace(' --board-socket ' + a.BOARD_SOCKET, ''),
                            old.replace(' --allow ' + scope('项目/Tillerwright-Docs'), ''), old + old,
                            old.replace('/binary', a.OLD_RELEASE + '/bin/studiolo-agent'), renamed_back):
                    with self.assertRaises(ValueError):
                        a.agent_unit(bad)

    def test_required_agent_scopes_are_visible_in_the_project_registry(self):
        # 部署上限（agent 的 --allow）由注册表生成；这里要的范围不在注册表里，agent 单元就永远凑不齐。
        spec = importlib.util.spec_from_file_location('project_registry', ROOT / 'scripts/project_registry.py')
        registry = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(registry)
        self.assertLessEqual(set(a.AGENT_READ_SCOPES), set(registry.allow_of(registry.load_registry(ROOT))))


if __name__ == '__main__':
    unittest.main()
