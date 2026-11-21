"""黑板形状发行（当前 r16→r17）的固定变更面；夹具不接触生产或凭据。"""
import ast
import importlib.util
from pathlib import Path
import re
import unittest
import assembly as a

ROOT = Path(__file__).resolve().parents[2]


def board_launcher():
    """把 r7/r8 那份启动器补成上一发行（r16）的形状：带黑板常量与参数、只读身份目录参数，RELEASE 指向上一发行。"""
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
                        '        "--sync-transport", "' + a.SYNC_APPROVAL + '",')


def board_unit():
    """把 r7/r8 那份人侧单元补成上一发行（r16）的形状：黑板 RuntimeDirectory，黑板五个写根加 Bridge 两个写根，
    都在 [Service] 节里、顺序和线上一致。"""
    text = (ROOT / 'deploy/human/current-v2/studiolo-human.service').read_text().replace(
        'm4-t092-20260920-r7', a.OLD_RELEASE.rsplit('/', 1)[1])
    text = text.replace('RuntimeDirectory=studiolo-human\n',
                        'RuntimeDirectory=studiolo-human studiolo-board\n')
    return text.replace('ReadWritePaths=/run/studiolo-human\n',
                        'ReadWritePaths=/run/studiolo-human\n' + ''.join(
                            'ReadWritePaths=' + path + '\n'
                            for path in (a.VAULT, a.WRITER, a.BOARD, a.AGENT, a.BOARD_RUNTIME,
                                         a.BRIDGE_ROOT, a.TRANSPORT_ROOT)))


def rust_seconds(relative, name):
    """从 Rust 源码读一个 `const NAME: Duration = Duration::from_secs(N);`，读不到或不止一处就失败。"""
    found = re.findall(r'^const ' + name + r': Duration = Duration::from_secs\((\d+)\);$',
                       (ROOT / relative).read_text(), re.MULTILINE)
    assert len(found) == 1, (relative, name, found)
    return int(found[0])


def rust_calls(relative, needle):
    """一份 Rust 源码里 `needle` 出现在哪些方法里、各几次：{方法名: 次数}。只认 impl 块里缩进四格的方法；
    出现在方法以外就失败（那是这个助手不认识的写法，要人来看）。"""
    counts, name = {}, None
    for line in (ROOT / relative).read_text().splitlines():
        head = re.match(r'    (?:pub(?:\([a-z]+\))? )?fn (\w+)[(<]', line)
        if head:
            name = head.group(1)
        elif line.startswith('}'):
            name = None
        found = line.count(needle)
        if found:
            assert name is not None, (relative, needle, line)
            counts[name] = counts.get(name, 0) + found
    return counts


class AssemblyTests(unittest.TestCase):
    def test_r17_upgrades_the_activated_r16(self):
        self.assertEqual(a.BUILD, 'm5m6-board-20260920-r17')
        self.assertEqual(a.RELEASE.rsplit('/', 1)[1], a.BUILD)
        self.assertTrue(a.OLD_RELEASE.endswith('/m5m6-board-20260920-r16'))
        self.assertEqual(a.WEB_PREFIX, 'web/')

    def test_launcher_preserves_the_approved_bridge_wiring(self):
        old = board_launcher()
        new = a.launcher(old)
        backup = ' "--backup-config", "' + a.BACKUP_CONFIG + '",'
        restored = new.replace(a.RELEASE, a.OLD_RELEASE).replace(backup, '')
        self.assertEqual(restored, old)
        self.assertEqual(new.count('"--backup-config"'), 1)
        self.assertEqual(new.count('"--agent-data"'), 1)
        self.assertEqual(new.count('"--sync-transport"'), 1)
        # 备份配置紧跟在同步审批后面，路径是写死的那一份，和主人配置同目录。
        self.assertIn('"--sync-transport", "' + a.SYNC_APPROVAL + '",' + backup, new)
        self.assertEqual(Path(a.BACKUP_CONFIG).parent, Path(a.SYNC_APPROVAL).parent)

        # 所有函数（含 main）的 AST 不变；只有模块级发行路径不同。
        def functions(source):
            return {n.name: ast.dump(n) for n in ast.parse(source).body
                    if isinstance(n, ast.FunctionDef)}
        self.assertEqual(functions(old), functions(restored))
        self.assertIn('"--canonical-writer-data", WRITER', new)
        self.assertIn('"STUDIOLO_TOTP_SEED_HEX": credential()', new)
        # 已转换的启动器不能再过一遍；接线或标签漂移也拒绝；还没带只读身份目录参数的（r15 形状）也拒绝。
        for bad in (new, old + '\n# "--backup-config"\n', old + '\n# "--agent-data"\n',
                    old.replace(' "--agent-data", AGENT,', ''),
                    old.replace('"--board-socket", BOARD_SOCKET,', ''),
                    old.replace('"--sync-transport"', '"--foreign"'),
                    old.replace(a.SYNC_APPROVAL, '/foreign/approval.json'),
                    old + '\n# "--sync-transport"\n',
                    old.replace('正式黑板库 · Obsidian 同步试跑', '未知标签')):
            with self.assertRaises(ValueError):
                a.launcher(bad)

    def test_human_unit_repoints_the_release_and_wires_the_backup_producer(self):
        old = board_unit()
        old_stop, new_stop = (a.stop_timeout_line(n) for n in (a.OLD_STOP_TIMEOUT_SECONDS, a.STOP_TIMEOUT_SECONDS))
        self.assertEqual(old.count(old_stop), 1)
        new = a.human_unit(old)
        added = ''.join('ReadWritePaths=' + path + '\n' for path in (a.BACKUP_RUNTIME, a.BACKUP_STAGING))
        restored = new.replace(a.RELEASE, a.OLD_RELEASE).replace(added, '').replace(new_stop, old_stop)
        self.assertEqual(restored, old)
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
        self.assertEqual(new_stop, 'TimeoutStopSec=195\n')
        # 拒绝：已转换过的；停止超时不是钉住的旧值、多出一行（不管值是多少）；已带备份路径；黑板或 Bridge 接线缺了；
        # 发行路径不是两处。
        for bad in (new, old.replace(old_stop, 'TimeoutStopSec=120\n'), old + old_stop, old + 'TimeoutStopSec=120\n',
                    old.replace(old_stop, ''),
                    old + 'ReadWritePaths=' + a.BACKUP_RUNTIME + '\n', old + '# studiolo-backup\n',
                    old.replace('ReadWritePaths=' + a.BOARD + '\n', ''),
                    old.replace('ReadWritePaths=' + a.BRIDGE_ROOT + '\n', ''),
                    old.replace('ReadWritePaths=' + a.TRANSPORT_ROOT + '\n', ''),
                    old.replace(a.OLD_RELEASE + '/launch-human.py', '/foreign/launch.py'),
                    old.replace('RuntimeDirectory=studiolo-human studiolo-board\n',
                                'RuntimeDirectory=studiolo-human\n')):
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
        self.assertEqual(a.STOP_TIMEOUT_SECONDS, backup + bridge + a.UNBUDGETED_STEPS_SECONDS)
        # 结果本身也钉住：任一项改了，这里先红，提醒把文档里的数字和算法一起看。
        self.assertEqual(a.STOP_TIMEOUT_SECONDS, 195)
        self.assertGreaterEqual(a.UNBUDGETED_STEPS_SECONDS, 10)
        self.assertGreater(a.STOP_TIMEOUT_SECONDS, a.OLD_STOP_TIMEOUT_SECONDS)
        # 程序里的三个常量：程序改了这里先红，免得装配还按旧值算。
        producer = 'crates/local-server/src/backup/producer.rs'
        self.assertEqual(a.BACKUP_CONNECTION_WINDOW_SECONDS,
                         rust_seconds('crates/local-server/src/backup/service.rs', 'WINDOW'))
        self.assertEqual(a.BACKUP_RECLAIM_SECONDS, rust_seconds(producer, 'RECLAIM_BUDGET'))
        self.assertEqual(a.BACKUP_SCAN_BUDGET_SECONDS, rust_seconds(producer, 'SCAN_BUDGET'))

    def test_request_scans_count_every_ledger_read_in_the_program(self):
        # 「处理请求」按读几次账本算。读账本的每一处都在这里点名：哪个函数里、几次。多一处、少一处、挪到别的函数，
        # 这里先红，提醒把 assembly.py 里的算法重看一遍（三轮复查各发现过一条当时没数到的路径，不再靠人去数）。
        producer = 'crates/local-server/src/backup/producer.rs'
        check = 'crates/local-server/src/backup/producer/check.rs'
        own, checks = rust_calls(producer, 'self.refresh('), rust_calls(check, 'self.refresh(')
        self.assertEqual(own, {'next_pending': 1, 'schedule': 2, 'decide': 1, 'jobs': 1})
        self.assertEqual(checks, {'repository_check_with': 3, 'report_check': 1, 'repository_checks_at': 1})
        # 一个请求自己最多读几次：四种请求里最多的那个。decide 只被 schedule 调一次，所以算在安排封存里。
        self.assertEqual(rust_calls(producer, 'self.decide('), {'schedule': 1})
        self.assertEqual(rust_calls(check, 'self.repository_check_with('), {'repository_check': 1})
        requests = (own['next_pending'], own['schedule'] + own['decide'],
                    checks['repository_check_with'], checks['report_check'])
        self.assertEqual(a.BACKUP_REQUEST_OWN_SCANS, max(requests))
        # 等状态锁时别人读几次：健康页一次快照调两个探针（备份作业、仓库抽查），各拿一次锁、各读一次。
        # 别处没有人调这两个探针；worker 的请求在同一个接受循环里一个个处理，不会互相等锁。
        health = 'crates/local-server/src/startup/health_input.rs'
        self.assertEqual(rust_calls(health, 'probe.jobs('), {'snapshot': 1})
        self.assertEqual(rust_calls(health, 'probe.repository_checks('), {'snapshot': 1})
        self.assertEqual(rust_calls(producer, 'self.repository_checks_at('), {'repository_checks': 1})
        self.assertEqual(a.BACKUP_SNAPSHOT_SCANS, own['jobs'] + checks['repository_checks_at'])
        self.assertEqual(a.BACKUP_REQUEST_SCANS, a.BACKUP_REQUEST_OWN_SCANS + a.BACKUP_SNAPSHOT_SCANS)

    def test_bridge_stop_margin_matches_the_sync_deployment_document(self):
        # Bridge 段的 60 秒是同步部署文档的要求，文档改了这里先红。
        bridge_readme = (ROOT / 'deploy/sync-bridge/README.md').read_text()
        self.assertEqual(bridge_readme.count(
            '审批 `budget.stop_grace_seconds` 加 %d s' % a.BRIDGE_STOP_MARGIN_SECONDS), 1)

    def test_this_release_requires_the_new_human_program(self):
        # 启动器加了上一发行的程序不认的参数，就必须换人侧程序（出候选和激活预检靠这两个常量拒绝沿用旧程序）。
        old = board_launcher()
        new = a.launcher(old)
        self.assertIs(a.REQUIRES_NEW_HUMAN_PROGRAM, True)
        self.assertEqual(a.NEW_HUMAN_PROGRAM_LITERALS, (b'--backup-config',))
        for literal in a.NEW_HUMAN_PROGRAM_LITERALS:
            name = literal.decode()
            # 确实是本版启动器新带的参数：上一发行的启动器里没有，新的里有。
            self.assertNotIn(name, old)
            self.assertEqual(new.count('"' + name + '"'), 1)
            # 新程序认它，而且参数名在源码里是原样的字符串字面量（编进程序后能按字节找到）。
            table = (ROOT / 'crates/local-server/src/options/table.rs').read_text()
            self.assertEqual(table.count('"' + name + '"'), 1)

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
