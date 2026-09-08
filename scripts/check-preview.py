#!/usr/bin/env python3
"""检查 M1 虚构示例库的真实 HTTP 入口，不读取或修改本地笔记文件。

用法：python3 scripts/check-preview.py http://127.0.0.1:4317 --example
仅接受明确指定的 IPv4 回环端口；--example 表示确认这是示例库端点。
刷新前必须通过示例指纹检查；不使用代理、不跟随重定向、不连接其他地址。
失败立即停止，仅输出检查名、结果和总数，不打印响应正文或异常中的数据。
退出码：0 全部通过；1 检查失败；2 参数或示例授权不符合要求。
"""

import argparse
import http.client
import json
import re
from html.parser import HTMLParser
from urllib.parse import quote, urlencode


READING = "项目/纸页/01-框架/阅读体验.md"
GUIDE = "项目/Studiolo/01-框架/搜索说明.md"
PROJECTS = {"项目/Studiolo", "项目/纸页", "项目/山野笔记", "项目/灵感盒"}
ATTACHMENTS = "附件演示/仅附件"
EMPTY_DIRECTORY = "附件演示/空目录"
ATTACHMENT_NAMES = {"关系画布.canvas", "不会执行的页面.html", "表格 & 说明.csv"}
MAX_RESPONSE = 8 * 1024 * 1024


class CheckFailed(Exception):
    """检查不通过；故意不携带可能含正文的异常信息。"""


class Assets(HTMLParser):
    """只收集页面声明的脚本、样式和图标入口，不执行或抓取其他引用。"""

    def __init__(self):
        super().__init__()
        self.urls = set()
        self.icons = set()

    def handle_starttag(self, tag, attrs):
        attrs = dict(attrs)
        if tag == "script" and "src" in attrs:
            self.urls.add(attrs["src"])
        if tag == "link" and attrs.get("rel") == "stylesheet":
            self.urls.add(attrs.get("href", ""))
        if tag == "link" and "icon" in attrs.get("rel", "").split():
            self.icons.add(attrs.get("href", ""))
            self.urls.add(attrs.get("href", ""))


class Checks:
    """固定回环连接、有限响应读取以及不泄露正文的检查计数。"""

    def __init__(self, port):
        self.port, self.total, self.failed = port, 0, 0

    def run(self, name, operation):
        self.total += 1
        try:
            result = operation()
            if result is False:
                raise CheckFailed()
        except Exception:
            self.failed += 1
            print(f"{name}：失败")
            raise CheckFailed() from None
        print(f"{name}：通过")
        return result

    def request(self, path, status=200, method="GET", headers=None, as_json=False):
        connection = http.client.HTTPConnection("127.0.0.1", self.port, timeout=10)
        try:
            connection.request(method, path, headers=headers or {})
            response = connection.getresponse()
            body = response.read(MAX_RESPONSE + 1)
            if response.status != status or len(body) > MAX_RESPONSE:
                raise CheckFailed()
            if as_json:
                return json.loads(body)
            return response.getheader("Content-Type", ""), body
        finally:
            connection.close()


def api(route, **parameters):
    return "/api/" + route + ("?" + urlencode(parameters) if parameters else "")


def check_assets(checks, html):
    assets = Assets()
    assets.feed(html.decode("utf-8"))
    checks.run("同源静态入口", lambda: 3 <= len(assets.urls) <= 32
               and all(re.fullmatch(r"/assets/[A-Za-z0-9_/-]+\.(js|css|svg)", url) for url in assets.urls)
               and any(url.endswith(".js") for url in assets.urls)
               and any(url.endswith(".css") for url in assets.urls)
               and len(assets.icons) == 1 and all(url.endswith(".svg") for url in assets.icons))
    types = {"js": "text/javascript", "css": "text/css", "svg": "image/svg+xml"}
    for number, url in enumerate(sorted(assets.urls), 1):
        mime, body = checks.run(f"静态资源 {number}", lambda url=url: checks.request(url))
        expected = types[url.rsplit(".", 1)[1]]
        checks.run(f"静态资源类型 {number}", lambda: mime.partition(";")[0].strip() == expected and bool(body))


def check_bootstrap(checks):
    mode = checks.run("前端认证引导", lambda: checks.request(api("v1/bootstrap"), as_json=True))
    checks.run("明确本机只读模式", lambda: mode == {"authentication": "local"})
    checks.run("应用会话未开放", lambda: checks.request(api("v1/session"), 404))
    checks.run("应用会话签发未开放", lambda: checks.request(api("v1/sessions"), 405, method="POST"))


def tree_url(path, generation, offset=0):
    return api("tree", path=path, generation=generation, offset=offset)


def tree_page(checks, name, path, generation):
    page = checks.run(name, lambda: checks.request(tree_url(path, generation), as_json=True))
    checks.run(name + "快照与分页", lambda: set(page) == {
        "generation", "path", "offset", "total", "entries", "next_offset"
    } and type(page["generation"]) is int and page["generation"] == generation
        and page["path"] == path and type(page["offset"]) is int and page["offset"] == 0
        and type(page["total"]) is int and isinstance(page["entries"], list)
        and page["total"] == len(page["entries"]) <= 200 and page["next_offset"] is None)
    checks.run(name + "只含节点元信息", lambda: all(
        set(item) == {"path", "name", "kind", "size_bytes", "child_count"}
        and isinstance(item["name"], str) and bool(item["name"])
        and "/" not in item["name"] and "\\" not in item["name"]
        and not item["name"].startswith(".")
        and item["path"] == (path + "/" if path else "") + item["name"]
        and ((item["kind"] == "directory" and item["size_bytes"] is None
              and type(item["child_count"]) is int and item["child_count"] >= 0)
             or (item["kind"] in {"markdown", "attachment"} and item["child_count"] is None
                 and isinstance(item["size_bytes"], str)
                 and re.fullmatch(r"0|[1-9][0-9]{0,19}", item["size_bytes"]) is not None
                 and int(item["size_bytes"]) <= 2**64 - 1))
        for item in page["entries"])
        and len({item["path"] for item in page["entries"]}) == page["total"])
    return page


def check_tree(checks, generation):
    root = tree_page(checks, "根文件树", "", generation)
    checks.run("根目录先目录后文档", lambda: [
        (item["name"], item["kind"]) for item in root["entries"]
    ] == [("附件演示", "directory"), ("项目", "directory"), ("README.md", "markdown")])
    folders = tree_page(checks, "附件演示目录", "附件演示", generation)
    checks.run("空目录与纯附件目录可见", lambda: {
        (item["name"], item["kind"], item["child_count"]) for item in folders["entries"]
    } == {("仅附件", "directory", 3), ("空目录", "directory", 0)})
    empty = tree_page(checks, "空目录", EMPTY_DIRECTORY, generation)
    checks.run("空目录无隐藏占位文件", lambda: empty["entries"] == [])
    files = tree_page(checks, "纯附件目录", ATTACHMENTS, generation)
    checks.run("附件名称与只读大小", lambda: {
        item["name"] for item in files["entries"]
    } == ATTACHMENT_NAMES and all(
        item["kind"] == "attachment" and int(item["size_bytes"]) > 0 for item in files["entries"]))
    checks.run("树穿越路径拒绝 400", lambda: checks.request(tree_url("../外部", generation), 400))
    checks.run("树越界分页拒绝 400", lambda: checks.request(tree_url("", generation, root["total"]), 400))
    checks.run("旧代文件树拒绝 409", lambda: checks.request(tree_url("", generation - 1), 409))
    checks.run("树写入拒绝 405", lambda: checks.request(tree_url("", generation), 405, method="POST"))
    checks.run("树跨源拒绝 403", lambda: checks.request(tree_url("", generation), 403,
               headers={"Origin": "http://invalid.example"}))
    attachment = ATTACHMENTS + "/不会执行的页面.html"
    checks.run("附件无正文接口", lambda: checks.request(api("document", path=attachment, generation=generation), 404))
    checks.run("附件无静态下载", lambda: checks.request("/" + quote(attachment, safe="/"), 404))


def exercise(checks):
    page = checks.run("工作台页面", lambda: checks.request("/"))
    checks.run("页面类型与标题", lambda: page[0].startswith("text/html") and b"Studiolo" in page[1])
    check_assets(checks, page[1])
    check_bootstrap(checks)
    snapshot = checks.run("工作台接口", lambda: checks.request(api("workbench"), as_json=True))
    checks.run("四项目示例指纹", lambda: {item["id"] for item in snapshot["projects"]} == PROJECTS
               and type(snapshot["generation"]) is int and snapshot["generation"] > 0)
    generation = snapshot["generation"]
    document_url = lambda path=READING, gen=generation: api("document", path=path, generation=gen)
    search_url = lambda text, gen=generation, **extra: api("search", q=text, generation=gen, **extra)
    readme = checks.run("示例说明原文", lambda: checks.request(document_url("README.md"), as_json=True))
    checks.run("仅虚构示例库", lambda: "M1 虚构示例库" in readme["content"]
               and "人工测试夹具" in readme["content"] and "没有真实笔记或凭据" in readme["content"])
    check_tree(checks, generation)
    all_hits = checks.run("中文全局搜索", lambda: checks.request(search_url("连续阅读", mode="full_text"), as_json=True))
    checks.run("跨项目中文结果", lambda: all_hits["generation"] == generation
               and {READING, GUIDE}.issubset({hit["path"] for hit in all_hits["hits"]}))
    scoped = checks.run("纸页项目收窄", lambda: checks.request(search_url("连续阅读", mode="full_text", project="项目/纸页"), as_json=True))
    checks.run("中文结果真实行 6", lambda: scoped["generation"] == generation and bool(scoped["hits"])
               and all(hit["path"].startswith("项目/纸页/") for hit in scoped["hits"])
               and any(hit["path"] == READING and hit["line"] == 6 and hit["kind"] == "content" for hit in scoped["hits"]))
    document = checks.run("阅读体验原文与导航", lambda: checks.request(document_url(), as_json=True))
    checks.run("原文第 6 行一致", lambda: document["path"] == READING
               and document["content"].splitlines()[5].startswith("连续阅读，保持段落节奏。"))
    navigation = document["navigation"]
    links = {link["raw"]: link for link in navigation["outgoing"]}
    checks.run("大纲标题行 8", lambda: any(item["title"] == "字体与间距"
               and item["source"] == {"path": READING, "line": 8} for item in navigation["outline"]))
    target = {"path": READING, "line": 8}
    checks.run("标题跳转真实行 8", lambda: links["阅读体验#字体与间距|字体与间距"]["status"] == "resolved"
               and [{key: item[key] for key in target} for item in links["阅读体验#字体与间距|字体与间距"]["targets"]] == [target])
    candidates = {f"项目/纸页/01-框架/方案{variant}/配色方案.md" for variant in "AB"}
    checks.run("同名两候选不猜选", lambda: links["配色方案"]["status"] == "ambiguous"
               and len(links["配色方案"]["targets"]) == 2
               and {item["path"] for item in links["配色方案"]["targets"]} == candidates)
    for raw, status, name in [("未写的说明", "missing", "缺页提示"), ("阅读体验#尚未定义", "missing_heading", "缺标题不跳首页")]:
        checks.run(name, lambda raw=raw, status=status: links[raw]["status"] == status and links[raw]["targets"] == [])
    checks.run("全库反链真实来源", lambda: any(item["source"] == {"path": GUIDE, "line": 10} for item in navigation["backlinks"]))
    checks.run("代码围栏无伪链接", lambda: all("代码示例而非链接" not in raw for raw in links))
    quick = checks.run("路径片段快速切换", lambda: checks.request(search_url("纸页 框架 阅读", mode="path"), as_json=True))
    checks.run("快速切换完整路径", lambda: quick["generation"] == generation and len(quick["hits"]) == 1
               and quick["hits"][0]["path"] == READING and quick["hits"][0]["line"] == 1 and quick["hits"][0]["kind"] == "path")
    checks.run("非法项目拒绝 400", lambda: checks.request(search_url("连续阅读", mode="full_text", project="项目/纸页不存在"), 400))
    checks.run("旧代搜索拒绝 409", lambda: checks.request(search_url("连续阅读", gen=generation - 1, mode="full_text"), 409))
    checks.run("旧代原文拒绝 409", lambda: checks.request(document_url(gen=generation - 1), 409))
    for name, headers in [("Host", {"Host": "invalid.example"}), ("Origin", {"Origin": "http://invalid.example"}),
                          ("FetchMetadata", {"Sec-Fetch-Site": "cross-site"})]:
        checks.run(f"{name} 拒绝 403", lambda headers=headers: checks.request(api("workbench"), 403, headers=headers))
    checks.run("POST 拒绝 405", lambda: checks.request(api("workbench"), 405, method="POST"))
    checks.run("静态路径不能读取笔记", lambda: checks.request("/" + quote(READING, safe="/"), 404))
    checks.run("静态路径不能读取说明", lambda: checks.request("/README.md", 404))
    refreshed = checks.run("显式刷新示例端点", lambda: checks.request(api("workbench", refresh="true"), as_json=True))
    checks.run("刷新代次加一", lambda: refreshed["generation"] == generation + 1
               and {item["id"] for item in refreshed["projects"]} == PROJECTS)
    checks.run("刷新后旧搜索拒绝 409", lambda: checks.request(search_url("连续阅读", mode="full_text"), 409))
    checks.run("刷新后旧原文拒绝 409", lambda: checks.request(document_url(), 409))
    checks.run("刷新后旧文件树拒绝 409", lambda: checks.request(tree_url("", generation), 409))
    tree_page(checks, "刷新后新代文件树", "", generation + 1)
    latest = checks.run("新代搜索可用", lambda: checks.request(search_url("连续阅读", gen=generation + 1, mode="full_text"), as_json=True))
    checks.run("新代搜索一致", lambda: latest["generation"] == generation + 1 and bool(latest["hits"]))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("base_url", help="仅允许 http://127.0.0.1:<端口>")
    parser.add_argument("--example", action="store_true", help="确认端点只提供虚构示例库，并允许显式刷新")
    args = parser.parse_args()
    match = re.fullmatch(r"http://127\.0\.0\.1:([1-9][0-9]{0,4})/?", args.base_url)
    if not args.example or not match or int(match[1]) > 65535:
        print("回环地址与示例授权：失败\n总计：1；通过：0；失败：1")
        return 2
    checks = Checks(int(match[1]))
    try:
        exercise(checks)
    except CheckFailed:
        pass
    except Exception:
        checks.total += 1
        checks.failed += 1
        print("响应结构解析：失败")
    print(f"总计：{checks.total}；通过：{checks.total - checks.failed}；失败：{checks.failed}")
    return 1 if checks.failed else 0


if __name__ == "__main__":
    raise SystemExit(main())
