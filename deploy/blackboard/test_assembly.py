"""黑板形状发行（当前 r14→r15）的固定变更面；夹具不接触生产或凭据。"""
import ast
from pathlib import Path
import unittest
import assembly as a

ROOT = Path(__file__).resolve().parents[2]


def board_launcher():
    """把 r7/r8 那份启动器补成黑板形状：带黑板常量与参数、RELEASE 指向上一发行。"""
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
                        '        "--board-agent-data", AGENT, "--board-socket", BOARD_SOCKET,\n'
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
    def test_r15_upgrades_the_activated_r14(self):
        self.assertEqual(a.BUILD, 'm5m6-board-20260920-r15')
        self.assertEqual(a.RELEASE.rsplit('/', 1)[1], a.BUILD)
        self.assertTrue(a.OLD_RELEASE.endswith('/m5m6-board-20260920-r14'))
        self.assertEqual(a.WEB_PREFIX, 'web/')

    def test_launcher_preserves_the_approved_bridge_wiring(self):
        old = board_launcher()
        new = a.launcher(old)
        restored = new.replace(a.RELEASE, a.OLD_RELEASE)
        self.assertEqual(restored, old)
        self.assertEqual(new.count('"--sync-transport"'), 1)

        # 所有函数（含 main）的 AST 不变；只有模块级发行路径不同。
        def functions(source):
            return {n.name: ast.dump(n) for n in ast.parse(source).body
                    if isinstance(n, ast.FunctionDef)}
        self.assertEqual(functions(old), functions(new))
        self.assertIn('"--canonical-writer-data", WRITER', new)
        self.assertIn('"STUDIOLO_TOTP_SEED_HEX": credential()', new)
        # 已转换的启动器不能再过一遍；接线或标签漂移也拒绝。
        for bad in (new, old.replace('"--board-socket", BOARD_SOCKET,', ''),
                    old.replace('"--sync-transport"', '"--foreign"'),
                    old.replace(a.SYNC_APPROVAL, '/foreign/approval.json'),
                    old + '\n# "--sync-transport"\n',
                    old.replace('正式黑板库 · Obsidian 同步试跑', '未知标签')):
            with self.assertRaises(ValueError):
                a.launcher(bad)

    def test_human_unit_only_repoints_the_release_path(self):
        old = board_unit()
        old += ''.join('ReadWritePaths=' + path + '\n' for path in (a.BRIDGE_ROOT, a.TRANSPORT_ROOT))
        new = a.human_unit(old)
        restored = new.replace(a.RELEASE, a.OLD_RELEASE)
        self.assertEqual(restored, old)
        self.assertIn('LoadCredentialEncrypted=totp-next:', new)
        self.assertIn('RuntimeDirectory=studiolo-human studiolo-board\n', new)
        for bad in (new, old.replace('ReadWritePaths=' + a.BOARD + '\n', ''),
                    old.replace('ReadWritePaths=' + a.BRIDGE_ROOT + '\n', ''),
                    old.replace('ReadWritePaths=' + a.TRANSPORT_ROOT + '\n', ''),
                    old.replace(a.OLD_RELEASE + '/launch-human.py', '/foreign/launch.py'),
                    old.replace('RuntimeDirectory=studiolo-human studiolo-board\n',
                                'RuntimeDirectory=studiolo-human\n')):
            with self.assertRaises(ValueError):
                a.human_unit(bad)

    def test_agent_unit_is_carried_over_byte_for_byte(self):
        for quoted in (False, True):
            scope = (lambda s: '"' + s + '"') if quoted else (lambda s: s)
            old = ('[Service]\nExecStart=/binary serve --allow ' + scope('项目/Studiolo')
                   + ' --allow ' + scope('项目/Tillerwright') + ' --allow ' + scope('_规则')
                   + ' --port 4400 --board-socket ' + a.BOARD_SOCKET + '\nProtectSystem=strict\n')
            with self.subTest(quoted=quoted):
                # 线上单元的范围是带引号的；带不带引号都必须认得出同一组范围。
                self.assertEqual(a.agent_unit(old), old)
                for bad in (old.replace(' --board-socket ' + a.BOARD_SOCKET, ''),
                            old.replace(' --allow ' + scope('项目/Tillerwright'), ''), old + old,
                            old.replace('/binary', a.OLD_RELEASE + '/bin/studiolo-agent')):
                    with self.assertRaises(ValueError):
                        a.agent_unit(bad)


if __name__ == '__main__':
    unittest.main()
