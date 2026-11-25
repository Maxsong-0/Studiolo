"""169 本机只读探针：固定三个单元、人侧自动重启、对时、内存水位、复用会话的 MCP 握手与 Bridge 同步状态回执。"""
import importlib.util
import json
import os
from pathlib import Path, PurePosixPath
import re
import subprocess
import sys
import time
import uuid
import urllib.request

from private_state import closed_object, locked_state, private_directory, private_file, read_secret, reject_constant
from bounded_process import output as command_output
from residency import child_environment

HUMAN_UNIT = "studiolo-human.service"
UNITS = ("studiolo-agent.service", HUMAN_UNIT, "studiolo-tunnel.service")
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
CGROUPS = Path("/sys/fs/cgroup")
CGROUP_FILE_MAX_BYTES = 65536
# A04 的两条线（T-229）。回收不掉的内存占上限满 80%：离内核 OOM 只剩两成。
MEMORY_HIGH_PERCENT = 80
# 最近 5 分钟里整组任务都因缺内存停住的时间满 10%：每分钟一张的快照看不到两次之间的突发，
# 也看不到没到 80% 却反复重读的抖动，靠这个窗口补。169 封存顶满上限时实测最高 0.15%（2026-11-24，一次完整封存每秒取一次）。
STALL_HIGH_PERCENT = 10
# A09（T-231）：人侧被 systemd 自动重启过、这一次起来还不到 1 小时（微秒）。只看 1 小时，不等人清次数：
# 看门狗一次只认一个代号，一直挂着会让同步的 A07、A08 等不到「恢复正常」。
RESTART_RECENT_USEC = 3600 * 1_000_000
RESTART_PROPERTIES = ("NRestarts", "ActiveEnterTimestampMonotonic")
# systemd 的这两个属性都是无符号整数，最长 20 位；isdecimal 会放过全角数字，这里不用它。
UNSIGNED = re.compile(r"[0-9]{1,20}")
STALL_LINE = re.compile(r"full avg10=\S+ avg60=\S+ avg300=([0-9]{1,3}\.[0-9]{2}) total=[0-9]+")


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


def cgroup_text(directory, name):
    with open(directory / name, "rb") as source:
        raw = source.read(CGROUP_FILE_MAX_BYTES + 1)
    if len(raw) > CGROUP_FILE_MAX_BYTES:
        raise ValueError("cgroup_file_too_large")
    return raw.decode("ascii")


def byte_count(text):
    # 无界的上限在内核里写作 max，不是数字，和坏值一起拒。
    if not text.isdecimal():
        raise ValueError("cgroup_count_invalid")
    return int(text)


def memory_stat(text):
    stat = {}
    for line in text.splitlines():
        key, value = line.split(" ")
        if key in stat:
            raise ValueError("cgroup_stat_duplicate")
        stat[key] = byte_count(value)
    return stat


def unreclaimable_bytes(current, stat):
    """总量减去能马上回收的（干净的文件缓存、可回收 slab）；没列到的种类一律算回收不掉。

    脏页和正在回写的页也在文件 LRU 上，落盘之前回收不掉（BUG-054 就是它们加匿名内存占满）；
    锁住的脏页不在 LRU 上，相减可能为负，已经算在总量里，按没有干净缓存算。
    """
    clean_cache = max(0, stat["inactive_file"] + stat["active_file"] - stat["file_dirty"] - stat["file_writeback"])
    unreclaimable = current - clean_cache - stat["slab_reclaimable"]
    if unreclaimable < 0:
        raise ValueError("memory_evidence_contradictory")
    return unreclaimable


def full_stall_percent(text):
    """memory.pressure 的 full avg300（百分数）；full 行没有、不止一行或写法不对都算坏证据。"""
    lines = [line for line in text.splitlines() if line.startswith("full")]
    match = STALL_LINE.fullmatch(lines[0]) if len(lines) == 1 else None
    if match is None:
        raise ValueError("memory_pressure_invalid")
    return float(match[1])


def unit_memory_high(unit, control_group):
    # 无界、缺失、读不到或互相矛盾的内存证据不能冒称在预算内；也不牵连服务在不在的结论。
    try:
        path = PurePosixPath(control_group)
        if ".." in path.parts or path.name != unit:
            raise ValueError("control_group_invalid")
        # 相对路径在 relative_to 这里就被拒。
        directory = CGROUPS / path.relative_to("/")
        limit = byte_count(cgroup_text(directory, "memory.max").strip())
        current = byte_count(cgroup_text(directory, "memory.current").strip())
        stat = memory_stat(cgroup_text(directory, "memory.stat"))
        stalled = full_stall_percent(cgroup_text(directory, "memory.pressure")) >= STALL_HIGH_PERCENT
        return stalled or unreclaimable_bytes(current, stat) * 100 >= limit * MEMORY_HIGH_PERCENT
    except (OSError, ValueError, KeyError):
        return True


def service_metrics():
    active = True
    memory_high = False
    for unit in UNITS:
        fields = dict(line.split("=", 1) for line in output([
            "systemctl", "show", unit, "--property=ActiveState,ControlGroup"
        ]).splitlines())
        active = active and fields.get("ActiveState") == "active"
        memory_high = unit_memory_high(unit, fields.get("ControlGroup", "")) or memory_high
    return active, memory_high


def recently_restarted(text, now_usec):
    """systemctl show 的两个属性 → 最近 1 小时内自动重启过；读不全、不合格或自相矛盾一律当成重启过。

    NRestarts 只数 systemd 自动拉起的，人手工启停或 reset-failed 之后归零。
    """
    fields = [line.partition("=") for line in text.splitlines()]
    values = {name: value for name, _, value in fields}
    if len(fields) != len(RESTART_PROPERTIES) or set(values) != set(RESTART_PROPERTIES):
        return True
    if not all(UNSIGNED.fullmatch(value) for value in values.values()):
        return True
    restarts, entered = (int(values[name]) for name in RESTART_PROPERTIES)
    if entered > now_usec or (restarts > 0 and entered == 0):
        return True
    return restarts > 0 and now_usec - entered < RESTART_RECENT_USEC


def human_restarted():
    text = output(["systemctl", "show", HUMAN_UNIT, "--property=" + ",".join(RESTART_PROPERTIES)])
    # 先问 systemd 再读钟：反过来，问的那一刻刚起来的会算成「在现在之后」。
    return recently_restarted(text, time.clock_gettime_ns(time.CLOCK_MONOTONIC) // 1000)


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
                  sync_unaligned=True, sync_deletes_blocked=False, human_restarted=True)
    try:
        result["services_ok"], result["memory_high"] = service_metrics()
    except Exception:
        pass
    try:
        result["human_restarted"] = human_restarted()
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
