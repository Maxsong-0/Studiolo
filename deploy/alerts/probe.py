"""169 本机只读探针：固定三个单元、对时、内存水位与复用会话的 MCP 握手。"""
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import time
import uuid
import urllib.request

from private_state import locked_state, read_secret
from bounded_process import output as command_output
from residency import child_environment

UNITS = ("studiolo-agent.service", "studiolo-human.service", "studiolo-tunnel.service")
TOKEN = "/home/maxsong/.config/studiolo-alert/agent-token"
STATE = "/home/maxsong/.local/share/studiolo-alert/probe"
RECEIPT = "/home/maxsong/.local/share/studiolo-alert/probe-receipt"


def boot_id():
    value = Path("/proc/sys/kernel/random/boot_id").read_text().strip()
    return str(uuid.UUID(value))


def save_receipt(result, directory=RECEIPT):
    with locked_state(directory) as state:
        state.save(dict(schema=1, observed=int(time.time()), boot=boot_id(), probe=result))


def read_receipt(directory=RECEIPT):
    with locked_state(directory) as state:
        value = state.load()
    if not isinstance(value, dict) or set(value) != {"schema", "observed", "boot", "probe"} or type(value["schema"]) is not int or value["schema"] != 1:
        raise ValueError("probe_receipt_invalid")
    if type(value["observed"]) is not int or not 0 <= int(time.time()) - value["observed"] <= 120 or value["boot"] != boot_id():
        raise ValueError("probe_receipt_stale")
    return value["probe"]


def output(command):
    return command_output(command, 4, child_environment()).decode().strip()


def service_metrics():
    active = True
    memory_high = False
    for unit in UNITS:
        fields = dict(line.split("=", 1) for line in output([
            "systemctl", "show", unit, "--property=ActiveState,MemoryCurrent,MemoryMax"
        ]).splitlines())
        active = active and fields.get("ActiveState") == "active"
        # 无界或缺失的内存证据不能冒称在预算内。
        current, maximum = fields.get("MemoryCurrent", ""), fields.get("MemoryMax", "")
        if not current.isdecimal() or not maximum.isdecimal() or int(maximum) <= 0:
            memory_high = True
        elif int(current) * 10 >= int(maximum) * 8:
            memory_high = True
    return active, memory_high


def mcp_probe(token_path=TOKEN, state_path=STATE):
    path = Path(__file__).resolve().parents[2] / "scripts/check-agent-replica.py"
    spec = importlib.util.spec_from_file_location("studiolo_replica_probe", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    token = read_secret(token_path)
    with locked_state(state_path) as state:
        value = state.load()
        if value is not None and (not isinstance(value, dict) or set(value) != {"schema", "session"} or type(value["schema"]) is not int or value["schema"] != 1 or not isinstance(value["session"], str) or not 0 < len(value["session"]) <= 1024 or any(c.isspace() for c in value["session"])):
            raise ValueError("probe_state_invalid")
        client = module.Client("http://127.0.0.1:4400/mcp", token, timeout=8, max_response_bytes=65536)
        client.opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), module.CHECK.NoRedirect())
        client.session = value["session"] if value else None
        text = None
        if client.session:
            try:
                text = client.call("handshake", {})
            except module.CHECK.CheckFailed:
                client.session = None
        if not client.session:
            initial = client.rpc("initialize", {"protocolVersion": module.CHECK.PROTOCOL, "capabilities": {}, "clientInfo": {"name": "studiolo-monitor", "version": "1"}})
            if not client.session or initial.get("protocolVersion") != module.CHECK.PROTOCOL or initial.get("serverInfo", {}).get("name") != "studiolo-agent":
                raise ValueError("mcp_invalid")
            client.rpc("notifications/initialized", {}, notification=True)
            state.save({"schema": 1, "session": client.session})
            text = initial.get("instructions")
        lines = text.splitlines() if isinstance(text, str) else []
        match = module.FRESHNESS.fullmatch(lines[1]) if len(lines) > 1 else None
        if match is None:
            raise ValueError("freshness_unknown")
        receipt = module.freshness(text, "handshake", match["status"])
        return receipt["status"] == "stale"


def probe():
    result = dict(services_ok=False, clock_ok=False, mcp_ok=False, replica_stale=True, memory_high=True)
    try:
        result["services_ok"], result["memory_high"] = service_metrics()
    except Exception:
        pass
    try:
        result["clock_ok"] = output(["timedatectl", "show", "--property=NTPSynchronized", "--value"]) == "yes"
    except Exception:
        pass
    try:
        result["replica_stale"] = mcp_probe()
        result["mcp_ok"] = True
    except Exception:
        pass
    return result
