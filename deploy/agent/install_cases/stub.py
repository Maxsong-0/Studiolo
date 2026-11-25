"""替身命令：按被调用的名字扮 systemctl、sudo、curl、sleep、date、find、id。

状态在 STUB_STATE 指的 JSON 里：现在跑的是新还是旧程序、进程号、重启次数、钟走了几秒。
`scenario` 决定哪一步出什么错。
systemctl 照真的两条：单元改了要 daemon-reload 才生效；对已在跑的单元 start 是空操作。
"""

import json
import os
import shutil
import signal
import subprocess
import sys
import time
from pathlib import Path

STATE = Path(os.environ["STUB_STATE"])
NAME = Path(sys.argv[0]).name
ARGS = sys.argv[1:]
state = json.loads(STATE.read_text())
scenario = state["scenario"]


def save():
    draft = STATE.with_suffix(".new")
    draft.write_text(json.dumps(state))
    os.replace(draft, STATE)


def finish(code=0):
    save()
    sys.exit(code)


def running_new():
    return state["program"] == "new"


def signal_script(*names):
    """给正在跑的安装脚本（替身的父进程）发信号。"""
    for name in names:
        os.kill(os.getppid(), getattr(signal, "SIG" + name))


def outer_group():
    """外层的进程组号：台架在起脚本之后才写，等它出现。"""
    path = Path(os.environ["STUB_OUTER"])
    for _ in range(200):
        if path.exists():
            return int(path.read_text())
        time.sleep(0.05)
    sys.exit("没等到外层的进程组号")


def sudo_install():
    paths = []
    skip = False
    for arg in ARGS[1:]:
        if skip:
            skip = False
        elif arg in ("-o", "-g", "-m"):
            skip = True
        else:
            paths.append(arg)
    source, target = paths
    new = state["NEW"] in Path(source).read_text(encoding="utf-8")
    if scenario == "put-new-fails" and new:
        sys.exit(1)
    if scenario == "put-back-fails" and not new and target.endswith(".t237-writing"):
        sys.exit(1)
    shutil.copyfile(source, target)
    if scenario == "term-while-putting" and new:
        signal_script("TERM")
    if scenario == "garbled-unit" and new:
        with open(target, "a", encoding="utf-8") as unit:
            unit.write("# 别人加的\n")
    sys.exit(0)


def sudo_readlink():
    if scenario == "unknown-old-program" and not running_new():
        print("/somewhere/else (deleted)")
        sys.exit(0)
    release = state["NEW"] if running_new() and scenario != "wrong-program" else state["OLD"]
    print(f"{state['ROOT']}/{release}/bin/studiolo-agent")
    sys.exit(0)


def sudo():
    state["sudo"].append(" ".join(ARGS))
    save()
    command = ARGS[0]
    if command == "install":
        sudo_install()
    if command == "readlink":
        sudo_readlink()
    if command == "chown":
        if scenario == "term-while-freezing":
            signal_script("TERM")
        sys.exit(0)
    if command == "mv" and scenario == "rename-fails":
        sys.exit(1)
    skipped = {
        "program-mode": command == "chmod",
        "file-mode": command == "find" and "f" in ARGS,
        "directory-mode": command == "find" and "d" in ARGS,
    }
    if skipped.get(scenario):
        sys.exit(0)
    if scenario == "swapped-while-freezing" and command == "chmod":
        other = Path(state["ROOT"], state["NEW"], "other")
        other.chmod(0o600)
        other.write_text("z")
        other.chmod(0o440)
    if ARGS[:2] == ["systemctl", "restart"] and scenario in ("term-while-restarting", "hup-while-restarting"):
        signal_script(scenario.split("-")[0].upper())
        time.sleep(1)
    if ARGS[:2] == ["systemctl", "stop"] and scenario == "signals-while-putting-back":
        signal_script("TERM", "HUP")
    code = subprocess.run(ARGS).returncode
    after = json.loads(STATE.read_text())
    after["sudo"].append("done:" + " ".join(ARGS))
    state.clear()
    state.update(after)
    finish(code)


def date():
    if ARGS == ["+%s"]:
        print(int(time.time()) + state["tick"])
        sys.exit(0)
    sys.exit(subprocess.run(["/usr/bin/date", *ARGS]).returncode)


def find():
    # 测试不是 root，属主改不了：只有点名的那条用例报属主不对。
    if "-user" in ARGS:
        if scenario == "owner":
            print("x")
        sys.exit(0)
    sys.exit(subprocess.run(["/usr/bin/find", *ARGS]).returncode)


def sleep():
    """不真睡，只让替身的钟走；新程序在跑的时候按钟点出错。"""
    state["tick"] += int(ARGS[0])
    state["session"] = os.getsid(0)
    tick = state["tick"]
    if not running_new():
        finish()
    if (scenario == "drops-at-14" and tick >= 14) or (scenario == "drops-at-10" and tick >= 10):
        state["running"] = False
    if scenario == "crash-loop" and tick % 4 == 0:
        state["pid"] += 1
        state["restarts"] += 1
    if scenario == "restarted-same-pid" and tick >= 5:
        state["restarts"] = 1
    if scenario == "new-pid" and tick >= 5:
        state["pid"] += 1
    if scenario == "deactivating" and tick >= 5:
        state["deactivating"] = True
    if scenario == "start-limit" and tick >= 3:
        state.update(running=False, failed=True, restarts=2)
    save()
    if tick == 5 and scenario in ("TERM", "HUP", "INT", "QUIT"):
        signal_script(scenario)
    if tick == 5 and scenario.startswith("outer-"):
        os.killpg(outer_group(), getattr(signal, "SIG" + scenario.removeprefix("outer-")))
    sys.exit(0)


# 探测必须带齐这些参数才有应答：不读配置、不走代理、不跟跳转、有时限。
CURL_FLAGS = ("--noproxy *", "--proxy  ", "--proto =http", "--max-redirs 0", "--max-time 2", "--write-out %{http_code}")


def curl():
    path = ARGS[-1].split(":4400", 1)[1]
    flags = " ".join(ARGS[:-1])
    careful = ARGS[0] == "--disable" and all(flag in flags for flag in CURL_FLAGS)
    silent = (scenario == "never-answers" and running_new()) or (scenario == "old-never-answers" and not running_new() and state["starts"])
    code = "000"
    if careful and state["running"] and not silent:
        code = "401" if path == "/mcp" or (running_new() and scenario != "answers-404") else "404"
    if scenario == "slow-answers" and running_new():
        state["tick"] += 2
        save()
        code = "000"
    if scenario == "mcp-200" and path == "/mcp":
        code = "200"
    if scenario == "connect-401" and path != "/mcp":
        code = "401"
    print(code, end="")
    sys.exit(0 if code != "000" else 7)


def systemctl_show():
    if "--value" not in ARGS:
        print("ActiveState=active")
        if scenario == "term-when-done":
            signal_script("TERM")
        finish(1 if scenario == "print-fails" else 0)
    if state["failed"]:
        active = "failed"
    elif state["deactivating"]:
        active = "deactivating"
    else:
        active = "active" if state["running"] else "activating"
    values = {"ActiveState": active, "MainPID": state["pid"] if state["running"] else 0, "NRestarts": state["restarts"]}
    print(values[ARGS[ARGS.index("-p") + 1]])
    finish()


def systemctl_start(command):
    if command == "start" and state["running"]:
        finish()
    if state["failed"]:
        finish(1)
    state.update(program=state["loaded"], starts=state["starts"] + 1, running=False)
    if scenario == "restart-fails" and running_new():
        finish(1)
    state.update(running=True, pid=state["pid"] + 10, restarts=0)
    finish()


def systemctl():
    command = ARGS[0]
    state["calls"].append(" ".join(ARGS))
    if command == "is-active":
        down = {"agent-down": "studiolo-agent", "human-down": "studiolo-human"}.get(scenario)
        finish(0 if state["running"] and ARGS[-1] != down else 3)
    if command == "show":
        systemctl_show()
    if command == "daemon-reload":
        if scenario == "reload-fails" and not state["reload_failed"]:
            state["reload_failed"] = True
            finish(1)
        state["loaded"] = "new" if state["NEW"] in Path(state["UNIT"]).read_text(encoding="utf-8") else "old"
        finish()
    if command == "stop":
        state.update(running=False, deactivating=False)
        finish()
    if command == "reset-failed":
        state.update(failed=False, restarts=0)
        finish()
    if command in ("restart", "start"):
        systemctl_start(command)
    sys.exit(9)


def identity():
    print("maxsong")


{"sudo": sudo, "id": identity, "date": date, "find": find, "sleep": sleep, "curl": curl, "systemctl": systemctl}[NAME]()
