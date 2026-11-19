#!/usr/bin/env python3
"""用虚构 Cargo 输出和源码树校验打包门槛；不构建、不联网、不运行服务。"""
import json
from pathlib import Path
import re
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

    def test_full_load_capacity_contract_is_built_but_not_shipped(self):
        # 部署验收在生产主机上原生跑运行包里的每个测试；满额负载的容量契约只在 ci 套跑（T-215 复查）。
        capacity = artifact("agent-server", "replica_capacity_contract")
        memory = artifact("agent-server", "replica_memory_contract")
        same_name_elsewhere = artifact("supply", "replica_capacity_contract")
        code, names = self.collect(self.complete() + [capacity, memory, same_name_elsewhere])
        self.assertEqual(code, 0)
        shipped = lambda item: "tests/" + Path(item["executable"]).name in names
        self.assertFalse(shipped(capacity))
        # 只排除这一个包的这一个目标：同包别的内存契约、别的包同名目标照旧随包。
        self.assertTrue(shipped(memory))
        self.assertTrue(shipped(same_name_elsewhere))
        self.assertEqual(len(names), 25)

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


# debug 下不跑、只在 release 跑的测试（如满额容量契约）：release 打包时照跑，所以必须在排除名单里。
# 认两种编译期写法：`cfg_attr(debug_assertions, ignore…)` 与 `cfg(not(debug_assertions))`。
RELEASE_ONLY = re.compile(r"cfg_attr\(\s*debug_assertions\s*,\s*ignore\b|\bcfg\(\s*not\(\s*debug_assertions\s*\)\s*\)")
# 运行时的 `cfg!(debug_assertions)` 守卫分不清是「debug 下提前返回」还是换个阈值，测试里一律不许用。
RUNTIME_DEBUG_CHECK = re.compile(r"\bcfg!\(\s*debug_assertions\s*\)")


def excluded_tests(script, label):
    """从打包脚本的 Node 段取出 PACKAGE_EXCLUDED_TESTS 的字面量；找不到返回 None。"""
    literal = re.search(r"const PACKAGE_EXCLUDED_TESTS = new Set\(\[(.*?)\]\);", embedded(script, label), re.S)
    return set(re.findall(r"'([^']+)'", literal.group(1))) if literal else None


# 守卫的报错文字：测试按这几个函数逐条断言是哪一条，免得一条检查坏了、别的检查碰巧报红把它盖住。
def missing_target(entry):
    return f"排除名单里的 {entry} 没有对应的测试目标（crates/<包>/tests/<目标>.rs 或 <目标>/main.rs）"


def unrun_exclusion(entry, package, target):
    return (f"排除名单里的 {entry} 在 ci 套里没有专门跑它的一步："
            f"要有一行 `cargo --locked test --release -p {package} --test {target}`")


def shipped_release_only(entry):
    return f"{entry} 只在 release 跑，却会随运行包上生产主机：把它加进两个打包脚本的 PACKAGE_EXCLUDED_TESTS"


def release_only_in_module(relative):
    return (f"{relative} 有只在 release 跑的用例，但它是测试目标的子模块：排除名单按目标排，"
            "把这些用例挪进独立的测试目标，再把目标加进排除名单")


def runtime_debug_check(relative):
    return (f"{relative} 用了运行时的 cfg!(debug_assertions)：守卫分不清它是不是只在 release 跑，"
            "改用 #[cfg_attr(debug_assertions, ignore = \"…\")] 写明")


def target_of(path, tests):
    """集成测试目标的根文件（tests/<目标>.rs 或 tests/<目标>/main.rs）返回目标名，子模块返回 None。"""
    if path.parent == tests:
        return path.stem
    if path.name == "main.rs" and path.parent.parent == tests:
        return path.parent.name
    return None


def exclusion_problems(agent, preview, root, testbox):
    """两个运行包的排除名单要逐项相等；每一项都对应一个真实的集成测试目标，并在 ci 套里另有专门的一步；
    凡是只在 release 跑的集成测试目标都必须在名单里（不然 release 打包时它随包上生产主机）。"""
    if agent is None or preview is None:
        return ["打包脚本里找不到 PACKAGE_EXCLUDED_TESTS"]
    problems = []
    if agent != preview:
        problems.append(f"两个运行包的排除名单不一致：{sorted(agent ^ preview)}")
    for entry in sorted(agent | preview):
        crate, target = entry.split("/", 1)
        tests = root / "crates" / crate / "tests"
        if not ((tests / f"{target}.rs").is_file() or (tests / target / "main.rs").is_file()):
            problems.append(missing_target(entry))
        package = "studiolo" if crate == "studiolo" else f"studiolo-{crate}"
        step = rf"^cargo --locked test --release -p {re.escape(package)} --test {re.escape(target)}$"
        if not re.search(step, testbox, re.M):
            problems.append(unrun_exclusion(entry, package, target))
    for path in sorted((root / "crates").glob("*/tests/**/*.rs")):
        text = path.read_text(encoding="utf-8")
        relative = path.relative_to(root).as_posix()
        if RUNTIME_DEBUG_CHECK.search(text):
            problems.append(runtime_debug_check(relative))
        if not RELEASE_ONLY.search(text):
            continue
        tests = next(parent for parent in path.parents if parent.name == "tests")
        target = target_of(path, tests)
        if target is None:
            problems.append(release_only_in_module(relative))
            continue
        entry = f"{tests.parent.name}/{target}"
        if entry not in agent or entry not in preview:
            problems.append(shipped_release_only(entry))
    return problems


class ExcludedTestsGuardTests(unittest.TestCase):
    """T-215 复查：排除名单按字符串匹配。目标改名、两个包各写一份对不上、新加只在 release 跑的测试，
    都会让满额负载悄悄回到生产主机的部署验收里，而别的检查照样全绿。每个反例断言恰好报哪几条。"""
    ROOT = SCRIPTS.parent
    PREVIEW = (SCRIPTS / "package-preview.sh").read_text(encoding="utf-8")
    TESTBOX = (SCRIPTS.parent / ".testbox.toml").read_text(encoding="utf-8")
    ENTRY = "agent-server/replica_capacity_contract"
    PACKAGE = "studiolo-agent-server"

    def names(self):
        return excluded_tests(PACKAGE, "NODE_TESTS"), excluded_tests(self.PREVIEW, "NODE")

    def sandbox(self, files):
        """临时仓库：排除名单里那个目标照样存在（带 release 专用属性），再加上 files 里的文件。"""
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        root = Path(temp.name)
        files = {"crates/agent-server/tests/replica_capacity_contract.rs":
                 "#[cfg_attr(debug_assertions, ignore = \"release\")]\nfn slow() {}\n", **files}
        for relative, text in files.items():
            path = root / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text, encoding="utf-8")
        return root

    def test_both_packages_exclude_exactly_the_release_only_targets(self):
        agent, preview = self.names()
        self.assertEqual(agent, {self.ENTRY})
        self.assertEqual(exclusion_problems(agent, preview, self.ROOT, self.TESTBOX), [])

    def test_renamed_mismatched_or_unrun_exclusions_are_reported(self):
        agent, preview = self.names()
        renamed = "agent-server/replica_capacity_contract_v2"
        cases = {
            "改名后名单失效": (({renamed}, {renamed}, self.TESTBOX),
                        [missing_target(renamed),
                         unrun_exclusion(renamed, self.PACKAGE, "replica_capacity_contract_v2"),
                         shipped_release_only(self.ENTRY)]),
            "两个包对不上": ((agent, set(), self.TESTBOX),
                       [f"两个运行包的排除名单不一致：{[self.ENTRY]}", shipped_release_only(self.ENTRY)]),
            "ci 套里那一步不是 release": ((agent, preview, self.TESTBOX.replace(
                "test --release -p studiolo-agent-server --test replica_capacity_contract",
                "test -p studiolo-agent-server --test replica_capacity_contract")),
                [unrun_exclusion(self.ENTRY, self.PACKAGE, "replica_capacity_contract")]),
            "ci 套里只有名字更长的一步": ((agent, preview, self.TESTBOX.replace(
                "--test replica_capacity_contract", "--test replica_capacity_contract_full")),
                [unrun_exclusion(self.ENTRY, self.PACKAGE, "replica_capacity_contract")]),
            "名单整个没了": ((None, preview, self.TESTBOX), ["打包脚本里找不到 PACKAGE_EXCLUDED_TESTS"]),
        }
        for case, ((a, p, testbox), expected) in cases.items():
            with self.subTest(case=case):
                self.assertEqual(exclusion_problems(a, p, self.ROOT, testbox), expected)

    def test_each_release_only_shape_is_reported_on_its_own(self):
        agent, preview = self.names()
        cases = {
            "名单里的目标不存在": (None, [missing_target(self.ENTRY)]),
            "新加 cfg_attr 忽略的目标": ({"crates/x/tests/y.rs": "#[cfg_attr(\n    debug_assertions,\n    ignore\n)]\n"},
                                [shipped_release_only("x/y")]),
            "目录式目标用 cfg(not(debug_assertions))": ({"crates/x/tests/y/main.rs": "#[cfg(not(debug_assertions))]\n"},
                                                    [shipped_release_only("x/y")]),
            "写在子模块里": ({"crates/x/tests/support/slow.rs": "#[cfg_attr(debug_assertions, ignore)]\n"},
                       [release_only_in_module("crates/x/tests/support/slow.rs")]),
            "运行时 cfg!": ({"crates/x/tests/y.rs": "if cfg!(debug_assertions) { return; }\n"},
                         [runtime_debug_check("crates/x/tests/y.rs")]),
            "只在 debug 下忽略的不算": ({"crates/x/tests/y.rs": "#[cfg_attr(not(debug_assertions), ignore)]\n"}, []),
        }
        for case, (files, expected) in cases.items():
            with self.subTest(case=case):
                if files is None:
                    temp = tempfile.TemporaryDirectory()
                    self.addCleanup(temp.cleanup)
                    root = Path(temp.name)
                else:
                    root = self.sandbox(files)
                self.assertEqual(exclusion_problems(agent, preview, root, self.TESTBOX), expected)


if __name__ == "__main__":
    unittest.main()
