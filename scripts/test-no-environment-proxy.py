#!/usr/bin/env python3
"""scripts、deploy 里的脚本一律不走环境代理：带令牌的请求经代理出去就等于交出令牌。

设了 http_proxy、没设 no_proxy 的机器上，urllib 默认连回环都走代理。T-237 的独立复查里这一类漏过两次，所以做成检查。
判红时改成 `urllib.request.build_opener(urllib.request.ProxyHandler({}), …).open(…)`。
"""
import ast
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parent.parent
# 默认就读环境代理的入口；只要出现名字就算，存成变量、当默认参数也一样。
DEFAULT_OPENERS = {"urlopen", "urlretrieve", "install_opener", "URLopener", "FancyURLopener"}
CLOSED = {"ProxyHandler({})", "urllib.request.ProxyHandler({})"}


def name_of(node):
    if isinstance(node, ast.Attribute):
        return node.attr
    return node.id if isinstance(node, ast.Name) else None


def leaks(source):
    """会走环境代理的写法，给出行号。"""
    tree = ast.parse(source)
    closed = {id(node.func) for node in ast.walk(tree) if isinstance(node, ast.Call)
              and any(ast.unparse(argument) in CLOSED for argument in node.args)}
    found = set()
    for node in ast.walk(tree):
        if isinstance(node, ast.ImportFrom) and node.module == "urllib.request":
            # 改名引入之后就认不出来了。
            found |= {node.lineno for item in node.names
                      if item.name in DEFAULT_OPENERS or (item.name == "build_opener" and item.asname)}
        name = name_of(node)
        if name in DEFAULT_OPENERS or (name == "build_opener" and id(node) not in closed):
            found.add(node.lineno)
    return sorted(found)


class NoEnvironmentProxy(unittest.TestCase):
    def test_every_script_closes_the_environment_proxy(self):
        checked = [path for folder in ("scripts", "deploy") for path in sorted((ROOT / folder).rglob("*.py"))
                   if not path.name.startswith(("test-", "test_"))]
        self.assertGreater(len(checked), 50, "没找到脚本")
        found = [f"{path.relative_to(ROOT)}:{line}" for path in checked
                 for line in leaks(path.read_text(encoding="utf-8"))]
        self.assertEqual(found, [], __doc__)

    def test_ways_that_use_the_environment_proxy_are_caught(self):
        for source in ("urllib.request.urlopen(url)", "urlopen(url)", "def get(url, send=urllib.request.urlopen): pass",
                       "send = urllib.request.urlopen", "from urllib.request import urlopen as fetch",
                       "from urllib.request import urlopen", "urllib.request.urlretrieve(url)",
                       "urllib.request.install_opener(opener)", "urllib.request.URLopener()",
                       "urllib.request.FancyURLopener()", "urllib.request.build_opener()",
                       "build_opener(NoRedirect())", "make = urllib.request.build_opener",
                       "from urllib.request import build_opener as make",
                       "urllib.request.build_opener(urllib.request.ProxyHandler())",
                       "urllib.request.build_opener(urllib.request.ProxyHandler(proxies))"):
            self.assertEqual(leaks(source), [1], source)

    def test_ways_that_do_not_are_let_through(self):
        for source in ("urllib.request.build_opener(urllib.request.ProxyHandler({}), NoRedirect()).open(url)",
                       "build_opener(ProxyHandler({}))", "opener.open(url)", "# urlopen(url)", "x = 'urlopen('",
                       "from urllib.request import ProxyHandler, build_opener, Request",
                       "from elsewhere import build_opener as make", "http.client.HTTPConnection(host)"):
            self.assertEqual(leaks(source), [], source)


if __name__ == "__main__":
    unittest.main()
