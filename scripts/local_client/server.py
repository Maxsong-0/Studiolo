"""Authenticated loopback companion; the upstream board remains task authority."""
import fcntl
from contextlib import closing
import hashlib
import hmac
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
import sqlite3
import threading
import time
from urllib.parse import parse_qs, urlsplit

from .config import identifier, task_id
from .contract import make_board_tool
from .mcp import McpClient, McpError
from .native import notification, notify_codex, subscription_status
from .store import Store

TERMINAL = {"succeeded", "failed", "cancelled", "abandoned", "needs_review", "approved"}
READ_TOOLS = {"handshake", "list_entries", "read_note", "search", "policy_read"}
MAX_BODY = 262144


class Conflict(ValueError):
    pass


class Companion:
    def __init__(self, cfg):
        self.cfg = cfg
        root = cfg["state_directory"]
        self.lock_fd = os.open(root / "service.lock", os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW, 0o600)
        try:
            fcntl.flock(self.lock_fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except OSError:
            os.close(self.lock_fd)
            raise ValueError("该本机连接程序已在运行") from None
        self.store = Store(root / "deliveries.sqlite3")
        self.store.recover_dispatching()
        self.clients = {c["id"]: c for c in cfg["clients"]}
        self.mcp = {k: McpClient(c["mcp_endpoint"], c["mcp_token_file"], root / (k + ".session.json")) for k, c in self.clients.items()}
        self.seen = {}
        self.auth_cache = {}
        self.task_status = {}
        self.mutex = threading.RLock()
        self.stopped = threading.Event()
        self.requests_path = root / "requests.sqlite3"
        fd = os.open(self.requests_path, os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW, 0o600)
        os.close(fd)
        with closing(sqlite3.connect(self.requests_path)) as db:
            db.execute("CREATE TABLE IF NOT EXISTS creation (id TEXT PRIMARY KEY, hash TEXT NOT NULL, task TEXT)")
            db.commit()

    def close(self):
        self.stopped.set()
        os.close(self.lock_fd)

    def client(self, name):
        if name not in self.clients:
            raise ValueError("未登记该客户端")
        return self.clients[name]

    def status(self, name):
        c = self.client(name)
        now = time.monotonic()
        with self.mutex:
            cached = self.auth_cache.get(name)
            if not cached or now - cached[0] > 30:
                cached = (now, subscription_status(c))
                self.auth_cache[name] = cached
            status = dict(cached[1])
            if now - self.seen.get(name, -1000) > 20:
                status = {"ready": False, "reason": "请在目标会话连接 Studiolo MCP；Claude 还需启用 Channel"}
        messages = {
            "subscription_only_billing_declaration_required": "尚未确认关闭额外付费用量；请先在原生账户核对",
            "subscription_login_not_verified": "未确认订阅登录；不允许 API 或第三方模型认证",
            "native_auth_status_unavailable_or_keychain_denied": "原生登录状态不可读取，请检查系统钥匙串访问",
            "native_login_unavailable_or_keychain_denied": "未登录或系统钥匙串访问受限",
            "subscription_login_verified_billing_declarations_only": "订阅登录已确认；仅订阅用量声明已登记",
            "invalid_native_registration": "本机客户端登记无效",
        }
        status["reason"] = messages.get(status["reason"], status["reason"])
        return {**{k: c[k] for k in ("id", "provider", "mode", "project", "cwd")}, **status}

    def board(self, name, operation, request):
        return self.mcp[name].board(self.client(name)["project"], operation, request)

    def eligible(self, name, tid):
        item = self.board(name, "get", {"task_id": task_id(tid)})
        if not isinstance(item, dict) or item.get("uses_old_policy") is not False:
            raise Conflict("权威状态或当前策略不可确认")
        task = item.get("task", {})
        spec = task.get("spec", {})
        if (spec.get("project") != self.client(name)["project"] or spec.get("adapter") != "consumer"
                or task.get("status") not in {"draft", "queued"}):
            raise Conflict("仅能通知当前项目尚未认领的接入方工作项")
        return item

    def dispatch(self, data):
        if set(data) != {"client_id", "task_id", "request_id"}:
            raise ValueError("派发参数无效")
        name, tid, rid = data["client_id"], task_id(data["task_id"]), identifier(data["request_id"])
        self.client(name)
        with self.mutex:
            # Replayed UI requests return the existing notification, even after claim.
            for item in self.store.list():
                if item.get("request_id") == rid:
                    if item["client_id"] != name or item["task_id"] != tid:
                        raise Conflict("幂等键已用于另一个工作项")
                    return item
                if item["client_id"] == name and item["task_id"] == tid and item["state"] != "cancelled":
                    return item
            if not self.status(name)["ready"]:
                raise Conflict("本机客户端尚未就绪")
            self.eligible(name, tid)
            return self.store.enqueue(rid, name, tid)

    def create(self, data):
        if set(data) != {"client_id", "request_id", "title", "goal", "acceptance_criteria"}:
            raise ValueError("创建参数无效")
        name, rid = data["client_id"], identifier(data["request_id"])
        c = self.client(name)
        for key in ("title", "goal"):
            if not isinstance(data[key], str) or not data[key].strip() or len(data[key]) > 2048:
                raise ValueError("标题与目标需要为有界文本")
        criteria = data["acceptance_criteria"]
        if (not isinstance(criteria, list) or not 1 <= len(criteria) <= 64
                or any(not isinstance(x, str) or not x.strip() or len(x) > 2048 for x in criteria)
                or len(set(criteria)) != len(criteria)):
            raise ValueError("验收条件无效或重复")
        digest = hashlib.sha256(json.dumps(data, sort_keys=True, ensure_ascii=False).encode()).hexdigest()
        with self.mutex, closing(sqlite3.connect(self.requests_path)) as db:
            previous = db.execute("SELECT hash,task FROM creation WHERE id=?", (rid,)).fetchone()
            if previous:
                if previous[0] != digest:
                    raise Conflict("幂等键不能更换任务内容")
                if previous[1] is None:
                    raise Conflict("上次创建结果不明，请核对黑板；不会自动重复创建")
                tid = previous[1]
            else:
                if not self.status(name)["ready"]:
                    raise Conflict("本机客户端尚未就绪")
                db.execute("INSERT INTO creation(id,hash) VALUES(?,?)", (rid, digest))
                db.commit()
                spec = {"task": {"role": "author", "adapter": "consumer", "input_paths": [],
                    "allow_web": [], "allow_db": [], "retry_policy": {"max_attempts": 1, "backoff_seconds": 0, "retry_on": []},
                    "timeout": 900, "project": c["project"], "parent": None,
                    "title": data["title"], "instruction": data["goal"]},
                    "acceptance_criteria": criteria, "files_owned": [], "max_children": 0, "artifact_target": None}
                tid = task_id(self.board(name, "create", {"request_id": "local-" + rid, "spec": spec}))
                db.execute("UPDATE creation SET task=? WHERE id=?", (tid, rid))
                db.commit()
        delivery = self.dispatch({"client_id": name, "task_id": tid, "request_id": rid})
        return {"task_id": tid, "delivery": delivery}

    def deliveries(self, project):
        return [{**item, "task_status": self.task_status.get(item["id"], "unknown")}
                for item in self.store.list() if self.client(item["client_id"])["project"] == project]

    def proxy(self, name, data):
        if set(data) != {"method", "params"}:
            raise ValueError("MCP 代理参数无效")
        method, params = data["method"], data["params"]
        if method == "tools/list":
            result = self.mcp[name].call(method, params)
            return {"tools": [make_board_tool(t, self.client(name)["project"]) if t["name"] == "board" else t
                              for t in result["tools"] if t["name"] in READ_TOOLS | {"board"}]}
        if method != "tools/call" or not isinstance(params, dict):
            raise ValueError("此代理只允许读取与黑板工具")
        tool = params.get("name")
        if tool not in READ_TOOLS | {"board"}:
            raise ValueError("工具未开放")
        if tool == "board":
            args = params.get("arguments", {})
            if args.get("project") != self.client(name)["project"]:
                raise ValueError("黑板项目越界")
            op = args.get("operation")
            if op not in {"list", "get"}:
                req = args.get("request", {})
                tid = req.get("task_id") or req.get("identity", {}).get("task_id")
                if not any(x["task_id"] == tid and x["client_id"] == name and x["state"] in {"pending", "dispatching", "sent", "uncertain"} for x in self.store.list()):
                    raise Conflict("只允许认领和写回已派给本客户端的工作项")
                if op not in {"claim", "heartbeat", "handoff", "return", "cancel", "number", "batch.preview", "batch.commit", "batch.rollback", "memory.preview"}:
                    raise ValueError("黑板操作未开放")
        return self.mcp[name].call(method, params)

    def channel_poll(self, name):
        c = self.client(name)
        with self.mutex:
            self.seen[name] = time.monotonic()
        if c["provider"] != "claude" or not self.status(name)["ready"]:
            return {"events": []}
        for item in self.store.pending(name):
            self.eligible(name, item["task_id"])
            if self.store.transition(item["id"], "pending", "dispatching", "dispatching"):
                return {"events": [{"id": item["id"], "task_id": item["task_id"],
                    "content": notification(c, item["task_id"], item["id"])}]}
        return {"events": []}

    def reconcile(self):
        for item in self.store.list():
            if item["state"] in {"completed", "cancelled"}:
                continue
            try:
                view = self.board(item["client_id"], "get", {"task_id": item["task_id"]})
                status = view["task"]["status"]
                self.task_status[item["id"]] = status
                if status in TERMINAL:
                    state = "completed" if view.get("board", {}).get("receipt") and item["state"] != "pending" else "cancelled"
                    previous = item["state"]
                    if previous == "dispatching":
                        self.store.transition(item["id"], previous, "uncertain", "uncertain")
                        previous = "uncertain"
                    self.store.transition(item["id"], previous, state, state)
                elif view.get("uses_old_policy") and item["state"] == "pending":
                    self.store.transition(item["id"], "pending", "cancelled", "cancelled")
                elif item["state"] == "dispatching" and time.time() * 1000 - item["updated_ms"] > 30000:
                    self.store.transition(item["id"], "dispatching", "uncertain", "uncertain")
            except (McpError, ValueError, KeyError):
                self.task_status[item["id"]] = "unknown"
                continue

    def worker(self):
        while not self.stopped.is_set():
            try:
                self.reconcile()
                for item in self.store.pending():
                    name = item["client_id"]
                    if self.client(name)["provider"] != "codex" or not self.status(name)["ready"]:
                        continue
                    try:
                        self.eligible(name, item["task_id"])
                    except (McpError, ValueError):
                        continue
                    if self.store.transition(item["id"], "pending", "dispatching", "dispatching"):
                        result = notify_codex(self.client(name), item["task_id"], item["id"])
                        state = result["state"]
                        self.store.transition(item["id"], "dispatching", state, state)
            except Exception:
                # No raw subprocess, HTTP, or model output enters logs.
                pass
            self.stopped.wait(2)


class Handler(BaseHTTPRequestHandler):
    server_version = "StudioloLocal/1"

    def log_message(self, *_args):
        pass

    def reply(self, status, body):
        content = json.dumps(body, ensure_ascii=False).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(content)))
        self.send_header("Cache-Control", "no-store")
        self.send_header("X-Content-Type-Options", "nosniff")
        if self.headers.get("Origin") == self.server.companion.cfg["origin"]:
            self.send_header("Access-Control-Allow-Origin", self.headers["Origin"])
            self.send_header("Vary", "Origin")
            self.send_header("Access-Control-Allow-Headers", "Authorization, Content-Type")
            self.send_header("Access-Control-Allow-Methods", "GET, POST, OPTIONS")
            self.send_header("Access-Control-Allow-Private-Network", "true")
        self.end_headers()
        self.wfile.write(content)

    def do_OPTIONS(self):
        if not self.boundary():
            return
        self.reply(200, {})

    def boundary(self):
        for name in ("Host", "Origin", "Authorization", "Content-Length", "Content-Type"):
            if len(self.headers.get_all(name, [])) > 1:
                self.reply(403, {"error": "重复安全请求头"})
                return False
        if self.headers.get("Host") != f"127.0.0.1:{self.server.server_port}":
            self.reply(403, {"error": "本机 Host 无效"})
            return False
        origin = self.headers.get("Origin")
        if origin is not None and origin != self.server.companion.cfg["origin"]:
            self.reply(403, {"error": "来源未登记"})
            return False
        return True

    def do_GET(self):
        self.handle_api(False)

    def do_POST(self):
        self.handle_api(True)

    def handle_api(self, post):
        if not self.boundary():
            return
        app = self.server.companion
        url = urlsplit(self.path)
        path = url.path
        local = path.startswith("/v1/client/")
        try:
            parts = path.split("/")
            name = parts[3] if local and len(parts) == 5 else None
            expected = app.client(name)["bridge_token"] if local else app.cfg["pairing_token"]
            if not hmac.compare_digest(self.headers.get("Authorization", ""), "Bearer " + expected):
                self.reply(401, {"error": "本机配对失效"})
                return
            data = {}
            if post:
                if self.headers.get_content_type() != "application/json" or self.headers.get("Transfer-Encoding"):
                    raise ValueError("仅接受定长 JSON")
                length = int(self.headers.get("Content-Length", "0"))
                if not 0 < length <= MAX_BODY:
                    raise ValueError("请求体大小无效")
                self.connection.settimeout(10)
                data = json.loads(self.rfile.read(length))
                if not isinstance(data, dict):
                    raise ValueError("请求必须为对象")
            if local:
                if not post:
                    raise ValueError("客户端方法无效")
                action = parts[4]
                if action == "poll":
                    result = app.channel_poll(name)
                elif action == "rpc":
                    result = app.proxy(name, data)
                elif action == "ack":
                    event = app.store.get(data["id"])
                    if not event or event["client_id"] != name:
                        raise ValueError("通知身份不符")
                    result = {"accepted": app.store.transition(event["id"], "dispatching", "sent", "sent")}
                else:
                    raise ValueError("客户端入口无效")
            elif not post and path == "/v1/clients":
                result = {"clients": [app.status(k) for k in app.clients]}
            elif not post and path == "/v1/deliveries":
                result = {"deliveries": app.deliveries(parse_qs(url.query).get("project", [""])[0])}
            elif post and path == "/v1/dispatch":
                result = app.dispatch(data)
            elif post and path == "/v1/tasks":
                result = app.create(data)
            elif post and path == "/v1/cancel":
                event = app.store.get(data["id"])
                if not event or not app.store.transition(event["id"], "pending", "cancelled", "cancelled"):
                    raise Conflict("通知已投递；停止执行须使用黑板权威撤销")
                result = {"cancelled": True}
            else:
                self.reply(404, {"error": "入口不存在"})
                return
            self.reply(200, result)
        except Conflict as error:
            self.reply(409, {"error": str(error)})
        except (ValueError, KeyError, TypeError):
            self.reply(400, {"error": "请求或登记配置无效"})
        except McpError:
            self.reply(503, {"error": "黑板拒绝操作或连接不可确认，请核对权威状态"})
        except Exception:
            self.reply(503, {"error": "本机连接暂不可用"})


def serve(cfg):
    os.umask(0o077)
    app = Companion(cfg)
    http = ThreadingHTTPServer(("127.0.0.1", cfg["port"]), Handler)
    http.companion = app
    thread = threading.Thread(target=app.worker, daemon=True)
    thread.start()
    print(f"Studiolo 本机连接：http://127.0.0.1:{cfg['port']}（仅本机，需配对）", flush=True)
    try:
        http.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        http.server_close()
        app.stopped.set()
        thread.join(timeout=25)
        app.close()
