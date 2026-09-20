"""MCP stdio facade, including Claude's documented channel notification contract."""
import json
import sys
import threading
import urllib.request

from .config import private_read


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *_args, **_kwargs):
        return None


def run(cfg, name, channel=False):
    client = next((c for c in cfg["clients"] if c["id"] == name), None)
    if client is None or channel != (client["provider"] == "claude"):
        raise ValueError("MCP 客户端登记与 Channel 模式不符")
    base = f"http://127.0.0.1:{cfg['port']}/v1/client/{name}/"
    token = private_read(client["bridge_token_file"]).strip()
    output_lock = threading.Lock()
    stop = threading.Event()
    initialized = False

    def emit(message):
        with output_lock:
            sys.stdout.write(json.dumps(message, ensure_ascii=False) + "\n")
            sys.stdout.flush()

    def request(action, body):
        opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), NoRedirect())
        req = urllib.request.Request(base + action, json.dumps(body).encode(), {
            "Authorization": "Bearer " + token, "Content-Type": "application/json",
        })
        with opener.open(req, timeout=25) as response:
            raw = response.read(1048577)
            if len(raw) > 1048576:
                raise ValueError("本机响应过大")
            return json.loads(raw)

    def poll():
        while not stop.is_set():
            try:
                for event in request("poll", {})["events"]:
                    if not channel:
                        continue
                    emit({"jsonrpc": "2.0", "method": "notifications/claude/channel", "params": {
                        "content": event["content"], "meta": {"task_id": event["task_id"], "delivery_id": event["id"]}}})
                    request("ack", {"id": event["id"]})
            except Exception:
                # Offline polling never produces a forged success or leaks a token.
                pass
            stop.wait(5)

    thread = None
    try:
        while True:
            line = sys.stdin.buffer.readline(262145)
            if not line:
                break
            if len(line) > 262144:
                raise ValueError("MCP 输入过大")
            request_id = None
            try:
                message = json.loads(line)
                request_id = message.get("id")
                method = message.get("method")
                if message.get("jsonrpc") != "2.0":
                    raise ValueError("JSON-RPC 版本无效")
                if method == "initialize":
                    capabilities = {"tools": {}}
                    if channel:
                        capabilities["experimental"] = {"claude/channel": {}}
                    result = {"protocolVersion": "2025-11-25", "capabilities": capabilities,
                        "serverInfo": {"name": "studiolo", "version": "0.1.0"},
                        "instructions": "Studiolo 本机黑板接入。通知只是工作项 ID；先 board get 获取权威任务，再 claim。遵守原生会话审批与文件权限；租约有效才写回，定期 heartbeat。必须通过 board return 提交完整结构化回执，不能把聊天回复当完成。禁止使用 API 或额外付费回退。"}
                    initialized = True
                elif method == "notifications/initialized" and initialized:
                    if thread is None:
                        thread = threading.Thread(target=poll, daemon=True)
                        thread.start()
                    continue
                elif method == "ping":
                    result = {}
                elif method in {"tools/list", "tools/call"} and initialized:
                    result = request("rpc", {"method": method, "params": message.get("params", {})})
                elif request_id is None:
                    continue
                else:
                    emit({"jsonrpc": "2.0", "id": request_id, "error": {"code": -32601, "message": "方法未开放或尚未初始化"}})
                    continue
                if request_id is not None:
                    emit({"jsonrpc": "2.0", "id": request_id, "result": result})
            except Exception:
                if request_id is not None:
                    emit({"jsonrpc": "2.0", "id": request_id, "error": {"code": -32603, "message": "Studiolo 连接或权限不可确认；刷新权威状态后再操作"}})
    finally:
        stop.set()

