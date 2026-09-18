#!/usr/bin/env python3
"""执行真实 Cargo JSON 收集片段与原生产品门禁；只用临时虚构文件，不构建、不联网。"""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parent.parent
PACKAGE = (ROOT / "scripts/package-preview.sh").read_text()
RUNNER = (ROOT / "scripts/run-preview-tests.sh").read_text()
SOURCE = "/home/maxsong/.local/share/studiolo-preview/releases/fixture/source"
# 与 package-preview.sh 的 required 列表逐项对应；末项刻意与前一项同 crate，
# 便于用最后一行验证「同一 crate 不能在不同目标间改变包身份」。
REQUIRED = [("studiolo", "architecture"), ("vault-broker", "linux_path_security"),
            ("local-server", "startup_contract"), ("auth-admin", "cli_process"),
            ("local-server", "maintenance_contract"), ("backup", "contract"),
            ("observability", "health_contract"), ("observability", "quota_contract"),
            ("observability", "alert_contract"), ("observability", "deadman_contract"),
            ("observability", "store_contract"), ("observability", "whitelist_contract"),
            ("host-config", "sync_transport_contract"), ("host-config", "alerting_contract")]
# sync-transport 的契约写在 lib 目标里，没有独立集成测试目标。
LIBRARIES = [("sync-transport", "studiolo_sync_transport")]
PRODUCTS = {"studiolo-local-server": ("STUDIOLO_TEST_SERVER_BIN", "runtime-libraries.txt"),
            "studiolo-auth-admin": ("STUDIOLO_TEST_AUTH_ADMIN_BIN", "auth-admin-runtime-libraries.txt"),
            "studiolo-maintain": ("STUDIOLO_TEST_MAINTAIN_BIN", "maintain-runtime-libraries.txt")}


def artifact(crate, target, kind="test"):
    return {"reason": "compiler-artifact", "profile": {"test": True},
            "manifest_path": f"{SOURCE}/crates/{crate}/Cargo.toml",
            "package_id": f"path+file://{SOURCE}/crates/{crate}#0.1.0",
            "target": {"name": target, "kind": [kind]},
            "executable": f"/build-target/release/deps/{crate}_{target}-01234567"}


class CargoCollectionTests(unittest.TestCase):
    def complete(self):
        return ([artifact(*target) for target in REQUIRED]
                + [artifact(crate, name, "lib") for crate, name in LIBRARIES])

    def collect(self, items, success=True):
        with tempfile.TemporaryDirectory(dir="/tmp", prefix="studiolo-preview-json-") as directory:
            out = Path(directory)
            (out / "build-meta").mkdir()
            (out / "payload").mkdir()
            rows = items + [{"reason": "build-finished", "success": success}]
            (out / "build-meta/tests.jsonl").write_text("\n".join(map(json.dumps, rows)))
            code = PACKAGE.split("<<'NODE'\n", 1)[1].split("\nNODE", 1)[0]
            result = subprocess.run(["node", "-", str(out), SOURCE], input=code,
                                    text=True, capture_output=True, timeout=15)
            listing = out / "payload/test-list"
            return result.returncode, listing.read_text().splitlines() if listing.exists() else []

    def test_complete_build_includes_maintenance_and_deduplicates_identical_artifacts(self):
        items = self.complete()
        expected = len(REQUIRED) + len(LIBRARIES)
        status, listing = self.collect(items + [items[-1]])
        self.assertEqual(status, 0)
        self.assertEqual(len(listing), expected)
        self.assertEqual(len(set(listing)), expected)
        self.assertTrue(any("maintenance_contract" in name for name in listing))

    def test_each_required_target_is_mandatory_and_failure_publishes_no_list(self):
        for missing in REQUIRED:
            with self.subTest(missing=missing):
                rows = [item for item in self.complete() if item != artifact(*missing)]
                status, listing = self.collect(rows)
                self.assertNotEqual(status, 0)
                self.assertEqual(listing, [])
        self.assertNotEqual(self.collect(self.complete(), success=False)[0], 0)

    def test_library_only_contract_crate_cannot_be_dropped_or_faked_by_integration_target(self):
        for crate, name in LIBRARIES:
            with self.subTest(crate=crate):
                rows = [item for item in self.complete() if item != artifact(crate, name, "lib")]
                self.assertNotEqual(self.collect(rows)[0], 0)
                self.assertNotEqual(self.collect(rows + [artifact(crate, name)])[0], 0)

    def test_wrong_package_or_library_cannot_supply_maintenance_contract(self):
        for replacement in [artifact("supply", "maintenance_contract"),
                            artifact("local-server", "maintenance_contract")]:
            if "local-server" in replacement["manifest_path"]:
                replacement["target"]["kind"] = ["lib"]
            rows = [item for item in self.complete()
                    if item["target"]["name"] != "maintenance_contract"]
            status, listing = self.collect(rows + [replacement])
            self.assertNotEqual(status, 0)
            self.assertEqual(listing, [])

    def test_one_executable_cannot_fill_two_distinct_targets(self):
        rows = self.complete()
        rows[-1]["executable"] = rows[2]["executable"]
        self.assertNotEqual(self.collect(rows)[0], 0)

    def test_wrong_source_executable_or_package_identity_is_rejected(self):
        # 改写 maintenance_contract 这一行：local-server 另有 startup_contract，
        # 因此包身份被改写时必然与同 crate 的兄弟目标冲突。
        index = REQUIRED.index(("local-server", "maintenance_contract"))
        for key, value in [("manifest_path", "/other/crates/local-server/Cargo.toml"),
                           ("executable", "/build-target/release/../private"),
                           ("package_id", "different-local-server-package")]:
            with self.subTest(key=key):
                rows = self.complete()
                rows[index][key] = value
                self.assertNotEqual(self.collect(rows)[0], 0)


class NativeProductGateTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(dir="/tmp", prefix="studiolo native package ")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root / "bin").mkdir()
        self.fake = self.root / "fake-tools"
        self.fake.mkdir()
        for binary, (_, runtime) in PRODUCTS.items():
            self.executable(self.root / "bin" / binary)
            (self.root / runtime).write_text("虚构打包时运行库清单\n")
        # 只替代本测试的系统工具；真实产品门禁不读取此环境变量。
        probe = self.fake / "ldd"
        probe.write_text('#!/bin/sh\ncase "${STUDIOLO_FIXTURE_RUNTIME:-}" in\n'
                         'exit) exit 1 ;;\nmissing) printf "libfixture.so => not found\\n" ;;\n'
                         '*) printf "libfixture.so => /lib/libfixture.so\\n" ;;\nesac\n')
        probe.chmod(0o755)

    def executable(self, path):
        path.write_text("#!/bin/sh\nexit 0\n")
        path.chmod(0o755)

    def gate(self, runtime=""):
        code = RUNNER.split("# BEGIN_NATIVE_PRODUCTS\n", 1)[1].split("# END_NATIVE_PRODUCTS", 1)[0]
        values = " ".join('"$' + key + '"' for key, _ in PRODUCTS.values())
        code = "set -euo pipefail\n" + code + '\nprintf "%s\\n" ' + values + "\n"
        environment = {"PATH": str(self.fake) + os.pathsep + os.environ.get("PATH", "/usr/bin:/bin"),
                       "STUDIOLO_PAYLOAD": str(self.root), "STUDIOLO_FIXTURE_RUNTIME": runtime}
        environment.update({key: "/untrusted/old-product" for key, _ in PRODUCTS.values()})
        return subprocess.run(["bash", "-c", code], env=environment, text=True,
                              capture_output=True, timeout=10)

    def test_verified_bundle_overrides_all_three_test_binary_paths(self):
        result = self.gate()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.splitlines(), [str(self.root / "bin" / name) for name in PRODUCTS])

    def test_missing_product_is_rejected_including_maintenance(self):
        for name in PRODUCTS:
            path = self.root / "bin" / name
            path.unlink()
            self.assertNotEqual(self.gate().returncode, 0)
            self.executable(path)

    def test_symlink_or_nonexecutable_product_is_rejected(self):
        path = self.root / "bin/studiolo-maintain"
        path.chmod(0o600)
        self.assertNotEqual(self.gate().returncode, 0)
        path.unlink()
        path.symlink_to(self.root / "bin/studiolo-local-server")
        self.assertNotEqual(self.gate().returncode, 0)

    def test_missing_empty_or_symlink_runtime_receipt_is_rejected(self):
        path = self.root / "maintain-runtime-libraries.txt"
        path.unlink()
        self.assertNotEqual(self.gate().returncode, 0)
        path.write_text("")
        self.assertNotEqual(self.gate().returncode, 0)
        path.unlink()
        path.symlink_to(self.root / "runtime-libraries.txt")
        self.assertNotEqual(self.gate().returncode, 0)

    def test_loader_failure_or_missing_shared_library_is_rejected(self):
        for runtime in ["exit", "missing"]:
            self.assertNotEqual(self.gate(runtime).returncode, 0)


class PackagingWiringTests(unittest.TestCase):
    def test_maintenance_build_install_runtime_and_full_checksum_are_required(self):
        self.assertIn("-p studiolo-local-server --features studiolo-auth/production --bin studiolo-maintain", PACKAGE)
        self.assertIn("install -m 755 /build-target/release/studiolo-maintain /out/bin/", PACKAGE)
        self.assertIn("ldd /out/bin/studiolo-maintain > /out/maintain-runtime-libraries.txt", PACKAGE)
        self.assertLess(PACKAGE.index("install -m 755 /build-target/release/studiolo-maintain"),
                        PACKAGE.index("find . -type f -print0"))
        self.assertIn('sha256sum --check SHA256SUMS', RUNNER)

    def test_known_process_contracts_use_test_only_relocation_with_cargo_fallback(self):
        for relative in ["project_operations/process.rs", "cargo_process_contract.rs", "rust_process_contract.rs"]:
            text = (ROOT / "crates/local-server/tests" / relative).read_text()
            self.assertIn('var_os("STUDIOLO_TEST_SERVER_BIN")', text)
            self.assertIn('env_remove("STUDIOLO_TEST_SERVER_BIN")', text)
            self.assertNotIn('Command::new(env!("CARGO_BIN_EXE_', text)
        text = (ROOT / "crates/local-server/tests/maintenance/terminal_process.rs").read_text()
        self.assertIn('var_os("STUDIOLO_TEST_MAINTAIN_BIN")', text)
        self.assertIn('env!("CARGO_BIN_EXE_studiolo-maintain")', text)
        for source in (ROOT / "crates/local-server/src").rglob("*.rs"):
            self.assertNotIn("STUDIOLO_TEST_MAINTAIN_BIN", source.read_text())


if __name__ == "__main__":
    unittest.main()
