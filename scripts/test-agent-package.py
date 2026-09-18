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



class CompileTimeInputsTests(unittest.TestCase):
    """随包测试在编译期读的文件必须在源码白名单里。

    运行包只携带 `STUDIOLO_INPUTS` 点名的路径；随包 crate 的测试若 `include_str!` 了白名单外的文件，
    CI（整仓检出）一路全绿，而 `package-agent.sh` 在 `cargo test --no-run` 处编译失败——只有真打包那天才发现
    （2026-09-18 T-076：workbench 两份契约测试读 `examples/`，自 T-041 起打包一直是坏的）。
    """

    def test_every_include_macro_of_packaged_crates_is_covered_by_the_whitelist(self):
        import re
        root = SCRIPTS.parent
        block = lambda name: PACKAGE.split(name + "=(\n", 1)[1].split("\n)", 1)[0]
        inputs = [word for line in block("STUDIOLO_INPUTS").splitlines()
                  if not line.strip().startswith("#") for word in line.split()]
        crates = ["crates/" + (name.removeprefix("studiolo-") if name != "studiolo" else name)
                  for name in re.findall(r"-p (studiolo(?:-[a-z-]+)?)", block("STUDIOLO_TEST_PACKAGES"))]
        self.assertGreaterEqual(len(crates), 9)
        macro = re.compile(r'include_(?:str|bytes)!\(\s*"([^"]+)"\s*,?\s*\)')
        checked, uncovered = 0, []
        for crate in crates:
            self.assertTrue((root / crate).is_dir(), crate)
            for source in sorted((root / crate).rglob("*.rs")):
                for literal in macro.findall(source.read_text(encoding="utf-8")):
                    target = (source.parent / literal).resolve()
                    relative = target.relative_to(root.resolve()).as_posix()
                    checked += 1
                    covered = any(relative == entry or relative.startswith(entry + "/") for entry in inputs)
                    if not covered or not target.is_file():
                        uncovered.append(f"{source.relative_to(root).as_posix()} -> {relative}")
        self.assertGreaterEqual(checked, 5, "守卫没有扫到任何 include 宏，正则或目录推导失效")
        self.assertEqual(uncovered, [], "随包测试编译期依赖的文件不在 package-agent.sh 的源码白名单内")


if __name__ == "__main__":
    unittest.main()
