"""看门狗纯状态机：连续两次失败、成功投递后冷却，失败下一轮重试。"""

CODES = {"A01": "critical", "A02": "critical", "A03": "warning", "A04": "warning", "A06": "critical",
         "A07": "warning", "A08": "warning"}
# 发送成功后隔多久再报：同步两项（A07 没对上、A08 删除被挡）按天提醒（用户 2026-11-22 定），其余 600 秒。
REPEAT_SECONDS = {"A07": 86400, "A08": 86400}
DEFAULT_REPEAT_SECONDS = 600
PROBE_FIELDS = {"services_ok", "clock_ok", "mcp_ok", "replica_stale", "memory_high", "sync_unaligned", "sync_deletes_blocked"}
FIELDS = {"schema", "observed", "code", "failures", "episode", "round", "sent"}


def initial(now):
    return dict(schema=1, observed=now, code=None, failures=0, episode=0, round=0, sent=None)


def validate(state):
    if not isinstance(state, dict) or set(state) != FIELDS or type(state["schema"]) is not int or state["schema"] != 1:
        raise ValueError("state_invalid")
    if state["code"] not in (None, *CODES):
        raise ValueError("state_invalid")
    for key in ("observed", "failures", "episode", "round"):
        if type(state[key]) is not int or not 0 <= state[key] < 2**63:
            raise ValueError("state_invalid")
    if state["sent"] is not None and (type(state["sent"]) is not int or not 0 <= state["sent"] <= state["observed"]):
        raise ValueError("state_invalid")


def observe(previous, code, now):
    validate(previous)
    if type(now) is not int or not 0 <= now < 2**63 - 1 or code not in (None, *CODES):
        raise ValueError("observation_invalid")
    state = dict(previous)
    rollback = now < state["observed"]
    if rollback:
        code = "A06"
    state["observed"] = now
    if code != state["code"] or rollback:
        state.update(code=code, failures=0, sent=None, episode=state["episode"] + 1)
    if code is None:
        state.update(failures=0, sent=None)
        return state, False
    state["failures"] = min(state["failures"] + 1, 2)
    repeat = REPEAT_SECONDS.get(code, DEFAULT_REPEAT_SECONDS)
    fire = rollback or (state["failures"] >= 2 and (state["sent"] is None or now - state["sent"] >= repeat))
    if fire:
        state["round"] += 1
    validate(state)
    return state, fire


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
