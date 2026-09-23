#!/usr/bin/env python3
"""预览检查 exercise() 的端到端回归：假服务端按虚构示例库作答，不联网、不读取真实笔记。

基线：对完全正确的假服务端，exercise() 全部通过，且检查项数等于钉死的常量，防止以后悄悄少查。
变异：逐一注入六种服务端缺陷，对应的那项检查必须失败，证明这些检查不是空转。
假连接、静态资源与文件树夹具复用 test-check-preview.py；示例库数据按 exercise() 的断言反推，
行号与真实示例库一致，且刻意不复用 check-preview.py 的常量：那些常量写错时基线会红。
"""

import contextlib
import importlib.util
import io
from pathlib import Path
import sys
import unittest
from unittest import mock
from urllib.parse import parse_qs, urlsplit

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location(
    "test_check_preview", Path(__file__).with_name("test-check-preview.py"))
helpers = importlib.util.module_from_spec(spec)
spec.loader.exec_module(helpers)
# 与辅助文件共用同一份已加载的 check-preview.py，CheckFailed 才是同一个类型。
preview = helpers.preview

# exercise() 当前的检查项数；增删检查都必须在这里显式改数，不能让它悄悄少查。
TOTAL = 74

READING = "项目/纸页/01-框架/阅读体验.md"
GUIDE = "项目/Studiolo/01-框架/搜索说明.md"
SCHEMES = [f"项目/纸页/01-框架/方案{variant}/配色方案.md" for variant in "AB"]
PROJECTS = ["项目/Studiolo", "项目/纸页", "项目/山野笔记", "项目/灵感盒"]
# 阅读体验第 6 行是被搜索的正文、第 8 行是二级标题；搜索说明第 10 行反链回阅读体验。
NOTES = {
    "README.md": "# M1 虚构示例库\n\n这些项目和动作均为人工测试夹具。\n仅供本机只读验收；没有真实笔记或凭据。\n",
    READING: "\n".join([
        "---", "aliases: [阅读指南]", "---", "# 阅读体验", "",
        "连续阅读，保持段落节奏。这个虚构文档用于验收中文子串搜索。", "", "## 字体与间距", "",
        "跳到 [[阅读体验#字体与间距|字体与间距]]，对比 [[配色方案]]。",
        "待补 [[未写的说明]]；尚未定义的标题是 [[阅读体验#尚未定义]]。",
        "```markdown", "这段代码里的 [[代码示例而非链接]] 不应出现在本页链接中。", "```"]),
    GUIDE: "\n".join([
        "# 搜索说明", "", "连续阅读也会出现在这个虚构项目，按项目收窄后不应混入纸页结果。", "", "## 操作", "",
        "在文档页搜索“连续阅读”，再将项目设为“纸页”。",
        "用快速切换查找“纸页 框架 阅读”，完整路径应指向阅读体验文档。", "",
        "来源示例：[[项目/纸页/01-框架/阅读体验#字体与间距|查看阅读指南]]。"]),
    SCHEMES[0]: "# 配色方案\n\n方案 A 的虚构配色。\n",
    SCHEMES[1]: "# 配色方案\n\n方案 B 的虚构配色。\n",
}


def link(raw, status, *targets):
    """一条出链：状态加候选目标；代码围栏里的伪链接不在其中。"""
    return {"raw": raw, "status": status, "targets": [{"path": path, "line": line} for path, line in targets]}


LINKS = [link("阅读体验#字体与间距|字体与间距", "resolved", (READING, 8)),
         link("配色方案", "ambiguous", (SCHEMES[0], 1), (SCHEMES[1], 1)),
         link("未写的说明", "missing"), link("阅读体验#尚未定义", "missing_heading")]
OUTLINE = [{"title": "阅读体验", "level": 1, "source": {"path": READING, "line": 4}},
           {"title": "字体与间距", "level": 2, "source": {"path": READING, "line": 8}}]
BACKLINKS = [{"source": {"path": GUIDE, "line": 10}}]
# 带上这些头（名称不分大小写）就必须取这些值，与真实服务端的同源门禁一致；
# 假连接只收到显式传入的头，真实 http.client 会自动补上正确的 Host，所以缺省视为同源。
ALLOWED = {"host": {"127.0.0.1:4322"}, "origin": {"http://127.0.0.1:4322"},
           "sec-fetch-site": {"same-origin", "none"}}
PAGE = helpers.RawResponse("text/html; charset=utf-8",
                           b"<!doctype html><title>Studiolo</title>\n" + helpers.HTML)


class ExampleServer(helpers.Transport):
    """按虚构示例库作答的假服务端；行为全由实例上的数据决定，变异用例改一处即注入一种缺陷。"""

    def __init__(self):
        super().__init__()
        self.generation, self.refresh_step = 7, 1
        self.notes = dict(NOTES)
        self.links = {item["raw"]: item for item in LINKS}
        self.static = {"/": PAGE, **helpers.ASSETS}
        self.allowed = dict(ALLOWED)
        self.generation_checked = {"/api/document", "/api/search", "/api/tree"}

    def reply(self, method, url, headers):
        self.requests.append((method, url, headers))
        location = urlsplit(url)
        query = {key: values[0] for key, values in parse_qs(location.query, keep_blank_values=True).items()}
        # 同源门禁先于方法与路由（真实服务端在中间件里先判 403）。
        sent = {name.lower(): value for name, value in headers.items()}
        if any(name in sent and sent[name] not in values for name, values in self.allowed.items()):
            return 403, {}
        if method != "GET":
            return 405, {}
        if location.path in self.generation_checked and int(query["generation"]) != self.generation:
            return 409, {}
        if location.path in self.static:
            return 200, self.static[location.path]
        routes = {"/api/v1/bootstrap": lambda query: (200, {"authentication": "local"}),
                  "/api/workbench": self.workbench, "/api/document": self.document,
                  "/api/search": self.search, "/api/tree": self.tree}
        # 其余路径一律 404：静态入口不下发库内笔记、说明或附件，也没有应用会话接口。
        route = routes.get(location.path)
        return route(query) if route else (404, {})

    def workbench(self, query):
        if query.get("refresh") == "true":
            self.generation += self.refresh_step
        return 200, {"generation": self.generation, "projects": [{"id": project} for project in PROJECTS]}

    def document(self, query):
        path = query["path"]
        if path not in self.notes:
            return 404, {}
        navigation = {"outline": [], "outgoing": [], "backlinks": [], "truncated": False}
        if path == READING:
            navigation.update(outline=OUTLINE, outgoing=list(self.links.values()), backlinks=BACKLINKS)
        return 200, {"path": path, "content": self.notes[path], "navigation": navigation}

    def search(self, query):
        project, words = query.get("project"), query["q"]
        if (project is not None and project not in PROJECTS) or query["mode"] not in {"full_text", "path"}:
            return 400, {}
        prefix = project + "/" if project else ""
        if query["mode"] == "path":
            hits = [{"path": path, "line": 1, "kind": "path"} for path in self.notes
                    if path.startswith(prefix) and all(part in path for part in words.split())]
        else:
            hits = [{"path": path, "line": number, "kind": "content"}
                    for path, text in self.notes.items() if path.startswith(prefix)
                    for number, line in enumerate(text.splitlines(), 1) if words in line]
        return 200, {"generation": self.generation, "hits": hits, "truncated": False}

    def tree(self, query):
        path, offset = query["path"], int(query["offset"])
        if ".." in path.split("/"):
            return 400, {}
        if path not in helpers.PAGES:
            return 404, {}
        entries = helpers.PAGES[path]
        if offset and offset >= len(entries):
            return 400, {}
        return 200, {"generation": self.generation, "path": path, "offset": offset,
                     "total": len(entries), "entries": entries, "next_offset": None}


class ExerciseTests(unittest.TestCase):
    def exercise(self, server):
        """对假服务端完整跑一遍 exercise()，返回检查计数与标准输出。"""
        checks, output = preview.Checks(4322), io.StringIO()
        with mock.patch.object(preview.http.client, "HTTPConnection", server.connection), \
                contextlib.redirect_stdout(output):
            try:
                preview.exercise(checks)
            except preview.CheckFailed:
                pass
        return checks, output.getvalue()

    def assert_caught(self, check, inject):
        """注入一种缺陷后指名的检查必须失败；exercise() 首败即停，它之前的检查都照常通过。"""
        server = ExampleServer()
        inject(server)
        checks, output = self.exercise(server)
        self.assertGreaterEqual(checks.failed, 1, output)
        self.assertIn(f"{check}：失败\n", output)

    def test_example_server_passes_every_check(self):
        checks, output = self.exercise(ExampleServer())
        self.assertEqual((checks.failed, checks.total), (0, TOTAL), output)

    def test_stale_search_answered_200_is_caught(self):
        """搜索接口不核对代次：旧代请求照常 200。"""
        self.assert_caught("旧代搜索拒绝 409", lambda server: server.generation_checked.discard("/api/search"))

    def test_changed_reading_line_six_is_caught(self):
        """第 6 行语序被改但仍含搜索词：搜索照常命中，只有逐字比对原文的检查能发现。"""
        self.assert_caught("原文第 6 行一致", lambda server: server.notes.update(
            {READING: server.notes[READING].replace("连续阅读，保持段落节奏", "保持段落节奏，连续阅读")}))

    def test_refresh_without_generation_bump_is_caught(self):
        """显式刷新后代次原地不动。"""
        self.assert_caught("刷新代次加一", lambda server: setattr(server, "refresh_step", 0))

    def test_cross_site_fetch_metadata_accepted_is_caught(self):
        """服务端不看 Sec-Fetch-Site：跨站请求照常 200。"""
        self.assert_caught("FetchMetadata 拒绝 403", lambda server: server.allowed.pop("sec-fetch-site"))

    def test_guessing_one_of_two_candidates_is_caught(self):
        """同名的两份配色方案被猜选成方案 A。"""
        self.assert_caught("同名两候选不猜选", lambda server: server.links.update(
            {"配色方案": link("配色方案", "resolved", (SCHEMES[0], 1))}))

    def test_static_readme_served_is_caught(self):
        """静态入口把库根的 README.md 当成页面资源下发。"""
        self.assert_caught("静态路径不能读取说明", lambda server: server.static.update(
            {"/README.md": helpers.RawResponse("text/markdown; charset=utf-8", NOTES["README.md"].encode())}))


if __name__ == "__main__":
    unittest.main()
