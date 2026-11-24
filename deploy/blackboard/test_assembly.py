"""黑板形状发行（当前 r23→r24）的固定变更面；夹具不接触生产或凭据。"""
import ast
import collections
import importlib.util
import os
from pathlib import Path
import re
import socket
import tempfile
import unittest
from unittest import mock
import assembly as a

ROOT = Path(__file__).resolve().parents[2]


def board_launcher():
    """把 r7/r8 那份启动器补成上一发行（r23，接线沿用 r17）的形状：带黑板常量与参数、只读身份目录参数、备份配置参数，
    RELEASE 指向上一发行。"""
    text = (ROOT / 'deploy/human/current-v2/launch-human.py').read_text().replace(
        'm4-t092-20260920-r7', a.OLD_RELEASE.rsplit('/', 1)[1]).replace(
        '服务器示例库', '正式黑板库 · Obsidian 同步试跑')
    text = text.replace(
        'SOCKET = "/run/studiolo-human/http.sock"',
        'SOCKET = "/run/studiolo-human/http.sock"\n'
        + '\n'.join(name + ' = ' + repr(value) for name, value in (
            ('VAULT', a.VAULT), ('WRITER', a.WRITER), ('BOARD', a.BOARD),
            ('AGENT', a.AGENT), ('BOARD_SOCKET', a.BOARD_SOCKET))))
    return text.replace('"--factor-socket", SOCKET, "--policy-data", POLICY,',
                        '"--factor-socket", SOCKET, "--policy-data", POLICY,\n'
                        '        "--canonical-writer-data", WRITER, "--board-data", BOARD,\n'
                        '        "--board-agent-data", AGENT, "--agent-data", AGENT, "--board-socket", BOARD_SOCKET,\n'
                        '        "--sync-transport", "' + a.SYNC_APPROVAL + '",'
                        ' "--backup-config", "' + a.BACKUP_CONFIG + '",')


def board_unit():
    """把 r7/r8 那份人侧单元补成上一发行（r23，接线沿用 r17）的形状：黑板 RuntimeDirectory，黑板五个写根、Bridge 两个写根、
    备份两个写根，都在 [Service] 节里、顺序和线上一致；停止超时是按预算算出来的那一行。"""
    text = (ROOT / 'deploy/human/current-v2/studiolo-human.service').read_text().replace(
        'm4-t092-20260920-r7', a.OLD_RELEASE.rsplit('/', 1)[1])
    text = text.replace('RuntimeDirectory=studiolo-human\n',
                        'RuntimeDirectory=studiolo-human studiolo-board\n')
    assert text.count('TimeoutStopSec=90\n') == 1
    text = text.replace('TimeoutStopSec=90\n', a.stop_timeout_line(a.STOP_TIMEOUT_SECONDS))
    return text.replace('ReadWritePaths=/run/studiolo-human\n',
                        'ReadWritePaths=/run/studiolo-human\n' + ''.join(
                            'ReadWritePaths=' + path + '\n'
                            for path in (a.VAULT, a.WRITER, a.BOARD, a.AGENT, a.BOARD_RUNTIME,
                                         a.BRIDGE_ROOT, a.TRANSPORT_ROOT, a.BACKUP_RUNTIME, a.BACKUP_STAGING)))


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
    def test_r24_upgrades_the_activated_r23(self):
        self.assertEqual(a.BUILD, 'm5m6-board-20260920-r24')
        self.assertEqual(a.RELEASE.rsplit('/', 1)[1], a.BUILD)
        self.assertTrue(a.OLD_RELEASE.endswith('/m5m6-board-20260920-r23'))
        self.assertEqual(a.WEB_PREFIX, 'web/')

    def test_launcher_preserves_the_approved_bridge_wiring(self):
        old = board_launcher()
        new = a.launcher(old)
        backup = ' "--backup-config", "' + a.BACKUP_CONFIG + '",'
        # 本版只换发行路径和 http.sock 的核对：换回去就是上一发行的那一份，一个字不差。
        restored = new.replace(a.RELEASE, a.OLD_RELEASE).replace(a.SOCKET_CHECK, a.OLD_SOCKET_CHECK)
        self.assertEqual(restored, old)
        self.assertEqual((new.count(a.SOCKET_CHECK), new.count(a.OLD_SOCKET_CHECK)), (1, 0))
        self.assertNotIn(a.OLD_RELEASE, new)
        self.assertEqual(new.count('"--backup-config"'), 1)
        self.assertEqual(new.count('"--agent-data"'), 1)
        self.assertEqual(new.count('"--sync-transport"'), 1)
        # 备份配置紧跟在同步审批后面，路径是写死的那一份，和主人配置同目录。
        self.assertIn('"--sync-transport", "' + a.SYNC_APPROVAL + '",' + backup, new)
        self.assertEqual(Path(a.BACKUP_CONFIG).parent, Path(a.SYNC_APPROVAL).parent)

        # 除了 preflight 里那一处核对，所有函数（含 main）的 AST 不变；模块级只有发行路径不同。
        def functions(source):
            return {n.name: ast.dump(n) for n in ast.parse(source).body
                    if isinstance(n, ast.FunctionDef)}
        before, after = functions(old), functions(new)
        self.assertEqual(before.keys(), after.keys())
        self.assertEqual({name for name in before if before[name] != after[name]}, {'preflight'})
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
                    # 已经换过核对的、核对被改过的，都不是上一发行的那一份。
                    old.replace(a.OLD_SOCKET_CHECK, a.SOCKET_CHECK),
                    old.replace('raise RuntimeError("rejected")\n    finally', 'pass\n    finally'),
                    old.replace(a.SYNC_APPROVAL, '/foreign/approval.json'),
                    old + '\n# "--sync-transport"\n',
                    old.replace('正式黑板库 · Obsidian 同步试跑', '未知标签')):
            with self.assertRaises(ValueError):
                a.launcher(bad)

    def test_launcher_hands_only_our_own_single_name_socket_to_the_product(self):
        """T-231：残留的 http.sock 由程序判死；启动器只放过自己的、一个名字的 socket，什么都不删。"""
        module = {'__name__': 'launcher_under_test'}
        exec(compile(a.launcher(board_launcher()), 'launch-human.py', 'exec'), module)
        module['inspect_existing'] = lambda *args, **kwargs: None

        def preflight(directory, uid=None):
            module['open_directory'] = lambda path, private=False: os.open(
                directory if path == '/run/studiolo-human' else '/', os.O_RDONLY | os.O_DIRECTORY)
            with mock.patch.object(os, 'geteuid', return_value=os.getuid() if uid is None else uid):
                module['preflight']()

        def dead(path):
            with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as listener:
                listener.bind(str(path))
                listener.listen()

        with tempfile.TemporaryDirectory(prefix='stu-l-', dir='/tmp') as root:
            node = Path(root) / 'http.sock'
            other = Path(root) / 'other'
            preflight(root)
            dead(node)
            before = node.lstat()
            preflight(root)
            self.assertEqual(node.lstat(), before)
            # 别人的 socket、两个名字的 socket 照旧拒绝，原样留着。
            with self.assertRaises(RuntimeError):
                preflight(root, uid=os.getuid() + 1)
            os.link(node, other)
            with self.assertRaises(RuntimeError):
                preflight(root)
            self.assertTrue(node.is_socket())
            node.unlink()
            # 链接（指着一个合格的 socket 也不行）、普通文件、目录、管道。
            node.symlink_to(other)
            with self.assertRaises(RuntimeError):
                preflight(root)
            node.unlink()
            for make, remove in ((node.touch, node.unlink), (node.mkdir, node.rmdir),
                                 (lambda: os.mkfifo(node), node.unlink)):
                make()
                with self.assertRaises(RuntimeError):
                    preflight(root)
                remove()

    def test_human_unit_only_repoints_the_release_and_raises_the_memory_limit(self):
        old = board_unit()
        new_stop = a.stop_timeout_line(a.STOP_TIMEOUT_SECONDS)
        self.assertEqual(old.count(new_stop), 1)
        new = a.human_unit(old)
        added = ''.join('ReadWritePaths=' + path + '\n' for path in (a.BACKUP_RUNTIME, a.BACKUP_STAGING))
        # 本版只换发行路径和内存上限那一行：换回去就是上一发行的那一份，一个字不差。
        restored = new.replace(a.RELEASE, a.OLD_RELEASE).replace(a.MEMORY_LINE, a.OLD_MEMORY_LINE)
        self.assertEqual(restored, old)
        # 上限恰好一行、落在 [Service] 节里，旧值一处不剩（只看「还原后相等」的话，忘了换上限的变换也能过）。
        self.assertEqual(a.OLD_MEMORY_LINE, 'MemoryMax=512M\n')
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
        # 内存上限不是恰好一行旧值（已经是新值、别的值、没有、多一行、写在注释里、前面带别的字）。
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
                    old.replace(a.OLD_MEMORY_LINE, a.MEMORY_LINE), old.replace(a.OLD_MEMORY_LINE, 'MemoryMax=1G\n'),
                    old.replace(a.OLD_MEMORY_LINE, ''), old + a.OLD_MEMORY_LINE, old + a.MEMORY_LINE,
                    old.replace(a.OLD_MEMORY_LINE, '#' + a.OLD_MEMORY_LINE),
                    old.replace(a.OLD_MEMORY_LINE, 'X' + a.OLD_MEMORY_LINE)):
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
        self.assertEqual(reads, {'next_pending': 1, 'schedule': 2, 'decide': 1, 'jobs': 1,
                                 'repository_check_with': 3, 'report_check': 1, 'repository_checks_at': 1})
        # 拿状态锁的每一处：新加一个拿着锁干活的地方（哪怕不读账本），请求和后台线程都会跟着等。
        # 后台线程收尾复位 busy 也拿这把锁（Idle 的 drop），assembly.py 把它列在没算进去的那几样里。
        # 后台读账本的线程（producer/catch_up.rs 的 catch_up_by，T-219）每页拿两次：取读到哪儿、把读回的一页记进队列。
        # 它读账本在锁外，拿着锁只做内存里的记账，所以不算进「等锁时别人读账本」。scanned_offset 只在测试构建里有。
        self.assertEqual(where('self.state', 'lock'), dict.fromkeys(
            ('next_pending', 'schedule', 'reclaim', 'launch', 'jobs', 'repository_check_with',
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
        # 一个请求自己最多读几次：四种请求里最多的那个。decide 只被 schedule 调一次，所以算在安排封存里。
        self.assertEqual(where('self', 'decide'), {'schedule': 1})
        self.assertEqual(where('self', 'repository_check_with'), {'repository_check': 1})
        requests = (reads['next_pending'], reads['schedule'] + reads['decide'],
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

    def test_this_release_requires_the_new_human_program(self):
        # 本版的修复在人侧程序里，必须换程序（出候选和激活预检靠这两个常量拒绝沿用旧程序）。
        self.assertIs(a.REQUIRES_NEW_HUMAN_PROGRAM, True)
        # 每段字节：在哪份源码里、原样写成什么样。每段都从一个字符串字面量的开头引号起，后面紧跟着占位符或结尾引号，
        # 所以程序里连续的字节就到这里为止（中文按 UTF-8 编码后比）。上一发行的程序里都没有，在 169 上只读核过。
        window = 'crates/local-server/src/checkpoint.rs'
        pinned = (
            ('写者冻结', window, 'steps.mark("写者冻结")'),
            ('定格写者两棵树', window, 'steps.mark("定格写者两棵树")'),
            ('核写者回执：', 'crates/local-server/src/checkpoint/released.rs',
             'Self::Receipts(error) => write!(f, "核写者回执：{error}")'),
        )
        self.assertEqual(a.NEW_HUMAN_PROGRAM_LITERALS, tuple(literal.encode() for literal, _, _ in pinned))
        for literal, path, written in pinned:
            # 只看编进程序的那部分源码（测试模块和注释不算）：原样的字符串字面量，只此一处。
            code = rust_code(ROOT / path)
            self.assertEqual(code.count(written), 1, written)
            self.assertEqual(code.count(literal), 1, written)
            self.assertTrue(written.split('"' + literal, 1)[1].startswith(('{', '"')), written)

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
