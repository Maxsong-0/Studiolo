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
                     "http_replica_contract", "index_cache_contract", "process_residency", "policy_contract"],
    "policy": ["policy_store_contract", "view_contract", "audit_contract"],
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
        self.assertEqual(len(names), 23)
        self.assertEqual(len(set(names)), 23)

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



class DeployAllowlistTests(unittest.TestCase):
    """部署模板里的 --allow 集合必须与 replica_common.ALLOW 逐项相等。

    Mac 捕获端按 ALLOW 生成清单，agent 启动时要求清单的 allow 与 --allow 严格相等
    （crates/supply/src/replica/verify.rs 的 manifest_allowlist）；两边任一不同，服务即拒绝全部内容。
    2026-09-22 T-101 排除 项目/Semiment-Docs 时只改了 ALLOW，五份模板仍是 24 项——
    按模板部署就会启动失败。模板清单自动发现，不硬编码：硬编码的文件清单正是反复漏改的根源。
    """

    def test_every_deploy_template_allows_exactly_the_replica_allowlist(self):
        import re
        root = SCRIPTS.parent
        sys.path.insert(0, str(SCRIPTS))
        from replica_common import ALLOW
        token = re.compile(r'--allow (?:"([^"]+)"|(\S+))')
        checked, mismatched = 0, []
        for path in sorted((root / "deploy").rglob("*")):
            if not path.is_file() or path.suffix in {".py", ".md"}:
                continue
            text = path.read_text(encoding="utf-8", errors="replace")
            found = [quoted or bare for quoted, bare in token.findall(text)]
            if not found:
                continue
            checked += 1
            if sorted(found) != sorted(ALLOW):
                mismatched.append(f"{path.relative_to(root).as_posix()}: {len(found)} 项")
        self.assertGreaterEqual(checked, 5, "没扫到任何带 --allow 的部署模板，发现逻辑失效")
        self.assertEqual(mismatched, [], f"部署模板的 --allow 与 ALLOW（{len(ALLOW)} 项）不一致")


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
        # 扫全部 crate，不只 STUDIOLO_TEST_PACKAGES 点名的那几个：被点名的包会把**依赖**一起编译，
        # 依赖里的 include 宏同样决定整包能否构建。2026-09-22 T-101 打包失败即此——
        # runner-adapter 只是 studiolo 的依赖、不在测试包列表里，它 include_bytes! 的
        # deploy/runner/{launch,rg_probe}.py 不在白名单内，守卫却扫不到。
        crates = sorted("crates/" + d.name for d in (root / "crates").iterdir()
                        if (d / "Cargo.toml").is_file())
        self.assertGreaterEqual(len(crates), 9)
        # 从左到右一次扫：注释、原始字符串、字符串、字符字面量整段吞掉，只有落在代码位置的
        # include 宏才算。架构守卫的用例把 `include_str!("secret.txt")` 写在字符串里，不是真依赖。
        token = re.compile(r'//[^\n]*|/\*[\s\S]*?\*/|r(#*)"[\s\S]*?"\1|b?"(?:\\.|[^"\\])*"'
                           r"|b?'(?:\\.|[^'\\])'|include_(?:str|bytes)!\(\s*\"([^\"]+)\"\s*,?\s*\)")
        targets = lambda text: [m[2] for m in token.finditer(text) if m[2] is not None]
        self.assertEqual(targets('a(); include_str!("x") // include_str!("y")\n"include_str!(\\"z\\")"'), ["x"])
        self.assertEqual(targets('r#"include_bytes!("k")"#; let c = \'"\'; include_bytes!("w")'), ["w"])
        checked, uncovered = 0, []
        for crate in crates:
            self.assertTrue((root / crate).is_dir(), crate)
            for source in sorted((root / crate).rglob("*.rs")):
                for literal in targets(source.read_text(encoding="utf-8")):
                    target = (source.parent / literal).resolve()
                    relative = target.relative_to(root.resolve()).as_posix()
                    checked += 1
                    covered = any(relative == entry or relative.startswith(entry + "/") for entry in inputs)
                    if not covered or not target.is_file():
                        uncovered.append(f"{source.relative_to(root).as_posix()} -> {relative}")
        self.assertGreaterEqual(checked, 5, "守卫没有扫到任何 include 宏，正则或目录推导失效")
        self.assertEqual(uncovered, [], "随包测试编译期依赖的文件不在 package-agent.sh 的源码白名单内")

    def test_directory_entries_of_the_whitelist_are_known_to_the_node_check(self):
        """`STUDIOLO_INPUTS` 里的目录条目必须同时登记进 node 校验的 directories 集合。

        脚本有两份清单：INPUTS 决定导出什么，node 的 directories 决定某条目按目录还是按单文件校验。
        只改前者，node 会把新目录当成文件而报「白名单文件尚未跟踪或目录为空」——
        2026-09-22 T-101 补 deploy/runner 时就先后踩了这两半。
        """
        import re
        root = SCRIPTS.parent
        block = lambda name: PACKAGE.split(name + "=(\n", 1)[1].split("\n)", 1)[0]
        inputs = [word for line in block("STUDIOLO_INPUTS").splitlines()
                  if not line.strip().startswith("#") for word in line.split()]
        literal = re.search(r"const directories = new Set\(\[(.*?)\]\);", PACKAGE, re.S).group(1)
        known = set(re.findall(r"'([^']+)'", literal))
        missing = [entry for entry in inputs
                   if (root / entry).is_dir() and entry not in known]
        self.assertEqual(missing, [], "白名单里的目录未登记进 node 的 directories 集合")

    def packaged_crates(self):
        import re
        block = PACKAGE.split("STUDIOLO_TEST_PACKAGES=(\n", 1)[1].split("\n)", 1)[0]
        return {name.removeprefix("studiolo-") if name != "studiolo" else name
                for name in re.findall(r"-p (studiolo(?:-[a-z-]+)?)", block)}

    def test_node_package_set_matches_the_cargo_package_list(self):
        """cargo 编译哪些包的测试、Node 段就必须接受哪些包；两份清单各写一处，
        T-065 加 process-safety 时只改了前者，真打包到最后一步才报「测试程序不属于选定的工作区包」。"""
        import re
        literal = re.search(r"const packages = new Set\(\[(.*?)\]\);", embedded(PACKAGE, "NODE_TESTS"), re.S).group(1)
        accepted = set(re.findall(r"'([a-z-]+)'", literal))
        self.assertEqual(accepted, self.packaged_crates())
        self.assertLessEqual(set(REQUIRED), accepted)

    def test_every_workspace_crate_linked_by_the_agent_ships_its_tests(self):
        """运行包的口径是「studiolo-agent 实际链接的 crate 都随包带测试」。T-069 让 agent 依赖了
        studiolo-policy 却没进打包列表；从 agent-server 的 Cargo.toml 出发求工作区依赖闭包来核对。"""
        import re
        root = SCRIPTS.parent
        def workspace_dependencies(crate):
            text = (root / "crates" / crate / "Cargo.toml").read_text(encoding="utf-8")
            section = text.split("[dependencies]", 1)[1].split("\n[", 1)[0] if "[dependencies]" in text else ""
            return {name for name in re.findall(r"^studiolo-([a-z-]+)\.workspace\s*=", section, re.M)}
        closure, pending = set(), ["agent-server"]
        while pending:
            crate = pending.pop()
            if crate in closure:
                continue
            closure.add(crate)
            pending.extend(workspace_dependencies(crate))
        self.assertIn("policy", closure)
        self.assertLessEqual(closure, self.packaged_crates(),
                             "agent 链接了这些工作区 crate，但 package-agent.sh 没有随包携带它们的测试")


if __name__ == "__main__":
    unittest.main()
