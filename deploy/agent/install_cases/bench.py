"""台架：在临时目录里摆好单元、发布目录和替身命令，跑一次安装脚本，把结果收成一份。

setsid、flock、tail、chmod、stat、sha256sum 是真的；替身见 stub.py。只认 Linux。
"""

import hashlib
import json
import os
import pty
import re
import subprocess
import sys
import tempfile
import time
from pathlib import Path
from types import SimpleNamespace

SCRIPT = Path(__file__).parents[1] / "t237-install-agent.sh"
STUB = Path(__file__).with_name("stub.py")
STUBBED = ("sudo", "id", "find", "sleep", "date", "curl", "systemctl")
UNITS = "/etc/systemd/system"
TEXT = SCRIPT.read_text(encoding="utf-8")


def constant(name):
    return re.search(rf"(?m)^{name}=(\S+)$", TEXT).group(1)


OLD, NEW, HUMAN_RELEASE = constant("OLD"), constant("NEW"), constant("HUMAN_RELEASE")
ANSWER_SECONDS = int(constant("ANSWER_SECONDS"))


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def human_unit_text(release):
    return f"ExecStart=/x/studiolo-human-releases/{release}/launch-human.py\n"


def command(bench, name, body):
    """在替身目录里放一个只给这条用例用的命令。"""
    path = bench.stubs / name
    path.write_text("#!/bin/bash\n" + body)
    path.chmod(0o755)


def expect_current_unit(bench):
    """单元被用例改过：让脚本把现在这份当成预期的旧单元。"""
    text = bench.unit.read_text(encoding="utf-8")
    bench.pins.update(OLD_UNIT_SHA=sha(bench.unit), NEW_UNIT_SHA=hashlib.sha256(text.replace(OLD, NEW).encode()).hexdigest())


def lay_out(root):
    home, units, stubs = root / "home", root / "units", root / "stubs"
    releases = home / ".local/share/studiolo-agent-releases"
    release = releases / NEW
    replica = home / ".local/share/studiolo-agent-replica/control"
    for directory in (units, stubs, release / "bin", replica):
        directory.mkdir(parents=True)
    (release / "bin/studiolo-agent").write_text("x")
    (release / "other").write_text("y")
    (release / "SHA256SUMS").write_text("".join(f"{sha(release / name)}  {name}\n" for name in ("bin/studiolo-agent", "other")))
    (replica / "state.json").write_text("{}")
    (home / f"{NEW}-agent-tests.log").write_text("ok\nRUN-AGENT-TESTS exit=0\n")
    for name in STUBBED:
        (stubs / name).write_text(f"#!{sys.executable} -IBS\n" + STUB.read_text(encoding="utf-8"), encoding="utf-8")
        (stubs / name).chmod(0o755)
    bench = SimpleNamespace(
        home=home, units=units, stubs=stubs, releases=releases, release=release,
        unit=units / "studiolo-agent.service", human_unit=units / "studiolo-human.service",
        replica_state=replica / "state.json", tests_log=home / f"{NEW}-agent-tests.log",
        backup=home / f"studiolo-agent.service.bak-{OLD}-to-{NEW}", lock=home / ".t237-install-agent.lock",
        pins={"SUMS_SHA": sha(release / "SHA256SUMS")},
    )
    bench.unit.write_text(f"[Service]\nWorkingDirectory=/r/{OLD}\nExecStart=/r/{OLD}/bin/studiolo-agent\n")
    bench.human_unit.write_text(human_unit_text(HUMAN_RELEASE))
    expect_current_unit(bench)
    return bench


def pinned(text, bench):
    """把三个摘要换成台架上的，把单元目录换成临时目录；别的一字不动。"""
    for name, value in bench.pins.items():
        text, count = re.subn(rf"(?m)^{name}=.*$", f"{name}={value}", text)
        assert count == 1, name
    assert text.count(UNITS) == 2
    return text.replace(UNITS, str(bench.units))


def logged_exit(log, wait):
    """日志末行的退出码。外层被打断时里层还在跑，要等它写出末行。"""
    deadline = time.monotonic() + (60 if wait else 0)
    while True:
        found = re.findall(r"(?m)^T237-INSTALL exit=(\d+)$", log.read_text(encoding="utf-8"))
        if found or time.monotonic() >= deadline:
            return int(found[-1]) if found else None
        time.sleep(0.1)


def run(scenario, text, args=("--apply",), setup=None, env=None, **initial):
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        bench = lay_out(root)
        state, outer = root / "state.json", root / "outer"
        state.write_text(json.dumps({
            "scenario": scenario, "UNIT": str(bench.unit), "ROOT": str(bench.releases), "OLD": OLD, "NEW": NEW,
            "program": "old", "loaded": "old", "running": True, "failed": False, "deactivating": False, "pid": 100, "restarts": 0,
            "tick": 0, "starts": 0, "reload_failed": False, "session": None, "calls": [], "sudo": [], **initial,
        }))
        release_after = setup(bench) if setup else None
        script = root / "script.sh"
        script.write_text(pinned(text, bench), encoding="utf-8")
        unit_before = bench.unit.read_bytes()
        backup_before = bench.backup.read_bytes() if bench.backup.is_file() else None
        environment = {"PATH": f"{bench.stubs}:/usr/bin:/bin", "HOME": str(bench.home), **(env or {})}
        environment.update(STUB_STATE=str(state), STUB_OUTER=str(outer))
        process = subprocess.Popen(
            ["bash", str(script), *args], env=environment, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
            encoding="utf-8", start_new_session=True,
        )
        draft = outer.with_suffix(".new")
        draft.write_text(str(process.pid))
        os.replace(draft, outer)
        shown, _ = process.communicate(timeout=240)
        interrupted = scenario.startswith("outer-")
        logs = sorted(bench.home.glob("*-install-*.log"))
        logged = logged_exit(logs[0], interrupted) if logs else None
        if interrupted:
            time.sleep(0.5)
        after = json.loads(state.read_text())
        which = {bench.pins["OLD_UNIT_SHA"]: "old", bench.pins["NEW_UNIT_SHA"]: "new"}
        result = SimpleNamespace(
            code=process.returncode, logged=logged, logs=len(logs), shown=shown, output=logs[0].read_text(encoding="utf-8") if logs else shown,
            detached=after["session"] not in (None, process.pid), unit=which.get(sha(bench.unit), "?"),
            program=after["program"], loaded=after["loaded"], running=after["running"], sudo=after["sudo"], calls=after["calls"],
            unit_untouched=bench.unit.read_bytes() == unit_before,
            backup_is_old_unit=bench.backup.is_file() and not bench.backup.is_symlink() and sha(bench.backup) == bench.pins["OLD_UNIT_SHA"],
            backup_untouched=(bench.backup.read_bytes() if bench.backup.is_file() else None) == backup_before,
            leftover=sorted(path.name for path in bench.units.iterdir() if path not in (bench.unit, bench.human_unit)),
            unexpected_kept=Path(f"{bench.backup}.unexpected").exists(),
        )
        # 冻结过的目录不可写，先放开才删得掉。
        subprocess.run(["chmod", "-R", "u+rwX", str(bench.home)], check=True)
        if release_after:
            release_after()
        return result


def run_detached_on_terminal(text):
    """有人在终端上直接用自用参数：只看它拒不拒绝。"""
    with tempfile.TemporaryDirectory() as directory:
        script = Path(directory) / "script.sh"
        script.write_text(text, encoding="utf-8")
        master, slave = pty.openpty()
        try:
            environment = {"PATH": "/usr/bin:/bin", "HOME": directory}
            code = subprocess.run(
                ["bash", str(script), "--detached"], env=environment, stdin=slave, stdout=slave, stderr=slave, timeout=30
            ).returncode
            return code, os.read(master, 4096).decode()
        finally:
            os.close(slave)
            os.close(master)
