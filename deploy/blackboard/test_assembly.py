"""黑板形状发行（当前 r16→r17）的固定变更面；夹具不接触生产或凭据。"""
import ast
import importlib.util
from pathlib import Path
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
    """把 r7/r8 那份人侧单元补成黑板形状：带黑板 RuntimeDirectory 与五个写根。"""
    text = (ROOT / 'deploy/human/current-v2/studiolo-human.service').read_text().replace(
        'm4-t092-20260920-r7', a.OLD_RELEASE.rsplit('/', 1)[1])
    text = text.replace('RuntimeDirectory=studiolo-human\n',
                        'RuntimeDirectory=studiolo-human studiolo-board\n')
    return text.replace('ReadWritePaths=/run/studiolo-human\n',
                        'ReadWritePaths=/run/studiolo-human\n' + ''.join(
                            'ReadWritePaths=' + path + '\n'
                            for path in (a.VAULT, a.WRITER, a.BOARD, a.AGENT, a.BOARD_RUNTIME)))


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
        old += ''.join('ReadWritePaths=' + path + '\n' for path in (a.BRIDGE_ROOT, a.TRANSPORT_ROOT))
        new = a.human_unit(old)
        added = ''.join('ReadWritePaths=' + path + '\n' for path in (a.BACKUP_RUNTIME, a.BACKUP_STAGING))
        restored = new.replace(a.RELEASE, a.OLD_RELEASE).replace(added, '').replace(a.STOP_TIMEOUT, a.STOP_TIMEOUT_OLD)
        self.assertEqual(restored, old)
        self.assertIn('LoadCredentialEncrypted=totp-next:', new)
        self.assertIn('RuntimeDirectory=studiolo-human studiolo-board\n', new)
        # 备份的两个写根紧跟在 Bridge 传输根后面，路径和 worker 部署模板里 tmpfiles 建的两个目录是同一组。
        self.assertIn('ReadWritePaths=' + a.TRANSPORT_ROOT + '\n' + added, new)
        tmpfiles = (ROOT / 'deploy/backup-worker/systemd/studiolo-backup.tmpfiles.conf').read_text()
        for path in (a.BACKUP_RUNTIME, a.BACKUP_STAGING):
            self.assertIn('d ' + path + ' 0750 ', tmpfiles)
        self.assertEqual(new.count('TimeoutStopSec='), 1)
        self.assertIn(a.STOP_TIMEOUT, new)
        for bad in (new, old.replace(a.STOP_TIMEOUT_OLD, 'TimeoutStopSec=120\n'), old + a.STOP_TIMEOUT_OLD,
                    old + 'ReadWritePaths=' + a.BACKUP_RUNTIME + '\n', old + '# studiolo-backup\n',
                    old.replace('ReadWritePaths=' + a.BOARD + '\n', ''),
                    old.replace('ReadWritePaths=' + a.BRIDGE_ROOT + '\n', ''),
                    old.replace('ReadWritePaths=' + a.TRANSPORT_ROOT + '\n', ''),
                    old.replace(a.OLD_RELEASE + '/launch-human.py', '/foreign/launch.py'),
                    old.replace('RuntimeDirectory=studiolo-human studiolo-board\n',
                                'RuntimeDirectory=studiolo-human\n')):
            with self.assertRaises(ValueError):
                a.human_unit(bad)

    def test_stop_timeout_covers_backup_shutdown_and_fits_the_activation_stop_window(self):
        seconds = int(a.STOP_TIMEOUT.strip().split('=', 1)[1])
        # 下限：在途的回收最多 30 秒，加上封存最多一个冻结预算（批准的是 30 秒），再留 30 秒给原来的关停链。
        self.assertGreaterEqual(seconds, 90)
        # 上限：激活脚本停人侧服务的命令时限。单元的停止超时比它长的话，慢关停会让激活和回滚在这条命令上超时。
        stop = "self.runtime.command(['systemctl', 'stop', 'studiolo-human.service'], 110)"
        self.assertEqual((ROOT / 'deploy/blackboard/activation_ops.py').read_text().count(stop), 1)
        self.assertLess(seconds, 110)
        self.assertNotEqual(a.STOP_TIMEOUT, a.STOP_TIMEOUT_OLD)

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
