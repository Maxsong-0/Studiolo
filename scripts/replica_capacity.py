"""副本四条协议上限的用量、到线提醒的文字与去重：纯逻辑，不读写文件、不发通知（T-217）。

为什么在 Mac 端也算一遍：agent 路牌第 2 节的容量提醒（crates/supply/src/replica/mod.rs 的 capacity_notice_for）
只给读得到副本里每个条目的接入方，生产上可能谁也收不到；越线那天 Mac 同步整批失败、AI 读到的资料停在越线前，
vault BUG-039 那次 27 天后才发现。所以 Mac 同步端每轮捕获时自己算同样四条，到线就给 Mac 用户发系统通知，
不经 agent、不看任何接入方的读授权。发通知、读写去重状态这些副作用在 replica_client.py。
"""
from replica_common import (MAX_ENTRIES, MAX_MANIFEST, MAX_NOTES_TOTAL, MAX_TOTAL, Invalid, integer,
                            markdown, require)

# 任一条上限用到这个百分比起提醒。与 crates/supply/src/replica/mod.rs 的 CAPACITY_WARN_PERCENT 相等，
# 由 test-replica-boundaries.py 守卫：两边口径一致，Mac 通知和 agent 路牌提醒同一天出现。
CAPACITY_WARN_PERCENT = 80
DAY_MS = 24 * 60 * 60 * 1000
# 提醒级别，从轻到重：到线（≥80%，同步照常）、越线（这一轮捕获整批失败）。
LEVELS = ("warn", "exceeded")
# 同一级别多久再提醒一次：到线时每周一次（按每月约 5 MiB 的增长，从 80% 到越线约两个月）；
# 越线时同步已经停了、资料天天在变旧，每天一次。只提醒一次的话，用户错过那一条就又是 BUG-039。
REMIND_AFTER_MS = {"warn": 7 * DAY_MS, "exceeded": DAY_MS}
MIB = 1024 * 1024

# 四条上限：键（也是同步结果 capacity 里的字段名）、给人看的名字、按字节还是按个数；顺序同 agent 路牌的提醒。
# 条目按 Mac 捕获实际拦的口径数，连隐藏项一起（replica_capture.scan 的 inspected），比 agent 只数可见项的多：
# Mac 先在这个数上越线，按它提醒才有提前量。可见条目数另外写进结果（capacity.entries.visible）。
GAUGES = (
    ("notes_bytes", "Markdown 合计", True),
    ("total_bytes", "文件总量", True),
    ("entries", "条目（含隐藏项）", False),
    ("manifest_bytes", "清单", True),
)
LIMITS = {"notes_bytes": MAX_NOTES_TOTAL, "total_bytes": MAX_TOTAL, "entries": MAX_ENTRIES,
          "manifest_bytes": MAX_MANIFEST}
ADVICE = ("调高两端上限（scripts/replica_common.py 与 crates/supply/src/replica/protocol.rs，"
          "agent 先部署、Mac 同步包紧跟）或精简白名单，见 docs/m3/replica-freshness.md。")


class LimitExceeded(Invalid):
    """捕获时某条上限越线；gauge 是 GAUGES 里的键。仍是 Invalid：不认得它的调用方照旧按校验失败处理。"""

    def __init__(self, gauge):
        super().__init__("validation_failed")
        self.gauge = gauge


def within(gauge, used, limit):
    """used 超过 limit 就以这条上限的名义整批失败。"""
    if used > limit:
        raise LimitExceeded(gauge)


def measure(manifest_raw, entries, inspected):
    """一次捕获的用量。字节三条按清单算，口径同 agent 的 replica::Usage（verify.rs）；条目见 GAUGES 的说明。"""
    files = [entry for entry in entries if entry["kind"] == "file"]
    return {"manifest_bytes": len(manifest_raw), "entries": inspected, "visible_entries": len(entries),
            "total_bytes": sum(entry["bytes"] for entry in files),
            "notes_bytes": sum(entry["bytes"] for entry in files if markdown(entry["path"]))}


def report(usage):
    """四条各用了多少、上限多少、百分之几（向下取整，同 agent）；原样写进同步结果。"""
    result = {key: {"used": usage[key], "limit": LIMITS[key], "percent": usage[key] * 100 // LIMITS[key]}
              for key, _, _ in GAUGES}
    result["entries"]["visible"] = usage["visible_entries"]
    return result


def levels(capacity):
    """这一轮四条各自的提醒级别：到线为 warn，没到为 None（捕获成功就没有越线）。"""
    return {key: "warn" if capacity[key]["percent"] >= CAPACITY_WARN_PERCENT else None for key, _, _ in GAUGES}


def show(in_bytes, value):
    return f"{value / MIB:.1f} MiB" if in_bytes else str(value)


def warn_notice(capacity):
    """到线的逐条列出：(标题, 正文)；一条都没到线时为 None。正文格式同 agent 路牌的提醒。"""
    reached = [f"{name} {show(in_bytes, capacity[key]['used'])}，上限 {show(in_bytes, capacity[key]['limit'])}"
               f" 的 {capacity[key]['percent']}%"
               for key, name, in_bytes in GAUGES if capacity[key]["percent"] >= CAPACITY_WARN_PERCENT]
    if not reached:
        return None
    return ("Studiolo 副本快到上限",
            "；".join(reached) + "。越过任一条上限后 Mac 同步整批失败、AI 读到的资料停在越线前（vault BUG-039）。趁早"
            + ADVICE)


def exceeded_notice(gauge):
    """越线那一条：(标题, 正文)。捕获停在越线处，用量只知道超了，所以只报上限。"""
    name, in_bytes = next((name, in_bytes) for key, name, in_bytes in GAUGES if key == gauge)
    return ("Studiolo 副本超上限，同步已停",
            f"{name}超过上限 {show(in_bytes, LIMITS[gauge])}，Mac 同步整批失败、AI 读到的资料停在越线前"
            "（vault BUG-039）。" + ADVICE)


def plan(state, observed, now_ms):
    """这一轮发不发提醒，以及发完（或不发）之后要记下的状态：返回 (发不发, 新状态)。

    state 是上一轮记下的 {键: {"level", "notified_at_ms"}}；observed 是这一轮知道的几条 {键: 级别或 None}，
    捕获中途越线时只知道越线那一条，别的照旧留着。某条第一次到线、级别升了（到线 → 越线）、或离上次提醒
    满了这一级的间隔（时钟往回拨也算满）就发；发的时候这一轮到线的每一条都记成现在。没到线的那条删掉，
    下次再到线重新提醒；级别降了（调高上限后从越线回到到线）不发，换成新级别、提醒时间照旧。
    """
    def due(key, level):
        previous = state.get(key)
        if previous is None or LEVELS.index(level) > LEVELS.index(previous["level"]):
            return True
        return not previous["notified_at_ms"] <= now_ms < previous["notified_at_ms"] + REMIND_AFTER_MS[level]

    send = any(due(key, level) for key, level in observed.items() if level is not None)
    result = forget(state, observed)
    for key, level in observed.items():
        if level is not None:
            result[key] = {"level": level, "notified_at_ms": now_ms if send else state[key]["notified_at_ms"]}
    return send, result


def forget(state, observed):
    """这一轮退到线下的几条从状态里删掉，其余照旧。发提醒没发成时也要记下这一步，
    否则那条下次再到线会被当成「刚提醒过」压掉。"""
    return {key: item for key, item in state.items() if not (key in observed and observed[key] is None)}


def parse_state(value):
    """去重状态文件的内容；形状不对抛 Invalid。"""
    require(isinstance(value, dict) and set(value) == {"schema_version", "gauges"})
    require(integer(value["schema_version"]) and value["schema_version"] == 1)
    gauges = value["gauges"]
    require(isinstance(gauges, dict) and set(gauges) <= set(LIMITS))
    for item in gauges.values():
        require(isinstance(item, dict) and set(item) == {"level", "notified_at_ms"})
        require(item["level"] in LEVELS and integer(item["notified_at_ms"]) and item["notified_at_ms"] >= 0)
    return gauges


def dump_state(gauges):
    return {"schema_version": 1, "gauges": gauges}
