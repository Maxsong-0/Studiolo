"""169 本机只读探针：固定三个单元、对时、内存水位、复用会话的 MCP 握手与 Bridge 同步状态回执。"""
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import time
import uuid
import urllib.request

from private_state import closed_object, locked_state, private_directory, private_file, read_secret, reject_constant
from bounded_process import output as command_output
from residency import child_environment

UNITS = ("studiolo-agent.service", "studiolo-human.service", "studiolo-tunnel.service")
TOKEN = "/home/maxsong/.config/studiolo-alert/agent-token"
STATE = "/home/maxsong/.local/share/studiolo-alert/probe"
RECEIPT = "/home/maxsong/.local/share/studiolo-alert/probe-receipt"
# 人侧每轮写的同步状态回执（T-224，契约 docs/contracts/sync-status-receipt.md）；人侧只写事实，两条线在这里。
SYNC_STATUS = "/var/lib/studiolo-owner/bridge/plane/sync-status.json"
SYNC_STATUS_FIELDS = {"schema", "written_at", "last_aligned_at", "deletes_blocked", "deletes_checked_at"}
SYNC_STATUS_MAX_BYTES = 4096
# 两边没对上满 6 小时报 A07（用户 2026-11-22 定）。
UNALIGNED_AFTER = 6 * 3600
# 删除守卫 1 小时内证明过，「删除被挡」才算数：人侧卡住、或轮次走不到守卫时，旧的被挡数不能一直盖住停摆
# （按证明时刻判，不按写出时刻：回执每轮都会刷新写出时刻）。证明时刻是那一轮开始的时刻、轮末才换新，
# 换新前它已经旧了「上一轮耗时＋轮间隔 300 秒＋这一轮耗时」：169 实测一轮 7～70 秒，留到 1 小时，
# 一轮慢到 27 分钟也不会让 A08 每轮掉一次再重报（T-224 复查第 2 轮 R2-F1；原来是 15 分钟，一轮 5 分钟就会掉）。
STATUS_FRESH = 3600


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


def whole_int(value, low, high):
    # bool 是 int 的子类，不能冒充整数。
    return type(value) is int and low <= value <= high


def read_sync_status(path=SYNC_STATUS):
    """读人侧同步状态回执；不是本人 0600 单链接、超限、不闭合或数值不对一律抛错，不纠正、不猜。"""
    parent = private_directory(str(Path(path).parent))
    try:
        fd = os.open(Path(path).name, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC | os.O_NONBLOCK, dir_fd=parent)
        with os.fdopen(fd, "rb") as source:
            private_file(source.fileno())
            raw = source.read(SYNC_STATUS_MAX_BYTES + 1)
    finally:
        os.close(parent)
    if len(raw) > SYNC_STATUS_MAX_BYTES:
        raise ValueError("sync_status_too_large")
    value = json.loads(raw, object_pairs_hook=closed_object, parse_constant=reject_constant)
    if not isinstance(value, dict) or set(value) != SYNC_STATUS_FIELDS or not whole_int(value["schema"], 1, 1):
        raise ValueError("sync_status_invalid")
    written, aligned = value["written_at"], value["last_aligned_at"]
    blocked, checked = value["deletes_blocked"], value["deletes_checked_at"]
    if not whole_int(written, 0, 2**63 - 1):
        raise ValueError("sync_status_invalid")
    if aligned is not None and not whole_int(aligned, 0, written):
        raise ValueError("sync_status_invalid")
    # 被挡数和证明它的那一轮时刻成对：要么都空，要么都有。
    if (blocked is None) != (checked is None):
        raise ValueError("sync_status_invalid")
    if blocked is not None and not (whole_int(blocked, 0, 2**64 - 1) and whole_int(checked, 0, written)):
        raise ValueError("sync_status_invalid")
    return value


def sync_flags(status, now):
    """(两边没对上满 6 小时, 删除被挡)；回执读不到、坏了或写出时刻在未来，按没对上报、不算被挡。"""
    if status is None or status["written_at"] > now:
        return True, False
    # 读进来时已核过 aligned、checked ≤ written，上面又排除了 written 在未来，这里只剩距今多久。
    aligned = status["last_aligned_at"]
    unaligned = aligned is None or now - aligned >= UNALIGNED_AFTER
    blocked, checked = status["deletes_blocked"], status["deletes_checked_at"]
    proven = checked is not None and now - checked <= STATUS_FRESH
    return unaligned, proven and blocked > 0


def probe():
    result = dict(services_ok=False, clock_ok=False, mcp_ok=False, replica_stale=True, memory_high=True,
                  sync_unaligned=True, sync_deletes_blocked=False)
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
    try:
        status = read_sync_status()
    except Exception:
        status = None
    result["sync_unaligned"], result["sync_deletes_blocked"] = sync_flags(status, int(time.time()))
    return result
