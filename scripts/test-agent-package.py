#!/usr/bin/env python3
"""用虚构 Cargo 输出和源码树校验打包门槛；不构建、不联网、不运行服务。"""
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

SCRIPTS = Path(__file__).resolve().parent
PACKAGE = (SCRIPTS / "package-agent.sh").read_text()
RUNNER = (SCRIPTS / "run-agent-tests.sh").read_text()
SOURCE_ROOT = "/home/maxsong/.local/share/studiolo-agent-releases/fixture/source"
REQUIRED = {
    "studiolo": ["architecture"],
    "vault-broker": ["linux_path_security"],
    "agent-server": ["mcp_contract", "mcp_transport_contract", "replica_mcp_contract", "http_text_contract",
                     "http_replica_contract", "index_cache_contract", "process_residency"],
    "supply": ["roots_contract", "signpost_contract", "http_signpost_contract",
               "agent_store_contract", "replica_contract", "index_cache_contract"],
    "vault-index": ["parse_cache_contract", "parse_cache_codec_contract", "parse_cache_links_contract"],
    "workbench": ["parse_cache_contract"],
}


def embedded(script, label):
    return script.split("<<'" + label + "'\n", 1)[1].split("\n" + label, 1)[0]


def run_script(runtime, source, *args):
    return subprocess.run([*runtime, "-", *map(str, args)], input=source, text=True,
                          capture_output=True, timeout=15)


def artifact(crate, target):
    return {"reason": "compiler-artifact", "profile": {"test": True},
            "manifest_path": f"{SOURCE_ROOT}/crates/{crate}/Cargo.toml",
            "package_id": f"path+file://{SOURCE_ROOT}/crates/{crate}#0.1.0",
            "target": {"name": target, "kind": ["test"]},
            "executable": f"/build-target/release/deps/{crate}_{target}-01234567"}


class CargoManifestTests(unittest.TestCase):
    def collect(self, items, success=True):
        with tempfile.TemporaryDirectory() as directory:
            out = Path(directory)
            (out / "build-meta").mkdir()
            (out / "payload").mkdir()
            records = items + [{"reason": "build-finished", "success": success}]
            (out / "build-meta/tests.jsonl").write_text("\n".join(map(json.dumps, records)))
            result = run_script(["node"], embedded(PACKAGE, "NODE_TESTS"), out, SOURCE_ROOT)
            listing = out / "payload/test-list"
            return result.returncode, listing.read_text().splitlines() if listing.exists() else []

    def complete(self):
        return [artifact(crate, target) for crate, targets in REQUIRED.items() for target in targets]

    def test_all_required_packages_and_targets_are_collected_once(self):
        items = self.complete()
        code, names = self.collect(items + [items[0]])
        self.assertEqual(code, 0)
        self.assertEqual(len(names), 19)
        self.assertEqual(len(set(names)), 19)

    def test_transport_gate_cannot_be_missing_or_satisfied_by_another_crate(self):
        items = [item for item in self.complete() if item["target"]["name"] != "mcp_transport_contract"]
        for replacement in [[], [artifact("supply", "mcp_transport_contract")]]:
            code, names = self.collect(items + replacement)
            self.assertNotEqual(code, 0)
            self.assertEqual(names, [])

    def test_same_target_from_another_package_does_not_fill_missing_cache_test(self):
        for crate, target in [("agent-server", "index_cache_contract"),
                              ("supply", "index_cache_contract"),
                              ("workbench", "parse_cache_contract"),
                              ("vault-index", "parse_cache_contract")]:
            with self.subTest(crate=crate):
                removed = artifact(crate, target)
                self.assertNotEqual(self.collect([item for item in self.complete() if item != removed])[0], 0)

    def test_library_target_does_not_replace_integration_test(self):
        items = self.complete()
        items[0]["target"]["kind"] = ["lib"]
        self.assertNotEqual(self.collect(items)[0], 0)

    def test_missing_http_target_and_incomplete_build_refuse_output(self):
        self.assertNotEqual(self.collect(self.complete(), success=False)[0], 0)
        items = [item for item in self.complete() if item["target"]["name"] != "http_text_contract"]
        code, names = self.collect(items)
        self.assertNotEqual(code, 0)
        self.assertEqual(names, [])

    def test_package_identity_cannot_change_between_targets(self):
        items = self.complete()
        items[3]["package_id"] = "other-package"
        self.assertNotEqual(self.collect(items)[0], 0)

    def test_one_executable_cannot_fill_two_packages(self):
        items = self.complete()
        items[1]["executable"] = items[0]["executable"]
        self.assertNotEqual(self.collect(items)[0], 0)

    def test_wrong_source_root_or_executable_path_refuses_output(self):
        for field, value in [("manifest_path", "/other/crates/studiolo/Cargo.toml"),
                             ("executable", "/build-target/release/../secrets")]:
            with self.subTest(field=field):
                items = self.complete()
                items[0][field] = value
                self.assertNotEqual(self.collect(items)[0], 0)


class SourceManifestTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name) / "source"
        self.root.mkdir()
        self.manifest = Path(self.temp.name) / "source-files"
        (self.root / "scripts").mkdir()
        (self.root / "scripts/probe.py").write_text("# 虚构脚本\n")
        self.names = ["scripts/probe.py"]
        self.write_manifest()

    def write_manifest(self):
        self.manifest.write_bytes(("\0".join(self.names) + "\0").encode())

    def package_check(self, *inputs):
        return run_script(["node"], embedded(PACKAGE, "NODE_SOURCE"), self.root,
                          self.manifest, *inputs).returncode

    def unpacked_check(self):
        return run_script([sys.executable, "-I", "-B"], embedded(RUNNER, "PY_SOURCE"),
                          self.root, self.manifest).returncode

    def test_exact_plain_source_passes_packaging_and_unpacked_check(self):
        self.assertEqual(self.package_check("scripts/probe.py"), 0)
        self.assertEqual(self.unpacked_check(), 0)

    def test_missing_explicit_input_refuses_packaging(self):
        self.assertNotEqual(self.package_check("scripts/untracked.py"), 0)

    def test_untracked_file_is_not_selected_and_extra_unpacked_file_refuses(self):
        (self.root / "scripts/private.txt").write_text("虚构未跟踪资料")
        self.assertEqual(self.package_check("scripts/probe.py"), 0)
        self.assertNotEqual(self.unpacked_check(), 0)

    def test_tracked_bytecode_and_cache_directory_refuse_packaging(self):
        for name in ["scripts/extra.pyc", "scripts/__pycache__/extra.txt"]:
            with self.subTest(name=name):
                file = self.root / name
                file.parent.mkdir(exist_ok=True)
                file.write_text("虚构缓存")
                self.names = ["scripts/probe.py", name]
                self.write_manifest()
                self.assertNotEqual(self.package_check("scripts/probe.py"), 0)

    def test_symlink_source_refuses_both_checks(self):
        (self.root / "scripts/probe.py").unlink()
        (self.root / "scripts/probe.py").symlink_to(self.manifest)
        self.assertNotEqual(self.package_check("scripts/probe.py"), 0)
        self.assertNotEqual(self.unpacked_check(), 0)

    def test_missing_source_and_duplicate_manifest_refuse(self):
        (self.root / "scripts/probe.py").unlink()
        self.assertNotEqual(self.unpacked_check(), 0)
        self.names *= 2
        self.write_manifest()
        self.assertNotEqual(self.package_check("scripts/probe.py"), 0)


if __name__ == "__main__":
    unittest.main()
