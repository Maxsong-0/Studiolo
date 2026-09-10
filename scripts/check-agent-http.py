#!/usr/bin/env python3
"""验收已部署的四个 HTTP 文本入口及拒绝门禁，不改变服务配置。
用法：python3 scripts/check-agent-http.py <HTTP(S) origin> <token 文件>
只输出已验证的新鲜度摘要。退出码 2 表示回执切换，可由调用方有限重试；其他失败为 1。
"""
import argparse
import http.client
import json
import re
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

PROJECT = "项目/Studiolo"
INDEX_PATH = PROJECT + "/00-总览/INDEX.md"
MAX_RESPONSE_BYTES = 4 * 1024 * 1024 + 2048
ULID = r"[0-7][0-9A-HJKMNP-TV-Z]{25}"
FRESHNESS = re.compile(
    r"副本状态 ready；stale=false；generation=(?P<generation>[0-9a-f]{32})；"
    r"source_digest=(?P<source_digest>[0-9a-f]{64})；"
    r"source_observed_at_ms=(?P<source_observed_at_ms>[0-9]{1,16})；"
    r"received_at_ms=(?P<received_at_ms>[0-9]{1,16})；"
    r"age_secs=(?P<age_secs>[0-9]{1,16})；max_age_secs=(?P<max_age_secs>[0-9]{1,5})"
)
ROUTES = (("handshake", {}), ("entries", {"project": PROJECT}),
          ("note", {"path": INDEX_PATH}),
          ("search", {"project": PROJECT, "text": "00-总览 INDEX.md", "mode": "path"}))


class CheckFailed(Exception):
    """只包含脚本生成的固定错误，不包含远端内容。"""


class ReceiptChanged(CheckFailed):
    """有效回执发生变化；调用方可重新验收完整一批。"""


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, *args, **kwargs):
        return None


def require(condition, message):
    if not condition:
        raise CheckFailed(message)


def header(headers, name):
    values = headers.get_all(name, [])
    require(len(values) == 1, "响应头缺失或重复")
    return values[0]


def safe_headers(headers):
    for name, expected in (("Content-Type", "text/plain; charset=utf-8"),
                           ("Cache-Control", "no-store"), ("Pragma", "no-cache"),
                           ("X-Content-Type-Options", "nosniff"),
                           ("Referrer-Policy", "no-referrer")):
        require(header(headers, name).lower() == expected, "纯文本私有响应头不符合约定")
    require(headers.get_all("Content-Encoding", ["identity"]) == ["identity"],
            "不支持压缩响应")


def freshness(text, name):
    """元数据仅允许在协议固定位置，不能使用笔记正文伪造的行。"""
    lines = text.split("\n")
    index = 1 if name in {"handshake", "note"} else 0
    require(len(lines) > index, "缺少副本新鲜度")
    match = FRESHNESS.fullmatch(lines[index])
    require(match is not None, "副本不是有效的 ready 回执")
    value = match.groupdict()
    for key in ("source_observed_at_ms", "received_at_ms", "age_secs", "max_age_secs"):
        value[key] = int(value[key])
    now = int(time.time() * 1000)
    observed, received = value["source_observed_at_ms"], value["received_at_ms"]
    require(0 < observed <= received <= now + 30_000, "副本回执时间无效")
    require(60 <= value["max_age_secs"] <= 86_400, "副本新鲜度阈值无效")
    require(value["age_secs"] <= value["max_age_secs"]
            and max(0, now - observed) <= value["max_age_secs"] * 1000
            and abs(value["age_secs"] - max(0, now - observed) // 1000) <= 31,
            "副本回执已经过期或年龄矛盾")
    return value


def content(text, name, instance):
    lines = text.split("\n")
    if name == "handshake":
        require(lines[0].startswith("# Studiolo 路牌（代次 "), "路牌标题无效")
        position = 3 if len(lines) > 2 and lines[2] == "" else 2
        require(len(lines) > position + 1 and lines[position] == "## 1 我是谁",
                "路牌身份段缺失")
        identity = re.fullmatch(r"类型 [a-z0-9][a-z0-9-]{0,31} · 实例 (" + ULID + r") · 注册 .+",
                                lines[position + 1])
        require(identity is not None and identity[1] == instance, "路牌实例身份不一致")
    elif name == "note":
        require(re.fullmatch(r"revision [0-9a-f]{64}", lines[0]) is not None
                and "Studiolo · 总览" in "\n".join(lines[2:]), "INDEX 正文或 revision 无效")
    else:
        require(len(lines) > 2 and re.fullmatch(r"# 代次 [0-9]+ · [0-9]+ 条", lines[1]),
                "条目或检索结果头无效")
        rows = lines[2:]
        require(int(lines[1].split(" · ")[1].split(" ")[0]) == len(rows), "结果数量不匹配")
        if name == "entries":
            require(any(re.fullmatch(r"note\t" + re.escape(INDEX_PATH) + r"\t[0-9]+", row)
                        for row in rows), "条目缺少 INDEX")
        else:
            require(any(re.match(re.escape(INDEX_PATH) + r":[0-9]+\t", row) for row in rows),
                    "检索没有命中 INDEX")


class Client:
    def __init__(self, origin, token):
        parsed = urllib.parse.urlsplit(origin)
        require(parsed.scheme in {"http", "https"} and parsed.hostname and parsed.username is None
                and parsed.password is None and not parsed.query and not parsed.fragment
                and parsed.path in {"", "/"} and parsed.port != 0
                and not any(ord(char) <= 32 or ord(char) == 127 for char in origin),
                "地址必须是不含凭据、路径、查询或片段的 HTTP(S) origin")
        require(0 < len(token) <= 4096 and all(33 <= ord(char) <= 126 for char in token),
                "凭据文件为空、过长或格式无效")
        self.origin, self.token = origin.rstrip("/"), token
        self.session, self.instance = None, None
        # 环境代理与重定向均关闭，防止凭据发送到另一个地址。
        self.opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), NoRedirect())

    def get(self, name, arguments, token, session, expected=200):
        query = urllib.parse.urlencode(arguments)
        target = self.origin + "/read/" + name + ("?" + query if query else "")
        headers = {"Accept": "text/plain", "User-Agent": "studiolo-http-check/1"}
        if token is not None:
            headers["Authorization"] = "Bearer " + token
        if session is not None:
            headers["Studiolo-Session-Id"] = session
        request = urllib.request.Request(target, headers=headers, method="GET")
        try:
            response = self.opener.open(request, timeout=15)
        except urllib.error.HTTPError as error:
            # 错误正文完全不读，它可能含反射的会话、凭据或资料。
            response = error
        with response:
            require(response.code == expected and response.geturl() == target,
                    "HTTP 状态或响应地址不符合预期")
            safe_headers(response.headers)
            if expected != 200:
                if expected == 401:
                    require(header(response.headers, "WWW-Authenticate").split(" ", 1)[0].lower()
                            == "bearer", "认证拒绝缺少 Bearer challenge")
                require(not response.headers.get_all("Studiolo-Session-Id", [])
                        and not response.headers.get_all("Studiolo-Instance-Id", []),
                        "拒绝响应不应返回会话身份")
                return None
            current = tuple(header(response.headers, key) for key in
                            ("Studiolo-Session-Id", "Studiolo-Instance-Id"))
            require(all(re.fullmatch(ULID, item) for item in current), "会话身份响应头无效")
            require(self.session is None or current == (self.session, self.instance),
                    "响应会话身份发生变化")
            self.session, self.instance = current
            raw = response.read(MAX_RESPONSE_BYTES + 1)
            require(len(raw) <= MAX_RESPONSE_BYTES, "响应超过有界读取上限")
            text = raw.decode("utf-8")
            value = freshness(text, name)
            content(text, name, self.instance)
            return value


def check(origin, token):
    client = Client(origin, token)
    receipts = [client.get(name, args, token, client.session) for name, args in ROUTES]
    reference = receipts[0]
    if any(any(value[key] != reference[key] for key in reference if key != "age_secs")
           for value in receipts[1:]):
        raise ReceiptChanged("副本回执发生切换，请重新验收完整一批")
    # 四入口都必须独立检查认证，后三入口还必须要求已握手会话与白名单。
    bad_token = "sag1_" + "0" * 64
    if bad_token == token:
        bad_token = "sag1_" + "1" * 64
    for name, args in ROUTES:
        for credential in (None, bad_token):
            client.get(name, args, credential, client.session, 401)
    for name, args in ROUTES[1:]:
        client.get(name, args, token, None, 401)
    for name, args in (("entries", {"project": "School"}),
                       ("note", {"path": "School/x.md"}),
                       ("search", {"project": "School", "text": "x"})):
        client.get(name, args, token, client.session, 400)
    summary = {key: value for key, value in reference.items() if key != "age_secs"}
    summary.update({"check": "passed", "status": "ready", "stale": "false",
                    "responses_checked": 18, "content_responses": 4, "denials_checked": 14,
                    "age_secs_min": min(value["age_secs"] for value in receipts),
                    "age_secs_max": max(value["age_secs"] for value in receipts)})
    print(json.dumps(summary, ensure_ascii=False, sort_keys=True))


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("base_origin")
    parser.add_argument("token_path")
    args = parser.parse_args(argv)
    try:
        with open(args.token_path, encoding="utf-8") as token_file:
            token = token_file.read(4097)
        require(len(token) <= 4096, "凭据文件超过读取上限")
        check(args.base_origin, token.strip())
    except ReceiptChanged as error:
        print(f"HTTP 验收失败：{error}", file=sys.stderr)
        return 2
    except CheckFailed as error:
        print(f"HTTP 验收失败：{error}", file=sys.stderr)
        return 1
    except (OSError, ValueError, OverflowError, http.client.HTTPException):
        print("HTTP 验收失败：无法读取凭据或完成 HTTP 文本读取。", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
