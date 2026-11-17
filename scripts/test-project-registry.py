#!/usr/bin/env python3
"""项目注册表的契约（T-209，ADR-034 方案 A）：注册表是唯一要手改的地方，副本清单和部署模板必须和它一致。

真实仓库现状必须全绿；复制到临时目录后改注册表，核对必须红并点名每一个没跟上的文件，sync 之后变绿。
只用标准库，不联网、不执行被测脚本以外的东西。
"""
import importlib.util
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parent
ROOT = SCRIPTS.parent
sys.path.insert(0, str(SCRIPTS))
import project_registry as registry  # noqa: E402
from replica_common import ALLOW, Invalid  # noqa: E402

NEW = "项目/虚构新项目"


def raw(entries, **top):
    return json.dumps({"schema_version": 1, "entries": entries, **top}, ensure_ascii=False).encode("utf-8")


def entry(path, visible=True, **extra):
    return {"path": path, "ai_visible": visible, **extra}


class RealRepositoryTests(unittest.TestCase):
    def test_registry_replica_allowlist_and_every_deploy_template_agree(self):
        problems, notes = registry.check(ROOT)
        self.assertEqual(problems, [], "注册表改了却没跑 sync，或有人手改了副本清单、部署模板")
        self.assertEqual(notes, [])
        entries = registry.load_registry(ROOT)
        self.assertEqual(registry.allow_of(entries), ALLOW, "副本清单必须等于注册表里给 AI 看的路径")
        self.assertEqual(registry.sync(ROOT, dry_run=True), ([], []), "预演必须是没有要改的")

    def test_the_templates_are_discovered_not_listed(self):
        found = [path.relative_to(ROOT).as_posix() for path in registry.templates(ROOT)]
        self.assertGreaterEqual(len(found), 5, "没扫到带 --allow 的部署模板，发现逻辑失效")
        for name in found:
            text = (ROOT / name).read_text(encoding="utf-8")
            self.assertEqual(re.findall(r'--allow "([^"]+)"', text), ALLOW, name)

    def test_the_command_line_reports_agreement(self):
        done = subprocess.run([sys.executable, str(SCRIPTS / "project_registry.py"), "check"],
                              capture_output=True, text=True, check=False)
        self.assertEqual((done.returncode, done.stdout), (0, ""), done.stderr)
        self.assertIn("一致", done.stderr)

    def test_project_names_written_into_code_are_registered(self):
        """本机客户端和网页能力集草稿模板各写死了项目名：库里改名后漏改，这里先红（BUG-038）。"""
        allow = set(registry.allow_of(registry.load_registry(ROOT)))
        from local_client.config import BOARD_PROJECT
        self.assertIn(BOARD_PROJECT, allow, "scripts/local_client/config.py 的 BOARD_PROJECT")
        source = (ROOT / "web/src/policy/submitDraft.ts").read_text(encoding="utf-8")
        found = re.findall(r"project_whitelist:\s*\[([^\]]*)\]", source)
        self.assertEqual(len(found), 1, "submitDraft.ts 里找不到唯一的能力集模板 project_whitelist")
        paths = re.findall(r"'([^']*)'", found[0])
        self.assertTrue(paths, "能力集模板的 project_whitelist 是空的")
        self.assertLessEqual(set(paths), allow, "web/src/policy/submitDraft.ts 的能力集模板")


class CopiedRepositoryTests(unittest.TestCase):
    """把注册表、副本清单和部署模板复制进临时目录再改，不动真实仓库。"""

    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)
        (self.root / "scripts").mkdir()
        for name in (registry.REGISTRY, registry.REPLICA):
            shutil.copy(ROOT / name, self.root / name)
        shutil.copytree(ROOT / "deploy" / "agent", self.root / "deploy" / "agent")
        self.targets = [registry.REPLICA] + [p.relative_to(self.root).as_posix() for p in registry.templates(self.root)]
        self.assertEqual(len(self.targets), 6)

    def rewrite(self, change):
        path = self.root / registry.REGISTRY
        value = json.loads(path.read_text(encoding="utf-8"))
        change(value["entries"])
        path.write_text(json.dumps(value, ensure_ascii=False), encoding="utf-8")

    def allowlist(self):
        """不 import 临时副本，按文本读回副本清单。"""
        block = (self.root / registry.REPLICA).read_text(encoding="utf-8").split("ALLOW = sorted([\n", 1)[1]
        return re.findall(r'^    "([^"]+)",$', block.split("])\n", 1)[0], flags=re.M)

    def test_a_new_project_is_red_everywhere_until_sync_then_green(self):
        self.rewrite(lambda entries: entries.append(entry(NEW)))
        problems, _ = registry.check(self.root)
        self.assertEqual(sorted(problems), sorted(f"{name}: 和注册表对不上，跑一次 sync" for name in self.targets))
        self.assertEqual(registry.sync(self.root, dry_run=True), (sorted(self.targets), []))
        self.assertNotIn(NEW, self.allowlist(), "预演不许写文件")
        self.assertEqual(registry.sync(self.root), (sorted(self.targets), []))
        self.assertEqual(registry.check(self.root), ([], []))
        expected = sorted(ALLOW + [NEW])
        self.assertEqual(self.allowlist(), expected)
        for name in self.targets[1:]:
            text = (self.root / name).read_text(encoding="utf-8")
            self.assertEqual(re.findall(r'--allow "([^"]+)"', text), expected, name)
            # 除了那一串 --allow，模板别处一个字不变。
            original = (ROOT / name).read_text(encoding="utf-8")
            self.assertEqual(text.replace(registry.render_run(expected), registry.render_run(ALLOW)), original, name)
        self.assertEqual(registry.sync(self.root), ([], []), "再跑一次没有要改的")

    def test_hiding_a_project_takes_it_out_of_every_list(self):
        hidden = ALLOW[-1]

        def hide(entries):
            next(item for item in entries if item["path"] == hidden)["ai_visible"] = False

        self.rewrite(hide)
        self.assertEqual(len(registry.check(self.root)[0]), 6)
        registry.sync(self.root)
        self.assertEqual(self.allowlist(), ALLOW[:-1])
        self.assertEqual(registry.check(self.root), ([], []))

    def test_file_modes_survive_a_sync(self):
        script = self.root / "deploy/agent/install-t036-system.sh"
        os.chmod(script, 0o755)
        self.rewrite(lambda entries: entries.append(entry(NEW)))
        registry.sync(self.root)
        self.assertEqual(os.stat(script).st_mode & 0o777, 0o755)
        self.assertEqual(list(self.root.rglob("*.registry-tmp")), [], "不留临时文件")

    def test_one_unreadable_target_stops_the_whole_sync(self):
        unit = self.root / "deploy/agent/studiolo-agent.service.in"
        original = unit.read_text(encoding="utf-8")
        self.rewrite(lambda entries: entries.append(entry(NEW)))
        for broken in (original.replace(' --allow "_规则"', "", 1) + '\n# --allow "_规则"\n',   # 第二串
                       original.replace('--allow "_规则"', "--allow _规则", 1),                # 不带引号
                       original + "\n# 说明里又提了一次 --allow\n"):
            with self.subTest(broken=broken[-40:]):
                unit.write_text(broken, encoding="utf-8")
                changed, problems = registry.sync(self.root)
                self.assertEqual(changed, [])
                self.assertEqual(len(problems), 1, problems)
                self.assertIn("deploy/agent/studiolo-agent.service.in", problems[0])
                self.assertNotIn(NEW, self.allowlist(), "有一处读不懂时一处都不许写")
        replica = self.root / registry.REPLICA
        unit.write_text(original, encoding="utf-8")
        replica.write_text(replica.read_text(encoding="utf-8").replace("ALLOW = sorted([", "ALLOW = ["), encoding="utf-8")
        self.assertEqual(registry.sync(self.root)[0], [])
        self.assertIn(registry.REPLICA, registry.sync(self.root)[1][0])
        self.assertIn(registry.REPLICA, registry.check(self.root)[0][0])

    def test_no_template_at_all_is_a_failure_not_a_pass(self):
        shutil.rmtree(self.root / "deploy" / "agent")
        problems, _ = registry.check(self.root)
        self.assertEqual(len(problems), 1)
        self.assertIn("发现逻辑失效", problems[0])

    def test_the_vault_report_names_missing_and_unregistered_directories(self):
        vault = self.root / "vault"
        for path in ALLOW:
            (vault / path).mkdir(parents=True)
        self.assertEqual(registry.check(self.root, vault), ([], []))
        gone, linked = ALLOW[1], ALLOW[2]
        (vault / gone).rmdir()
        (vault / linked).rmdir()
        (vault / linked).symlink_to(vault / ALLOW[3], target_is_directory=True)
        (vault / "项目" / "还没登记的").mkdir()
        (vault / "项目" / ".隐藏目录").mkdir()
        (vault / "项目" / "散放文件.md").write_text("", encoding="utf-8")
        problems, notes = registry.check(self.root, vault)
        self.assertEqual(problems, [f"{gone}: 登记了给 AI 看，但库里没有这个目录",
                                    f"{linked}: 登记了给 AI 看，但库里没有这个目录"])
        self.assertEqual(notes, ["项目/还没登记的: 库里有、注册表里没登记"])


class RegistryValidationTests(unittest.TestCase):
    def test_a_minimal_registry_parses_sorted(self):
        parsed = registry.parse_registry(raw([entry("项目/乙", note="备注"), entry("_规则"), entry("项目/甲", False)]))
        self.assertEqual(parsed, [("_规则", True, ""), ("项目/乙", True, "备注"), ("项目/甲", False, "")])
        self.assertEqual(registry.allow_of(parsed), ["_规则", "项目/乙"])

    def test_anything_irregular_refuses_the_whole_registry(self):
        good = entry("项目/甲")
        cases = {
            "不是 JSON": b"{",
            "重复的键": b'{"schema_version":1,"schema_version":1,"entries":[{"path":"_\xe8\xa7\x84\xe5\x88\x99","ai_visible":true}]}',
            "多出顶层键": raw([good], extra=1),
            "版本不认": json.dumps({"schema_version": 2, "entries": [good]}).encode(),
            "版本是布尔": json.dumps({"schema_version": True, "entries": [good]}).encode(),
            "空列表": raw([]),
            "多出字段": raw([entry("项目/甲", repository="git@example.invalid:x/y.git")]),
            "缺 ai_visible": raw([{"path": "项目/甲"}]),
            "ai_visible 不是布尔": raw([{"path": "项目/甲", "ai_visible": 1}]),
            "备注太长": raw([entry("项目/甲", note="字" * 201)]),
            "登记两次": raw([good, good]),
            "三段路径": raw([entry("项目/甲/子目录")]),
            "别的顶层": raw([entry("资料/甲")]),
            "只有项目两字": raw([entry("项目")]),
            "隐藏目录": raw([entry("项目/.git")]),
            "带双引号": raw([entry('项目/甲"乙')]),
            "带美元号": raw([entry("项目/甲$乙")]),
            "带百分号": raw([entry("项目/甲%乙")]),
            "带反引号": raw([entry("项目/甲`乙")]),
            "带单引号": raw([entry("项目/甲'乙")]),
            "尾部空格": raw([entry("项目/甲 ")]),
            "路径不是字符串": raw([{"path": 7, "ai_visible": True}]),
            "项不是对象": raw(["项目/甲"]),
        }
        for label, body in cases.items():
            with self.subTest(label=label), self.assertRaises(Invalid):
                registry.parse_registry(body)

    def test_a_registry_with_nothing_visible_cannot_produce_an_allowlist(self):
        with self.assertRaises(Invalid):
            registry.allow_of(registry.parse_registry(raw([entry("项目/甲", False)])))


class DesiredProposalTests(unittest.TestCase):
    def setUp(self):
        self.entries = registry.parse_registry(raw([entry("_规则"), entry("项目/甲"), entry("项目/乙"), entry("项目/丙", False)]))
        self.capability = {
            "ai_type": "claude",
            "read": {"project_whitelist": ["项目/甲", "_规则"], "deny_by_default": True, "rules_whitelist": ["通用工作规则.md"]},
            "write": {"board": True, "board_projects": ["项目/甲"], "content_patch_only": True, "content_direct": False},
            "policy": {"propose_desired": True, "activate": False},
            "hard_bound_skills": [],
        }

    def test_nothing_to_do_yields_no_proposal(self):
        result, notes = registry.desired(self.capability, self.entries)
        self.assertIsNone(result)
        self.assertEqual(len(notes), 1)

    def test_a_named_grant_is_added_and_nothing_else_widens(self):
        result, notes = registry.desired(self.capability, self.entries, grants=["项目/乙"])
        self.assertEqual(result["read"]["project_whitelist"], ["_规则", "项目/乙", "项目/甲"])
        self.assertEqual(notes, ["加上：项目/乙"])
        untouched = dict(result, read=dict(result["read"], project_whitelist=self.capability["read"]["project_whitelist"]))
        self.assertEqual(untouched, self.capability, "除了读取白名单，别的字段一个不变")
        self.assertEqual(self.capability["read"]["project_whitelist"], ["项目/甲", "_规则"], "不改传进来的那份")

    def test_grant_all_adds_exactly_the_visible_projects(self):
        result, _ = registry.desired(self.capability, self.entries, grant_all=True)
        self.assertEqual(result["read"]["project_whitelist"], ["_规则", "项目/乙", "项目/甲"])

    def test_a_grant_outside_the_registry_or_hidden_is_refused(self):
        for grant in ("项目/没登记", "项目/丙", "项目/甲/子目录"):
            with self.subTest(grant=grant), self.assertRaises(Invalid):
                registry.desired(self.capability, self.entries, grants=[grant])

    def test_prefixes_outside_the_deploy_ceiling_are_dropped_with_their_board_projects(self):
        self.capability["read"]["project_whitelist"] += ["项目/丙", "项目/早就删了的", "项目/甲/01-框架", "项目"]
        self.capability["write"]["board_projects"] += ["项目/丙"]
        result, notes = registry.desired(self.capability, self.entries)
        # 更窄的子路径和更宽的上级前缀都和部署上限有交集，留着；完全在外面的删掉。
        self.assertEqual(result["read"]["project_whitelist"], ["_规则", "项目", "项目/甲", "项目/甲/01-框架"])
        self.assertEqual(result["write"]["board_projects"], ["项目/甲"])
        self.assertEqual(notes, ["删掉（部署上限之外）：项目/丙", "删掉（部署上限之外）：项目/早就删了的", "黑板写项目同步删掉：项目/丙"])

    def test_a_grant_already_covered_by_a_wider_prefix_is_not_repeated(self):
        self.capability["read"]["project_whitelist"] = ["项目"]
        self.capability["write"]["board_projects"] = []
        self.assertIsNone(registry.desired(self.capability, self.entries, grants=["项目/乙"])[0])

    def test_an_unrecognised_capability_shape_is_refused(self):
        for broken in ({}, [], {"read": {}, "write": {}}, {"read": {"project_whitelist": "项目/甲"}, "write": {"board_projects": []}},
                       {"read": {"project_whitelist": ["项目/甲"]}, "write": {"board_projects": "项目/甲"}},
                       {"read": {"project_whitelist": [1]}, "write": {"board_projects": []}}):
            with self.subTest(broken=broken), self.assertRaises(Invalid):
                registry.desired(broken, self.entries)

    def test_a_rename_moves_whitelist_and_board_projects_to_the_new_name(self):
        entries = registry.parse_registry(raw([entry("_规则"), entry("项目/甲-Docs"), entry("项目/乙")]))
        self.capability["read"]["project_whitelist"] += ["项目/甲/01-框架"]
        # 几份能力集共用一张对照表：这份里没有的旧名跳过。
        renames = [("项目/甲", "项目/甲-Docs"), ("项目/这份里没有", "项目/乙")]
        result, notes = registry.desired(self.capability, entries, renames=renames)
        self.assertEqual(result["read"]["project_whitelist"], ["_规则", "项目/甲-Docs", "项目/甲-Docs/01-框架"])
        self.assertEqual(result["write"]["board_projects"], ["项目/甲-Docs"])
        self.assertEqual(notes, ["改名：项目/甲 → 项目/甲-Docs", "改名：项目/甲/01-框架 → 项目/甲-Docs/01-框架",
                                 "黑板写项目改名：项目/甲 → 项目/甲-Docs",
                                 "提示：对照表里的 项目/这份里没有 这份能力集没有（几份能力集共用一张表时正常）"])

    def test_a_rename_only_moves_whole_path_segments(self):
        # 真实对照表就有这种形状：项目/Semiment → 项目/Semiment-网站-Docs，旁边的 项目/Semiment-Docs 不能跟着变。
        entries = registry.parse_registry(raw([entry("_规则"), entry("项目/甲-Docs"), entry("项目/甲-网站-Docs"),
                                               entry("项目/甲乙")]))
        self.capability["read"]["project_whitelist"] = ["项目/甲", "项目/甲-Docs", "项目/甲乙", "_规则"]
        result, notes = registry.desired(self.capability, entries, renames=[("项目/甲", "项目/甲-网站-Docs")])
        self.assertEqual(result["read"]["project_whitelist"], ["_规则", "项目/甲-Docs", "项目/甲-网站-Docs", "项目/甲乙"])
        self.assertEqual(result["write"]["board_projects"], ["项目/甲-网站-Docs"])
        self.assertEqual(notes, ["改名：项目/甲 → 项目/甲-网站-Docs", "黑板写项目改名：项目/甲 → 项目/甲-网站-Docs"])

    def test_without_the_rename_a_renamed_board_project_is_refused_not_emptied(self):
        entries = registry.parse_registry(raw([entry("_规则"), entry("项目/甲-Docs")]))
        with self.assertRaisesRegex(Invalid, "--rename"):
            registry.desired(self.capability, entries)

    def test_with_board_writes_off_dropping_every_board_project_is_allowed(self):
        # 黑板写入关着（先批只读能力集那一支）时，写项目删光是正常结果，不拒绝。
        entries = registry.parse_registry(raw([entry("_规则"), entry("项目/甲-Docs")]))
        self.capability["write"]["board"] = False
        result, notes = registry.desired(self.capability, entries)
        self.assertEqual(result["read"]["project_whitelist"], ["_规则"])
        self.assertEqual(result["write"]["board_projects"], [])
        self.assertEqual(notes, ["删掉（部署上限之外）：项目/甲", "黑板写项目同步删掉：项目/甲"])

    def test_a_bad_rename_table_is_refused(self):
        cases = {
            "新名没登记": [("项目/甲", "项目/没登记")],
            "新名不给 AI 看": [("项目/甲", "项目/丙")],
            "两边一样": [("项目/甲", "项目/甲")],
            "旧名重复": [("项目/甲", "项目/乙"), ("项目/甲", "_规则")],
            "新名重复": [("项目/甲", "项目/乙"), ("项目/戊", "项目/乙")],
            "旧名嵌套": [("项目/甲", "项目/乙"), ("项目/甲/01-框架", "_规则")],
            "串联": [("项目/甲", "项目/乙"), ("项目/乙", "_规则")],
        }
        for case, renames in cases.items():
            with self.subTest(case=case), self.assertRaises(Invalid):
                registry.desired(self.capability, self.entries, renames=renames)

    def test_the_real_initial_capability_sets_need_no_proposal(self):
        entries = registry.load_registry(ROOT)
        for name in ("claude.json", "codex.json"):
            with self.subTest(name=name):
                capability = json.loads((ROOT / "docs/m5/initial-capabilities" / name).read_text(encoding="utf-8"))
                self.assertIsNone(registry.desired(capability, entries)[0])


class CommandLineTests(unittest.TestCase):
    def run_cli(self, *args):
        return subprocess.run([sys.executable, str(SCRIPTS / "project_registry.py"), *args],
                              capture_output=True, text=True, check=False)

    def test_sync_dry_run_on_the_real_tree_changes_nothing(self):
        done = self.run_cli("sync", "--dry-run")
        self.assertEqual((done.returncode, done.stdout), (0, ""), done.stderr)
        self.assertIn("没有要改的", done.stderr)

    def test_desired_prints_only_the_proposal_body_on_stdout(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "claude.json"
            shutil.copy(ROOT / "docs/m5/initial-capabilities/claude.json", path)
            quiet = self.run_cli("desired", "--capability", str(path))
            self.assertEqual((quiet.returncode, quiet.stdout), (0, ""), quiet.stderr)
            granted = self.run_cli("desired", "--capability", str(path), "--grant", "项目/Tillerwright-Docs")
            self.assertEqual(granted.returncode, 0, granted.stderr)
            body = json.loads(granted.stdout)
            self.assertIn("项目/Tillerwright-Docs", body["read"]["project_whitelist"])
            self.assertIn("由人批准后才生效", granted.stderr)
            refused = self.run_cli("desired", "--capability", str(path), "--grant", "项目/没登记")
            self.assertEqual((refused.returncode, refused.stdout), (2, ""))

    def test_desired_takes_renames_as_old_new_pairs(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "claude.json"
            capability = json.loads((ROOT / "docs/m5/initial-capabilities/claude.json").read_text(encoding="utf-8"))
            capability["read"]["project_whitelist"] = ["项目/Studiolo", "_规则"]
            capability["write"]["board_projects"] = ["项目/Studiolo"]
            path.write_text(json.dumps(capability, ensure_ascii=False), encoding="utf-8")
            refused = self.run_cli("desired", "--capability", str(path))
            self.assertEqual((refused.returncode, refused.stdout), (2, ""))
            self.assertIn("--rename", refused.stderr)
            done = self.run_cli("desired", "--capability", str(path), "--rename", "项目/Studiolo", "项目/Studiolo-Docs")
            self.assertEqual(done.returncode, 0, done.stderr)
            body = json.loads(done.stdout)
            self.assertEqual(body["read"]["project_whitelist"], ["_规则", "项目/Studiolo-Docs"])
            self.assertEqual(body["write"]["board_projects"], ["项目/Studiolo-Docs"])
            self.assertIn("改名：项目/Studiolo → 项目/Studiolo-Docs", done.stderr)

    def test_the_module_loads_without_side_effects(self):
        spec = importlib.util.spec_from_file_location("registry_probe", SCRIPTS / "project_registry.py")
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        self.assertEqual(module.REGISTRY, "scripts/project_registry.json")


if __name__ == "__main__":
    unittest.main()
