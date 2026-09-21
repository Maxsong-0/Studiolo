"""黑板部署装配的固定变更面；夹具不接触生产或凭据。"""
import ast
from pathlib import Path
import unittest
import assembly as a

ROOT = Path(__file__).resolve().parents[2]


class AssemblyTests(unittest.TestCase):
    def test_launcher_keeps_factor_reader_and_environment_exact(self):
        old = (ROOT / 'deploy/human/current-v2/launch-human.py').read_text().replace(
            'm4-t092-20260920-r7', 'm4-t094-20260920-r8')
        new = a.launcher(old)
        def functions(source):
            return {n.name: ast.dump(n) for n in ast.parse(source).body
                    if isinstance(n, ast.FunctionDef)}
        original, changed = functions(old), functions(new)
        self.assertEqual(original.keys(), changed.keys())
        for name in original.keys() - {'main', 'preflight'}:
            self.assertEqual(original[name], changed[name], name)
        self.assertIn('"--canonical-writer-data", WRITER', new)
        self.assertNotIn('"--runner-config"', new)
        self.assertIn('"STUDIOLO_TOTP_SEED_HEX": credential()', new)
        self.assertIn('持续同步未接通', new)
        with self.assertRaises(ValueError):
            a.launcher(new)

    def test_unit_only_adds_declared_writer_roots_and_runtime(self):
        old = (ROOT / 'deploy/human/current-v2/studiolo-human.service').read_text().replace(
            'm4-t092-20260920-r7', 'm4-t094-20260920-r8')
        new = a.human_unit(old)
        restored = new.replace(a.RELEASE, a.OLD_RELEASE).replace(
            'RuntimeDirectory=studiolo-human studiolo-board\n',
            'RuntimeDirectory=studiolo-human\n')
        for path in (a.VAULT, a.WRITER, a.BOARD, a.AGENT, a.BOARD_RUNTIME):
            restored = restored.replace('ReadWritePaths=' + path + '\n', '')
        self.assertEqual(old, restored)
        self.assertIn('LoadCredentialEncrypted=totp-next:', new)

    def test_agent_preserves_all_bytes_except_socket_argument(self):
        old = '[Service]\nExecStart=/binary serve --allow 项目/Studiolo --allow 项目/Tillerwright --allow _规则 --port 4400\nProtectSystem=strict\n'
        new = a.agent_unit(old)
        self.assertEqual(new.replace(' --board-socket ' + a.BOARD_SOCKET, ''), old)
        for bad in (new, old.replace('--allow 项目/Tillerwright ', ''), old + old):
            with self.assertRaises(ValueError):
                a.agent_unit(bad)


if __name__ == '__main__':
    unittest.main()
