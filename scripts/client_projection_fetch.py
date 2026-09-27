"""从已认证的 HTTP 文本供给读取完整一批；不保留路牌或秘密句柄。"""
import http.client
import json
import re
import time
import urllib.error
import urllib.parse
import urllib.request

from client_projection_schema import rules_source
from replica_common import Invalid, digest, logical_path_ok, require

MAX_NOTE = 4 * 1024 * 1024
MAX_TOTAL = 16 * 1024 * 1024
MAX_SOURCES = 128
ULID = r"[0-7][0-9A-HJKMNP-TV-Z]{25}"
# 策略平面文本格式唯一来源：crates/agent-server/src/server/policy.rs `text_policy_read`。
POLICY_HEAD = re.compile(r"# 策略版本 ([a-f0-9]{12})")
POLICY_COUNT = {"rules": re.compile(r"([0-9]{1,10}) 份规则母本"),
                "skills": re.compile(r"([0-9]{1,10}) 个 skill(?:（[^\t]*）)?")}
POLICY_ROW = {"rules": re.compile(r"([^\t]+)\tv([1-9][0-9]{0,15})\t([a-f0-9]{12})"),
              "skills": re.compile(r"([a-z0-9][a-z0-9-]{0,63})\tv([1-9][0-9]{0,15})\t.*")}
POLICY_ITEM = re.compile(r"@v([1-9][0-9]{0,15}) sha256 ([a-f0-9]{64})")
# skill 单条正文是入库的规范 JSON（serde 的 Skill 形状，deny_unknown_fields）：键恰为这四个。
SKILL_KEYS = {"name", "summary", "triggers", "markdown"}
FRESHNESS = re.compile(
    r"副本状态 ready；stale=false；generation=(?P<generation>[a-f0-9]{32})；"
    r"source_digest=(?P<source_digest>[a-f0-9]{64})；"
    r"source_observed_at_ms=(?P<source_observed_at_ms>[0-9]{1,16})；"
    r"received_at_ms=(?P<received_at_ms>[0-9]{1,16})；"
    r"age_secs=(?P<age_secs>[0-9]{1,16})；max_age_secs=(?P<max_age_secs>[0-9]{1,5})"
)


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, *args, **kwargs):
        return None


def _header(headers, name):
    values = headers.get_all(name, [])
    require(len(values) == 1)
    return values[0]


def _freshness(line):
    match = FRESHNESS.fullmatch(line)
    require(match is not None)
    fields = match.groupdict()
    for key in ("source_observed_at_ms", "received_at_ms", "age_secs", "max_age_secs"):
        fields[key] = int(fields[key])
    now = int(time.time() * 1000)
    observed, received = fields["source_observed_at_ms"], fields["received_at_ms"]
    require(0 < observed <= received <= now + 30_000)
    require(60 <= fields["max_age_secs"] <= 86_400)
    require(fields["age_secs"] <= fields["max_age_secs"])
    require(max(0, now - observed) <= fields["max_age_secs"] * 1000)
    require(abs(fields["age_secs"] - max(0, now - observed) // 1000) <= 31)
    return fields


class Client:
    def __init__(self, config, token):
        require(isinstance(token, str) and 0 < len(token) <= 4096)
        require(all(33 <= ord(char) <= 126 for char in token))
        self.url = config["source_url"]
        self.token, self.session, self.identity, self.receipt = token, None, None, None
        self.latest_times = None
        self.transferred = 0
        # 明确禁用环境代理，令牌只送达配置中的同一地址；拒绝所有重定向。
        self.opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), NoRedirect())

    def get(self, name, arguments=None):
        query = urllib.parse.urlencode(arguments or {})
        require(len(query.encode()) <= 8192)
        target = self.url + "/" + name + ("?" + query if query else "")
        headers = {"Authorization": "Bearer " + self.token, "Accept": "text/plain",
                   "User-Agent": "studiolo-client-projection/1"}
        if self.session:
            headers["Studiolo-Session-Id"] = self.session
        request = urllib.request.Request(target, headers=headers, method="GET")
        limit = MAX_NOTE + 2048 if name in ("note", "policy") else MAX_NOTE
        with self.opener.open(request, timeout=15) as response:
            require(response.status == 200 and response.geturl() == target)
            require(_header(response.headers, "Content-Type").lower() == "text/plain; charset=utf-8")
            require(response.headers.get("Content-Encoding", "identity") == "identity")
            raw = response.read(limit + 1)
            require(len(raw) <= limit)
            self.transferred += len(raw)
            require(self.transferred <= 32 * 1024 * 1024)
            text = raw.decode("utf-8")
            if name == "handshake":
                session = _header(response.headers, "Studiolo-Session-Id")
                instance = _header(response.headers, "Studiolo-Instance-Id")
                require(re.fullmatch(ULID, session) and re.fullmatch(ULID, instance))
                require(self.session is None or session == self.session)
                self.session = session
                lines = text.split("\n", 5)
                require(len(lines) >= 5 and lines[0].startswith("# Studiolo 路牌（代次 "))
                # Rust 托管路牌在新鲜度之后多写一个换行；只兼容固定位置的零或一个空行。
                identity_line = 3 if lines[2] == "" else 2
                require(lines[identity_line] == "## 1 我是谁")
                match = re.fullmatch(r"类型 ([a-z0-9][a-z0-9-]{0,31}) · 实例 (" + ULID + r") · 注册 .+",
                                     lines[identity_line + 1])
                require(match is not None and match[2] == instance)
                identity = {"type": match[1], "instance": instance}
                require(self.identity is None or identity == self.identity)
                self.identity = identity
            if name == "policy":
                # 策略平面响应不带副本新鲜度行；一致性改由策略版本行与逐项版本/摘要核对。
                return text
            lines = text.split("\n", 2)
            index = 0 if name == "entries" else 1
            require(len(lines) > index)
            fresh = _freshness(lines[index])
            # 相同不可变批次允许确认时间前进；保留最早回执作为保守的新鲜度证明。
            receipt = {key: value for key, value in fresh.items() if key != "age_secs"}
            require(self.receipt is None or all(receipt[key] == self.receipt[key]
                    for key in ("generation", "source_digest", "max_age_secs")))
            times = (receipt["source_observed_at_ms"], receipt["received_at_ms"])
            require(self.latest_times is None or all(new >= old for new, old in zip(times, self.latest_times)))
            self.latest_times = times
            if self.receipt is None:
                self.receipt = receipt
            return text


def _entries(text, prefix):
    lines = text.split("\n")
    require(len(lines) >= 2)
    match = re.fullmatch(r"# 代次 [0-9]+ · ([0-9]+) 条", lines[1])
    require(match is not None)
    rows = [] if len(lines) == 3 and not lines[2] else lines[2:]
    require(int(match[1]) == len(rows) and len(rows) <= 25_000)
    result, folded = {}, set()
    for row in rows:
        fields = row.split("\t")
        require(len(fields) == 3)
        kind, path, size = fields
        logical_path_ok(path)
        require(path.startswith(prefix + "/") and kind in {"dir", "note", "attachment"})
        require(path.casefold() not in folded)
        folded.add(path.casefold())
        require(size == "" if kind == "dir" else re.fullmatch(r"[0-9]{1,16}", size))
        result[path] = (kind, None if size == "" else int(size))
    return result


def _policy_listing(text, kind):
    """策略清单：首行策略版本、次行计数，其后逐行 `名\\tv版本\\t…`；返回 (策略版本, {名: (版本, 规则短摘要)})。"""
    lines = text.split("\n")
    require(len(lines) >= 3)
    head, count = POLICY_HEAD.fullmatch(lines[0]), POLICY_COUNT[kind].fullmatch(lines[1])
    require(head is not None and count is not None)
    # 零项时服务端在计数行后留一个换行；有项时逐行无尾随换行。
    rows = [] if lines[2:] == [""] else lines[2:]
    require(int(count[1]) == len(rows) <= 25_000)
    listed = {}
    for row in rows:
        match = POLICY_ROW[kind].fullmatch(row)
        require(match is not None and match[1] not in listed)
        listed[match[1]] = (int(match[2]), match[3] if kind == "rules" else None)
    return head[1], listed


def _skill_text(name, body):
    """服务端给的 skill 正文是入库的规范 JSON；核对名字与闭集键后渲染成落盘的三段文本。

    落盘仍是「摘要：…」「触发：…」、空行加 Markdown 正文，与改前逐字节相同；摘要已按收到的
    JSON 字节核对过，这里只做渲染，摘要含换行、触发含 ` | ` 也不影响核对。
    """
    value = json.loads(body)
    require(isinstance(value, dict) and set(value) == SKILL_KEYS and value["name"] == name)
    summary, triggers, markdown = value["summary"], value["triggers"], value["markdown"]
    require(isinstance(summary, str) and isinstance(markdown, str) and isinstance(triggers, list)
            and all(isinstance(trigger, str) for trigger in triggers))
    return f"摘要：{summary}\n触发：{' | '.join(triggers)}\n\n{markdown}"


def _policy_item(client, kind, name, effective, listed, token):
    """逐项正文：策略版本行与 `policy://种类/名@v版本 sha256 摘要` 行之后的余下部分。

    两类正文都是入库字节，摘要直接按收到的字节核对；规则母本原样落盘，skill 由规范 JSON 渲染。
    """
    require(name in listed)
    parts = client.get("policy", {"kind": kind, "subject": name}).split("\n", 2)
    require(len(parts) == 3)
    head, prefix = POLICY_HEAD.fullmatch(parts[0]), "policy://" + kind + "/" + name
    item = POLICY_ITEM.fullmatch(parts[1][len(prefix):]) if parts[1].startswith(prefix) else None
    # 与清单不同的策略版本说明导出途中 effective 变化；逐项版本/摘要再防单次响应内的撕裂读。
    require(head is not None and head[1] == effective and item is not None)
    version, full, body = int(item[1]), item[2], parts[2]
    require(version == listed[name][0])
    require(len(body.encode("utf-8")) <= MAX_NOTE and digest(body.encode("utf-8")) == full)
    require(token not in body and client.session not in body)
    if kind == "rules":
        require(full[:12] == listed[name][1])
        path = "policy/rules/" + name
    else:
        body = _skill_text(name, body)
        # 渲染后再查一次：JSON 转义过的秘密在落盘文本里会还原成原样。
        require(token not in body and client.session not in body)
        path = "policy/skills/" + name + "/SKILL.md"
    return {"path": path, "body": body, "policy_version": version, "policy_sha12": full[:12]}


def _policy_sources(client, config, token):
    """先取清单再逐项取正文；规则、skills 两份清单与每份正文须同一策略版本。"""
    effective, sources = None, []
    for kind in ("rules", "skills"):
        if not config[kind]:
            continue
        version, listed = _policy_listing(client.get("policy", {"kind": kind}), kind)
        require(effective in (None, version))
        effective = version
        sources += [_policy_item(client, kind, name, effective, listed, token) for name in config[kind]]
    require(effective is not None and 0 < len(sources) <= MAX_SOURCES)
    require(sum(len(s["body"].encode("utf-8")) for s in sources) <= MAX_TOTAL)
    return effective, sources


def _selected(config, entries):
    selected = set(config["rules"])
    for root in config["skills"]:
        require(root in entries and entries[root][0] == "dir")
        require(root + "/SKILL.md" in entries)
        for path, (kind, _) in entries.items():
            if path.startswith(root + "/") and kind != "dir":
                require(kind == "note" and path.endswith(".md"))
                selected.add(path)
    require(0 < len(selected) <= MAX_SOURCES)
    for path in selected:
        require(path in entries and entries[path][0] == "note" and path.endswith(".md"))
        require(entries[path][1] is not None and entries[path][1] <= MAX_NOTE)
    require(sum(entries[path][1] for path in selected) <= MAX_TOTAL)
    return sorted(selected)


def _vault_sources(client, config, token):
    entries = {}
    for prefix in ("_规则", config["project"]):
        entries.update(_entries(client.get("entries", {"project": prefix}), prefix))
    sources, total = [], 0
    for path in _selected(config, entries):
        text = client.get("note", {"path": path})
        parts = text.split("\n", 2)
        require(len(parts) == 3)
        match = re.fullmatch(r"revision ([a-f0-9]{64})", parts[0])
        require(match is not None)
        body = parts[2]
        require(token not in body and client.session not in body)
        raw = body.encode("utf-8")
        total += len(raw)
        require(len(raw) == entries[path][1] and len(raw) <= MAX_NOTE and total <= MAX_TOTAL)
        # Revision 绑定 Vault UUID/路径，不能冒充正文摘要；manifest 另记 digest(raw)。
        sources.append({"path": path, "revision": match[1], "body": body})
    return sources


def fetch(config, token):
    """配置由调用方验证；失败只抛固定类别，不回显底层 URL、正文或凭据。"""
    try:
        client = Client(config, token)
        client.get("handshake")
        policy = rules_source(config) == "policy"
        if policy:
            effective, sources = _policy_sources(client, config, token)
        else:
            sources = _vault_sources(client, config, token)
        client.get("handshake")
        snapshot = {"identity": client.identity,
                    "freshness": {key: client.receipt[key] for key in
                                  ("generation", "source_digest", "source_observed_at_ms", "received_at_ms")},
                    "sources": sources, "generated_at_ms": int(time.time() * 1000)}
        if policy:
            snapshot.update(rules_source="policy", policy_effective=effective)
        return snapshot
    except urllib.error.HTTPError as error:
        error.close()
        raise Invalid("projection_fetch_failed") from None
    # RecursionError：skill 正文按 JSON 解析，畸形的深层嵌套同样只报固定类别。
    except (Invalid, OSError, ValueError, UnicodeError, OverflowError, RecursionError,
            http.client.HTTPException):
        raise Invalid("projection_fetch_failed") from None
