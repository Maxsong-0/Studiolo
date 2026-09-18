#!/usr/bin/env python3
"""校验运行包安装器的三段真实代码：整包校验、私有权限落盘、待重启登记。

只用临时虚构目录，不接触真实发布目录，不调用 systemctl，也不需要 root。
安装器在 PATH 上被放置了会立即失败并留痕的 systemctl 替身：
只要有任何一段代码试图启停共享服务，对应用例就会失败。
"""
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parent.parent
INSTALLER = (ROOT / "deploy/preview/install-runtime-package.sh").read_text()
RELEASE = "m3-t049-20260917-runtime01"
SOURCE_ROOT = f"/home/maxsong/.local/share/studiolo-preview/releases/{RELEASE}/source"
PRODUCTS = [("studiolo-local-server", "local-server"), ("studiolo-maintain", "local-server"),
            ("studiolo-auth-admin", "auth-admin")]
LINKED = ["backup", "observability", "sync-transport", "host-config", "core", "vault-broker"]
RECEIPT = "\n".join(
    [f"product:{product} crate:{crate}" for product, crate in PRODUCTS]
    + ["crate:auth features:production"]
    + [f"crate:{crate} features:" for crate in sorted(LINKED)]) + "\n"


def block(label):
    return INSTALLER.split(f"# BEGIN_{label}\n", 1)[1].split(f"# END_{label}", 1)[0]


class InstallerHarness(unittest.TestCase):
    def setUp(self):
        # 目录名带空格：独立发布目录含空格时安装器必须照常工作。
        self.temp = tempfile.TemporaryDirectory(prefix="studiolo install fixture ")
        self.addCleanup(self.temp.cleanup)
        self.base = Path(self.temp.name)
        self.payload = self.base / "payload"
        (self.payload / "bin").mkdir(parents=True)
        (self.payload / "tests").mkdir()
        (self.payload / "SHA256SUMS").write_text("虚构整包摘要清单\n")
        (self.payload / "product-features.txt").write_text(RECEIPT)
        (self.payload / "expected-source-root").write_text(SOURCE_ROOT + "\n")
        (self.payload / "tests/startup_contract").write_text("#!/bin/sh\nexit 0\n")
        (self.payload / "tests/startup_contract").chmod(0o755)
        for product, _ in PRODUCTS:
            binary = self.payload / "bin" / product
            binary.write_text("#!/bin/sh\nexit 0\n")
            binary.chmod(0o755)
        self.fake = self.base / "fake-tools"
        self.fake.mkdir()
        self.marker = self.base / "systemctl-was-called"
        sums = self.fake / "sha256sum"
        sums.write_text('#!/bin/sh\ncase "${STUDIOLO_FIXTURE_SUMS:-}" in\n'
                        'bad) exit 1 ;;\n*) exit 0 ;;\nesac\n')
        sums.chmod(0o755)
        probe = self.fake / "systemctl"
        probe.write_text(f'#!/bin/sh\necho "$@" >> "{self.marker}"\nexit 97\n')
        probe.chmod(0o755)

    def run_block(self, labels, call, sums=""):
        code = "set -Eeuo pipefail\n" + "".join(block(label) for label in labels) + "\n" + call + "\n"
        environment = {"PATH": str(self.fake) + os.pathsep + os.environ.get("PATH", "/usr/bin:/bin"),
                       "HOME": str(self.base), "STUDIOLO_FIXTURE_SUMS": sums}
        result = subprocess.run(["bash", "-c", code], env=environment, text=True,
                                capture_output=True, timeout=20)
        self.assertFalse(self.marker.exists(), "安装器不得启停任何共享服务")
        return result


class PackageVerifyTests(InstallerHarness):
    def verify(self, release=RELEASE, payload=None, sums=""):
        target = str(self.payload if payload is None else payload)
        return self.run_block(["LOOSE_MODES", "PACKAGE_VERIFY"],
                              f'verify_package "{target}" "{release}"', sums)

    def test_intact_package_with_expected_release_name_is_accepted(self):
        result = self.verify()
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_relative_path_missing_directory_or_bad_release_name_is_rejected(self):
        self.assertNotEqual(self.verify(payload="payload").returncode, 0)
        self.assertNotEqual(self.verify(payload=self.base / "absent").returncode, 0)
        for release in ["../escape", "-leading-dash", "名字", ""]:
            with self.subTest(release=release):
                self.assertNotEqual(self.verify(release=release).returncode, 0)

    def test_missing_production_program_is_rejected(self):
        for product, _ in PRODUCTS:
            with self.subTest(product=product):
                path = self.payload / "bin" / product
                path.unlink()
                self.assertNotEqual(self.verify().returncode, 0)
                path.write_text("#!/bin/sh\nexit 0\n")
                path.chmod(0o755)

    def test_non_executable_or_symlinked_program_is_rejected(self):
        path = self.payload / "bin/studiolo-maintain"
        path.chmod(0o600)
        self.assertNotEqual(self.verify().returncode, 0)
        path.unlink()
        path.symlink_to(self.payload / "bin/studiolo-local-server")
        self.assertNotEqual(self.verify().returncode, 0)

    def test_checksum_mismatch_or_missing_manifest_is_rejected(self):
        self.assertNotEqual(self.verify(sums="bad").returncode, 0)
        (self.payload / "SHA256SUMS").write_text("")
        self.assertNotEqual(self.verify().returncode, 0)
        (self.payload / "SHA256SUMS").unlink()
        self.assertNotEqual(self.verify().returncode, 0)

    def test_group_or_other_writable_entry_is_rejected(self):
        (self.payload / "product-features.txt").chmod(0o666)
        self.assertNotEqual(self.verify().returncode, 0)

    def test_receipt_gaps_and_test_support_contamination_are_rejected(self):
        path = self.payload / "product-features.txt"
        path.write_text(RECEIPT.replace("crate:sync-transport features:", "crate:other features:"))
        self.assertNotEqual(self.verify().returncode, 0)
        path.write_text(RECEIPT.replace("product:studiolo-maintain crate:local-server", ""))
        self.assertNotEqual(self.verify().returncode, 0)
        path.write_text(RECEIPT.replace("crate:auth features:production", "crate:auth features:"))
        self.assertNotEqual(self.verify().returncode, 0)
        path.write_text(RECEIPT + "crate:extra features:test-support\n")
        self.assertNotEqual(self.verify().returncode, 0)

    def test_source_root_that_does_not_match_the_release_name_is_rejected(self):
        path = self.payload / "expected-source-root"
        path.write_text("/home/maxsong/.local/share/studiolo-preview/releases/other/source\n")
        self.assertNotEqual(self.verify().returncode, 0)
        path.unlink()
        self.assertNotEqual(self.verify().returncode, 0)


class PrivateLayoutTests(InstallerHarness):
    def install(self, root=None, sums=""):
        target = str(self.base / "releases" if root is None else root)
        return self.run_block(["LOOSE_MODES", "PRIVATE_LAYOUT"],
                              f'install_private_tree "{target}" "{RELEASE}" "{self.payload}"', sums)

    def test_installed_tree_is_private_with_executable_programs_and_tests(self):
        result = self.install()
        self.assertEqual(result.returncode, 0, result.stderr)
        target = Path(result.stdout.strip())
        self.assertEqual(target, self.base / "releases" / RELEASE)
        self.assertEqual((self.base / "releases").stat().st_mode & 0o777, 0o700)
        self.assertEqual(target.stat().st_mode & 0o777, 0o700)
        self.assertEqual((target / "SHA256SUMS").stat().st_mode & 0o777, 0o600)
        self.assertEqual((target / "bin/studiolo-maintain").stat().st_mode & 0o777, 0o700)
        self.assertEqual((target / "tests/startup_contract").stat().st_mode & 0o777, 0o700)
        for entry in target.rglob("*"):
            self.assertEqual(entry.stat().st_mode & 0o077, 0, entry)
        self.assertEqual((target / "product-features.txt").read_text(), RECEIPT)

    def test_release_directories_are_immutable_and_leftover_staging_blocks_install(self):
        self.assertEqual(self.install().returncode, 0)
        self.assertNotEqual(self.install().returncode, 0)
        root = self.base / "second"
        root.mkdir(mode=0o700)
        (root / f".{RELEASE}.partial").mkdir(mode=0o700)
        self.assertNotEqual(self.install(root).returncode, 0)

    def test_copy_that_no_longer_matches_the_manifest_never_reaches_the_release_directory(self):
        # 恢复契约：复制中断或磁盘写满会让副本与随包摘要不一致；发布目录不可变，
        # 因此必须在改名就位前发现，且不得留下半截发布目录。
        result = self.install(sums="bad")
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse((self.base / "releases" / RELEASE).exists())
        # 暂存目录被刻意保留供人工确认，并因此挡住下一次同名安装。
        self.assertTrue((self.base / "releases" / f".{RELEASE}.partial").is_dir())
        self.assertNotEqual(self.install().returncode, 0)

    def test_symlinked_release_root_is_rejected(self):
        root = self.base / "linked"
        root.symlink_to(self.base / "releases")
        self.assertNotEqual(self.install(root).returncode, 0)


class RestartRegisterTests(InstallerHarness):
    def register(self, *units, registry=None):
        path = str(self.base / "state/pending-restart.log" if registry is None else registry)
        names = " ".join(f'"{unit}"' for unit in units)
        return self.run_block(["RESTART_REGISTER"],
                              f'register_pending_restart "{path}" "{RELEASE}" {names}')

    def test_units_are_only_registered_and_appended_never_restarted(self):
        registry = self.base / "state/pending-restart.log"
        self.assertEqual(self.register("studiolo-preview.service").returncode, 0)
        self.assertEqual(self.register("studiolo-access.service").returncode, 0)
        lines = registry.read_text().splitlines()
        self.assertEqual(len(lines), 2)
        self.assertTrue(lines[0].endswith(f"待重启 studiolo-preview.service 发布 {RELEASE}"))
        self.assertTrue(lines[1].endswith(f"待重启 studiolo-access.service 发布 {RELEASE}"))
        self.assertEqual(registry.stat().st_mode & 0o777, 0o600)
        self.assertEqual(registry.parent.stat().st_mode & 0o777, 0o700)

    def test_no_units_illegal_unit_name_or_symlinked_registry_is_rejected(self):
        self.assertNotEqual(self.register().returncode, 0)
        for unit in ["studiolo-preview", "../escape.service", "studiolo preview.service"]:
            with self.subTest(unit=unit):
                self.assertNotEqual(self.register(unit).returncode, 0)
        link = self.base / "link.log"
        link.symlink_to(self.base / "absent.log")
        self.assertNotEqual(self.register("studiolo-preview.service", registry=link).returncode, 0)


class LinuxEndToEndTests(InstallerHarness):
    """在真实 Linux 上整脚本跑一遍；本机没有 sha256sum 时明确跳过，不冒称已验证。"""

    def setUp(self):
        if sys.platform != "linux" or shutil.which("sha256sum") is None:
            self.skipTest("整脚本端到端安装需要带 sha256sum 的 Linux")
        super().setUp()
        (self.payload / "SHA256SUMS").unlink()
        listing = sorted(str(path.relative_to(self.payload)) for path in self.payload.rglob("*")
                         if path.is_file())
        sums = subprocess.run(["sha256sum", *listing], cwd=self.payload, text=True,
                              capture_output=True, timeout=20, check=True).stdout
        (self.payload / "SHA256SUMS").write_text(sums)

    def install(self, release=RELEASE):
        root = self.base / "发布根 目录"
        registry = self.base / "state/pending-restart.log"
        environment = dict(os.environ, HOME=str(self.base), STUDIOLO_RELEASES_ROOT=str(root),
                           STUDIOLO_RESTART_REGISTRY=str(registry))
        return subprocess.run(["bash", str(ROOT / "deploy/preview/install-runtime-package.sh"),
                               str(self.payload), release], env=environment, text=True,
                              capture_output=True, timeout=60)

    def test_full_script_installs_privately_registers_restarts_and_refuses_reinstall(self):
        result = self.install()
        self.assertEqual(result.returncode, 0, result.stderr)
        target = self.base / "发布根 目录" / RELEASE
        self.assertEqual(target.stat().st_mode & 0o777, 0o700)
        for entry in target.rglob("*"):
            self.assertEqual(entry.stat().st_mode & 0o077, 0, entry)
        self.assertEqual((target / "bin/studiolo-maintain").stat().st_mode & 0o777, 0o700)
        registry = (self.base / "state/pending-restart.log").read_text()
        self.assertIn(f"待重启 studiolo-preview.service 发布 {RELEASE}", registry)
        self.assertIn(f"待重启 studiolo-access.service 发布 {RELEASE}", registry)
        self.assertFalse(self.marker.exists(), "安装器不得启停任何共享服务")
        # 发布目录不可变：同名重装必须被拒绝。
        self.assertNotEqual(self.install().returncode, 0)

    def test_full_script_refuses_a_bundle_whose_bytes_no_longer_match_the_manifest(self):
        (self.payload / "product-features.txt").write_text(RECEIPT + "crate:extra features:\n")
        self.assertNotEqual(self.install().returncode, 0)
        self.assertFalse((self.base / "发布根 目录" / RELEASE).exists())


class InstallerBoundaryTests(unittest.TestCase):
    def test_installer_never_touches_systemd_or_system_directories(self):
        body = "\n".join(line for line in INSTALLER.splitlines() if not line.lstrip().startswith("#"))
        for forbidden in ["systemctl", "systemd-run", "daemon-reload", "runuser", "sudo",
                          "/etc/systemd", "journalctl"]:
            self.assertNotIn(forbidden, body, forbidden)

    def test_installer_refuses_root_and_documents_the_pending_restart_rule(self):
        self.assertIn('if [[ "$EUID" == 0 ]]; then', INSTALLER)
        self.assertIn("绝不 systemctl start/stop/restart/reload", INSTALLER)
        self.assertIn("register_pending_restart", INSTALLER)


if __name__ == "__main__":
    unittest.main()
