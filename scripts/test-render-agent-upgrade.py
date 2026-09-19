#!/usr/bin/env python3
"""render-agent-upgrade.py 回归：虚构运行包，不联网、不执行渲染产物。"""
import hashlib
import importlib.util
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

sys.dont_write_bytecode = True
SCRIPTS = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("render_upgrade", SCRIPTS / "render-agent-upgrade.py")
render_upgrade = importlib.util.module_from_spec(spec)
spec.loader.exec_module(render_upgrade)
OLD_BINARY, OLD_UNIT = "a" * 64, "b" * 64


def payload(root, template_sha=None, release="m9-fixture-01"):
    (root / "bin").mkdir(parents=True)
    (root / "bin/studiolo-agent").write_bytes(b"fixture-binary")
    (root / "studiolo-agent-system.service").write_text("[Service]\n")
    (root / "source-files").write_bytes(b"Cargo.toml\0")
    (root / "expected-source-root").write_text(f"{render_upgrade.ROOT}{release}/source\n")
    sha = template_sha or hashlib.sha256(render_upgrade.TEMPLATE.read_bytes()).hexdigest()
    (root / "SOURCE_SHA256SUMS").write_text(f"{sha}  ./{render_upgrade.TEMPLATE_NAME}\n")
    return root


class RenderTests(unittest.TestCase):
    def run_main(self, root, *extra, old_binary=OLD_BINARY, old_release="m8-fixture-00"):
        out = root.parent / "installer.sh"
        code = render_upgrade.main([str(root), "--old-release", old_release, "--old-binary-sha", old_binary,
                                    "--old-unit-sha", OLD_UNIT, "--output", str(out), *extra])
        return code, out

    def test_all_placeholders_are_bound_and_the_result_is_valid_bash(self):
        with tempfile.TemporaryDirectory() as directory:
            code, out = self.run_main(payload(Path(directory) / "payload"))
            self.assertEqual(code, 0)
            text = out.read_text()
            self.assertNotRegex(text, r"@[A-Z_]+@")
            self.assertIn("old_name=m8-fixture-00\n", text)
            self.assertIn(f"old_binary_sha={OLD_BINARY}\n", text)
            self.assertIn(f"old_unit_sha={OLD_UNIT}\n", text)
            self.assertIn("studiolo-agent-releases/m9-fixture-01\n", text)
            self.assertIn("binary_sha=" + hashlib.sha256(b"fixture-binary").hexdigest() + "\n", text)
            self.assertEqual(out.stat().st_mode & 0o777, 0o600)
            self.assertEqual(subprocess.run(["bash", "-n", str(out)], timeout=15).returncode, 0)

    def test_bad_baseline_same_release_and_foreign_template_are_refused(self):
        cases = [dict(old_binary="A" * 64), dict(old_binary="a" * 63), dict(old_release="m9-fixture-01"),
                 dict(old_release="../escape"), dict(old_binary=hashlib.sha256(b"fixture-binary").hexdigest())]
        for case in cases:
            with self.subTest(case=case), tempfile.TemporaryDirectory() as directory:
                code, out = self.run_main(payload(Path(directory) / "payload"), **case)
                self.assertEqual(code, 1)
                self.assertFalse(out.exists())
        with tempfile.TemporaryDirectory() as directory:
            code, out = self.run_main(payload(Path(directory) / "payload", template_sha="c" * 64))
            self.assertEqual(code, 1)
            self.assertFalse(out.exists())

    def test_existing_output_is_never_overwritten(self):
        with tempfile.TemporaryDirectory() as directory:
            root = payload(Path(directory) / "payload")
            (root.parent / "installer.sh").write_text("keep")
            code, out = self.run_main(root)
            self.assertEqual(code, 1)
            self.assertEqual(out.read_text(), "keep")

    def test_template_no_longer_hardcodes_one_upgrade_hop(self):
        text = render_upgrade.TEMPLATE.read_text()
        for placeholder in ("@OLD_RELEASE@", "@OLD_BINARY_SHA@", "@OLD_UNIT_SHA@"):
            self.assertIn(placeholder, text)
        self.assertNotRegex(text, r"m3-t036-20260910-replica01|[0-9a-f]{64}")


if __name__ == "__main__":
    unittest.main()
