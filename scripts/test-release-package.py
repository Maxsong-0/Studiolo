#!/usr/bin/env python3
"""校验运行包的生产程序门禁与整包验收门禁；只用临时虚构文件，不构建、不联网、不起服务。

覆盖两处真实代码片段：
  * `package-preview.sh` 的 `NODE_PRODUCTS`：生产构建的 Cargo JSON 判定：
    程序是否齐全、新增运行时 crate 是否真的被链接、有没有混入测试特征或测试产物。
  * `run-preview-tests.sh` 的 `BEGIN_PAYLOAD_INTEGRITY`：服务器原生验收前的整包门禁：
    条目类型、权限位、SHA256SUMS 与生产特征回执。
"""
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
# 与 package-preview.sh 的 linked 列表逐项对应。
LINKED = ["backup", "observability", "sync-transport", "host-config", "core", "vault-broker"]
PRODUCTS = [("studiolo-local-server", "local-server"), ("studiolo-maintain", "local-server"),
            ("studiolo-auth-admin", "auth-admin")]
RECEIPT = "\n".join(
    [f"product:{product} crate:{crate}" for product, crate in PRODUCTS]
    + ["crate:auth features:production"]
    + [f"crate:{crate} features:" for crate in sorted(LINKED + ["local-server", "auth-admin"])]) + "\n"


def artifact(crate, name, kind, features, executable=True):
    item = {"reason": "compiler-artifact", "profile": {"test": False},
            "manifest_path": f"{SOURCE}/crates/{crate}/Cargo.toml",
            "package_id": f"path+file://{SOURCE}/crates/{crate}#0.1.0",
            "target": {"name": name, "kind": [kind]}, "features": features}
    if executable:
        item["executable"] = f"/build-target/release/{name}"
    return item


def complete_rows():
    rows = [artifact("auth", "studiolo_auth", "lib", ["production"], executable=False)]
    rows += [artifact(crate, "studiolo_" + crate.replace("-", "_"), "lib", [], executable=False)
             for crate in LINKED]
    rows += [artifact(crate, product, "bin", ["production"] if crate == "auth-admin" else [])
             for product, crate in PRODUCTS]
    # 注册表依赖不在源码快照内，应被跳过而不是判为身份不明。
    rows.append({"reason": "compiler-artifact", "profile": {"test": False},
                 "manifest_path": "/usr/local/cargo/registry/src/serde-1.0.0/Cargo.toml",
                 "package_id": "registry+serde#1.0.0",
                 "target": {"name": "serde", "kind": ["lib"]}, "features": ["std"]})
    return rows + [{"reason": "build-finished", "success": True} for _ in PRODUCTS]


class ProductBuildGateTests(unittest.TestCase):
    def gate(self, rows):
        with tempfile.TemporaryDirectory(prefix="studiolo-products-") as directory:
            out = Path(directory)
            (out / "build-meta").mkdir()
            (out / "payload").mkdir()
            (out / "build-meta/products.jsonl").write_text("\n".join(map(json.dumps, rows)))
            code = PACKAGE.split("<<'NODE_PRODUCTS'\n", 1)[1].split("\nNODE_PRODUCTS", 1)[0]
            result = subprocess.run(["node", "-", str(out), SOURCE], input=code,
                                    text=True, capture_output=True, timeout=15)
            receipt = out / "payload/product-features.txt"
            return result.returncode, receipt.read_text() if receipt.exists() else None

    def without(self, predicate):
        return [row for row in complete_rows() if not predicate(row)]

    def test_complete_production_build_publishes_receipt_for_products_and_linked_crates(self):
        status, receipt = self.gate(complete_rows())
        self.assertEqual(status, 0)
        for product, crate in PRODUCTS:
            self.assertIn(f"product:{product} crate:{crate}\n", receipt)
        for crate in LINKED:
            self.assertIn(f"crate:{crate} features:\n", receipt)
        self.assertIn("crate:auth features:production\n", receipt)
        self.assertNotIn("serde", receipt)

    def test_every_published_product_is_mandatory(self):
        for product, _ in PRODUCTS:
            with self.subTest(product=product):
                rows = self.without(lambda row: row.get("target", {}).get("name") == product)
                self.assertEqual(self.gate(rows), (1, None))

    def test_newly_added_runtime_crate_must_be_linked_into_the_products(self):
        for crate in ["backup", "observability", "sync-transport", "host-config"]:
            with self.subTest(crate=crate):
                rows = self.without(lambda row: f"/crates/{crate}/" in row.get("manifest_path", ""))
                self.assertEqual(self.gate(rows), (1, None))

    def test_test_support_feature_on_any_workspace_crate_refuses_to_publish(self):
        for crate in ["auth", "backup", "local-server"]:
            with self.subTest(crate=crate):
                rows = complete_rows()
                for row in rows:
                    if f"/crates/{crate}/" in row.get("manifest_path", ""):
                        row["features"] = row["features"] + ["test-support"]
                self.assertEqual(self.gate(rows), (1, None))

    def test_identity_module_without_production_feature_refuses_to_publish(self):
        rows = complete_rows()
        for row in rows:
            if "/crates/auth/" in row.get("manifest_path", ""):
                row["features"] = []
        self.assertEqual(self.gate(rows), (1, None))

    def test_same_crate_with_diverging_features_between_builds_refuses_to_publish(self):
        rows = complete_rows()
        rows.append(artifact("auth", "studiolo_auth", "lib", [], executable=False))
        self.assertEqual(self.gate(rows), (1, None))

    def test_test_profile_artifact_inside_the_product_build_refuses_to_publish(self):
        rows = complete_rows()
        rows.insert(0, artifact("local-server", "startup_contract", "test", []))
        rows[0]["profile"]["test"] = True
        self.assertEqual(self.gate(rows), (1, None))

    def test_failed_or_incomplete_build_finished_refuses_to_publish(self):
        rows = complete_rows()
        rows[-1]["success"] = False
        self.assertEqual(self.gate(rows), (1, None))
        self.assertEqual(self.gate(complete_rows()[:-1]), (1, None))

    def test_product_built_from_another_crate_refuses_to_publish(self):
        rows = self.without(lambda row: row.get("target", {}).get("name") == "studiolo-maintain")
        rows.insert(0, artifact("supply", "studiolo-maintain", "bin", []))
        self.assertEqual(self.gate(rows), (1, None))


class PayloadIntegrityGateTests(unittest.TestCase):
    def setUp(self):
        # 目录名带空格：服务器上的独立发布目录必须能原样工作，不靠无空格路径侥幸通过。
        self.temp = tempfile.TemporaryDirectory(prefix="studiolo release payload ")
        self.addCleanup(self.temp.cleanup)
        self.payload = Path(self.temp.name) / "payload"
        (self.payload / "bin").mkdir(parents=True)
        (self.payload / "tests").mkdir()
        (self.payload / "SHA256SUMS").write_text("虚构整包摘要清单\n")
        (self.payload / "product-features.txt").write_text(RECEIPT)
        for product, _ in PRODUCTS:
            binary = self.payload / "bin" / product
            binary.write_text("#!/bin/sh\nexit 0\n")
            binary.chmod(0o755)
        self.fake = Path(self.temp.name) / "fake-tools"
        self.fake.mkdir()
        # 只替代本测试的系统工具；真实门禁不读取此环境变量。
        probe = self.fake / "sha256sum"
        probe.write_text('#!/bin/sh\ncase "${STUDIOLO_FIXTURE_SUMS:-}" in\n'
                         'bad) printf "SHA256SUMS: 校验失败\\n" >&2; exit 1 ;;\n*) exit 0 ;;\nesac\n')
        probe.chmod(0o755)

    def gate(self, sums=""):
        code = RUNNER.split("# BEGIN_PAYLOAD_INTEGRITY\n", 1)[1].split("# END_PAYLOAD_INTEGRITY", 1)[0]
        environment = {"PATH": str(self.fake) + os.pathsep + os.environ.get("PATH", "/usr/bin:/bin"),
                       "STUDIOLO_PAYLOAD": str(self.payload), "STUDIOLO_FIXTURE_SUMS": sums}
        return subprocess.run(["bash", "-c", "set -euo pipefail\n" + code], env=environment,
                              text=True, capture_output=True, timeout=15)

    def receipt(self, text):
        (self.payload / "product-features.txt").write_text(text)

    def test_intact_bundle_in_a_path_with_spaces_passes_every_gate(self):
        result = self.gate()
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_checksum_mismatch_refuses_the_acceptance_run(self):
        self.assertNotEqual(self.gate("bad").returncode, 0)

    def test_symlink_inside_the_bundle_refuses_the_acceptance_run(self):
        (self.payload / "tests/link").symlink_to(self.payload / "SHA256SUMS")
        self.assertNotEqual(self.gate().returncode, 0)

    def test_group_or_other_writable_entry_refuses_the_acceptance_run(self):
        for mode in [0o664, 0o666, 0o775]:
            with self.subTest(mode=oct(mode)):
                (self.payload / "SHA256SUMS").chmod(mode)
                self.assertNotEqual(self.gate().returncode, 0)
        (self.payload / "SHA256SUMS").chmod(0o644)
        self.assertEqual(self.gate().returncode, 0)

    def test_missing_empty_or_symlink_receipt_refuses_the_acceptance_run(self):
        path = self.payload / "product-features.txt"
        path.unlink()
        self.assertNotEqual(self.gate().returncode, 0)
        path.write_text("")
        self.assertNotEqual(self.gate().returncode, 0)
        path.unlink()
        path.symlink_to(self.payload / "SHA256SUMS")
        self.assertNotEqual(self.gate().returncode, 0)

    def test_receipt_missing_a_product_or_a_new_runtime_crate_refuses(self):
        for absent in [f"product:{product} crate:" for product, _ in PRODUCTS] \
                + [f"crate:{crate} features:" for crate in
                   ["backup", "observability", "sync-transport", "host-config"]]:
            with self.subTest(absent=absent):
                self.receipt("".join(line + "\n" for line in RECEIPT.splitlines()
                                     if not line.startswith(absent)))
                self.assertNotEqual(self.gate().returncode, 0)

    def test_receipt_without_production_identity_or_with_test_support_refuses(self):
        self.receipt(RECEIPT.replace("crate:auth features:production", "crate:auth features:"))
        self.assertNotEqual(self.gate().returncode, 0)
        self.receipt(RECEIPT.replace("crate:backup features:", "crate:backup features:test-support"))
        self.assertNotEqual(self.gate().returncode, 0)


if __name__ == "__main__":
    unittest.main()
