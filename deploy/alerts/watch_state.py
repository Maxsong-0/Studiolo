"""看门狗纯状态机：连续两次失败、成功投递后冷却，失败下一轮重试。

冷却有两种：A01～A06 跟着情节走，代号一换就清（600 秒）；A07、A08（同步两项，按天提醒）每个代号单独记
上次发送成功的时刻，换代号也不清——中间被更高优先级的代号打断、或 A07／A08 互换再回来，一天内都不再发；
看到正常（无故障）才清，再出事算新的一条（T-224 复查 F1）。
"""

CODES = {"A01": "critical", "A02": "critical", "A03": "warning", "A04": "warning", "A06": "critical",
         "A07": "warning", "A08": "warning"}
# 发送成功后隔多久再报：同步两项（A07 没对上、A08 删除被挡）按天提醒（用户 2026-11-22 定），其余 600 秒。
REPEAT_SECONDS = {"A07": 86400, "A08": 86400}
DEFAULT_REPEAT_SECONDS = 600
PROBE_FIELDS = {"services_ok", "clock_ok", "mcp_ok", "replica_stale", "memory_high", "sync_unaligned", "sync_deletes_blocked"}
# 状态第 2 版加了 daily_sent；第 1 版的状态文件读不进（F4 还没装上生产，没有要迁的旧状态）。
SCHEMA = 2
FIELDS = {"schema", "observed", "code", "failures", "episode", "round", "sent", "daily_sent"}


def initial(now):
    return dict(schema=SCHEMA, observed=now, code=None, failures=0, episode=0, round=0, sent=None,
                daily_sent=dict.fromkeys(REPEAT_SECONDS))


def validate(state):
    if not isinstance(state, dict) or set(state) != FIELDS or type(state["schema"]) is not int or state["schema"] != SCHEMA:
        raise ValueError("state_invalid")
    if state["code"] not in (None, *CODES):
        raise ValueError("state_invalid")
    for key in ("observed", "failures", "episode", "round"):
        if type(state[key]) is not int or not 0 <= state[key] < 2**63:
            raise ValueError("state_invalid")
    if state["sent"] is not None and (type(state["sent"]) is not int or not 0 <= state["sent"] <= state["observed"]):
        raise ValueError("state_invalid")
    daily = state["daily_sent"]
    if not isinstance(daily, dict) or set(daily) != set(REPEAT_SECONDS):
        raise ValueError("state_invalid")
    for sent in daily.values():
        if sent is not None and (type(sent) is not int or not 0 <= sent <= state["observed"]):
            raise ValueError("state_invalid")


def observe(previous, code, now):
    validate(previous)
    if type(now) is not int or not 0 <= now < 2**63 - 1 or code not in (None, *CODES):
        raise ValueError("observation_invalid")
    state = dict(previous, daily_sent=dict(previous["daily_sent"]))
    rollback = now < state["observed"]
    if rollback:
        code = "A06"
    state["observed"] = now
    if code != state["code"] or rollback:
        state.update(code=code, failures=0, sent=None, episode=state["episode"] + 1)
    # 恢复正常算这一阵结束；时钟回拨后记着的时刻都在「现在」之后、说不清隔了多久。两种都清掉按天的记录。
    if code is None or rollback:
        state["daily_sent"] = dict.fromkeys(REPEAT_SECONDS)
    if code is None:
        state.update(failures=0, sent=None)
        return state, False
    state["failures"] = min(state["failures"] + 1, 2)
    last = state["daily_sent"][code] if code in REPEAT_SECONDS else state["sent"]
    repeat = REPEAT_SECONDS.get(code, DEFAULT_REPEAT_SECONDS)
    fire = rollback or (state["failures"] >= 2 and (last is None or now - last >= repeat))
    if fire:
        state["round"] += 1
    validate(state)
    return state, fire


def delivered(state, now):
    """发送成功后记账：本情节的冷却起点，按天提醒的代号另记一份、换代号也留着。"""
    validate(state)
    updated = dict(state, sent=now, daily_sent=dict(state["daily_sent"]))
    if updated["code"] in REPEAT_SECONDS:
        updated["daily_sent"][updated["code"]] = now
    validate(updated)
    return updated


def failure_code(probe):
    if not isinstance(probe, dict) or set(probe) != PROBE_FIELDS or any(type(value) is not bool for value in probe.values()):
        return "A01"
    if not probe["clock_ok"]:
        return "A06"
    if not probe["services_ok"] or not probe["mcp_ok"]:
        return "A02"
    if probe["replica_stale"]:
        return "A03"
    if probe["memory_high"]:
        return "A04"
    # 删除被挡时同步本来就整批暂停，原因已知：报 A08，不再另报 A07。
    if probe["sync_deletes_blocked"]:
        return "A08"
    if probe["sync_unaligned"]:
        return "A07"
    return None
