#!/usr/bin/env python3
"""M4 操作包契约：只接受已知单元形状，限定权限增量，绑定每份操作材料。"""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('prepare_m4', Path(__file__).with_name('prepare-m4-policy.py'))
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)


class PreparePolicyTests(unittest.TestCase):
    def setUp(self):
        self.addCleanup(os.umask, os.umask(0o022))
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.raw = (m.ROOT / 'deploy/agent/studiolo-agent-system.service.in').read_bytes().replace(
            b'@RELEASE@', b'm4-fixture-001')
        self.unit = self.root / 'unit.service'
        self.unit.write_bytes(self.raw)
        self.output = self.root / 'bundle'

    def test_only_policy_flag_and_writable_path_change(self):
        new, release = m.render_unit(self.raw)
        old_lines, new_lines = self.raw.decode().splitlines(), new.decode().splitlines()
        self.assertEqual(len(old_lines), len(new_lines))
        changes = [(a, b) for a, b in zip(old_lines, new_lines) if a != b]
        self.assertEqual(len(changes), 2)
        self.assertEqual(changes[0][1], changes[0][0] + ' --policy ' + m.POLICY)
        self.assertEqual(changes[1][1], changes[1][0] + ' ' + m.POLICY)
        self.assertEqual(release, m.BASE + 'studiolo-agent-releases/m4-fixture-001')

    def test_permissions_manifest_and_baseline_bindings(self):
        script = m.prepare(self.unit, 'a' * 64, self.output)
        self.assertEqual(self.output.stat().st_mode & 0o777, 0o700)
        for line in (self.output / 'FILES.sha256').read_text().splitlines():
            sha, name = line.split('  ')
            self.assertEqual(sha, hashlib.sha256((self.output / name).read_bytes()).hexdigest())
            self.assertEqual((self.output / name).stat().st_mode & 0o777, 0o600)
        body = script.read_text()
        self.assertNotRegex(body, r'@[A-Z_]+@')
        self.assertIn('old_unit_sha=' + hashlib.sha256(self.raw).hexdigest(), body)
        self.assertEqual(subprocess.run(['bash', '-n', str(script)], timeout=10).returncode, 0)
        capability = json.loads((self.output / 'claude.json').read_text())
        self.assertEqual(capability['read']['project_whitelist'], ['项目/Studiolo', '_规则'])
        self.assertFalse(capability['write']['board'])
        self.assertFalse(capability['write']['content_direct'])
        self.assertFalse(capability['policy']['activate'])
        self.assertTrue(capability['policy']['propose_desired'])

    def test_foreign_or_weakened_unit_rejected_before_output(self):
        for bad in [self.raw + b'ExecStart=/bin/true\n',
                    self.raw.replace(b'--port 4400', b'--port 4402'),
                    self.raw.replace(b'NoNewPrivileges=yes', b'NoNewPrivileges=no'),
                    self.raw.replace(b'LimitMEMLOCK=infinity', b'LimitMEMLOCK=8M'),
                    self.raw.replace(b'User=maxsong', b'User=root'),
                    self.raw.replace(b' --port 4400', b' --policy /tmp/policy --port 4400')]:
            with self.subTest(unit=hashlib.sha256(bad).hexdigest()):
                self.unit.write_bytes(bad)
                with self.assertRaises(ValueError):
                    m.prepare(self.unit, 'a' * 64, self.output)
                self.assertFalse(self.output.exists())

    def test_existing_outputs_symlinks_and_bad_digest_are_preserved(self):
        self.output.mkdir()
        marker = self.output / 'keep'
        marker.write_text('keep')
        with self.assertRaises(ValueError):
            m.prepare(self.unit, 'a' * 64, self.output)
        self.assertEqual(marker.read_text(), 'keep')
        for sha in ['A' * 64, 'a' * 63, 'x; touch injected']:
            with self.assertRaises(ValueError):
                m.prepare(self.unit, sha, self.root / 'new')
        link = self.root / 'linked.service'; link.symlink_to(self.unit)
        with self.assertRaises(ValueError):
            m.prepare(link, 'a' * 64, self.root / 'new')
        self.assertFalse((self.root / 'new').exists())


if __name__ == '__main__':
    unittest.main()
