"""空正文心跳；两个服务独立尝试，只输出固定结果，不回显URL、凭据或响应。"""
import os
import re
import urllib.parse
import urllib.request

# workers.dev 前面的 Cloudflare 按 urllib 默认的 Python-urllib 标识拦成 403（error code 1010），两路都显式报自己的名字。
USER_AGENT = "studiolo-alert-heartbeat/1"


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


def endpoints(environment, node):
    if node not in ("n108", "n169"):
        raise ValueError("node_invalid")
    health = environment.get("STUDIOLO_HEALTHCHECKS_URL", "")
    worker = environment.get("STUDIOLO_WORKER_HEARTBEAT_URL", "")
    token = environment.get("STUDIOLO_WORKER_HEARTBEAT_TOKEN", "")
    return [("health", health, {}), ("worker", worker, {"Authorization": "Bearer " + token})]


def valid_endpoint(kind, url, headers, node):
    parsed = urllib.parse.urlsplit(url)
    if parsed.scheme != "https" or not parsed.hostname or parsed.username or parsed.password or parsed.query or parsed.fragment:
        return False
    if kind == "health":
        return parsed.netloc == "hc-ping.com" and re.fullmatch(r"/[a-fA-F0-9-]{36}", parsed.path) is not None
    token = headers["Authorization"][7:]
    return parsed.path == "/heartbeat/" + node and bool(token) and len(token.encode()) <= 1024 and not any(char.isspace() for char in token)


def ping(node, environment=None, opener=None):
    environment = os.environ if environment is None else environment
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), NoRedirect()) if opener is None else opener
    outcomes = []
    for kind, url, headers in endpoints(environment, node):
        try:
            if not valid_endpoint(kind, url, headers, node):
                raise ValueError("heartbeat_configuration")
            request = urllib.request.Request(url, method="POST", headers={**headers, "User-Agent": USER_AGENT})
            with opener.open(request, timeout=8) as response:
                accepted = response.status in (200, 204) and len(response.read(1025)) <= 1024
            outcomes.append(accepted)
        except Exception:
            outcomes.append(False)
    return outcomes
