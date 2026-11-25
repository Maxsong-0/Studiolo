"""用例：每条是 (名字, 判定)；判定收脚本原文，跑一次或几次，返回过没过。改错核对按名字点用例。"""

import fcntl
import hashlib
import os

from .bench import ANSWER_SECONDS, HUMAN_RELEASE, NEW, OLD, command, expect_current_unit, human_unit_text, run, run_detached_on_terminal, sha

FIRST_STEADY = 'steady "$NEW" /connect/v1/board ||'
LAST_LINE = "echo 'T-237 agent 已换上并在运行。'"


def replaced(text, old, new):
    assert text.count(old) == 1, old
    return text.replace(old, new)


# ---- 结局的判据


def restarted(result):
    return any(call.split()[0] in ("stop", "start", "restart") for call in result.calls)


def installed(result, code=0):
    """换好了：新单元、新程序在跑，没换回过，而且是脱离终端跑的。外层被打断时看日志里的退出码。"""
    shown = "输出在" in result.shown and (code != 0 or "T237-INSTALL exit=0" in result.shown)
    return (result.logged == 0 and result.code == code and shown and result.detached and result.unit == "new" and result.program == "new"
            and result.running and result.backup_is_old_unit and not result.leftover and "已换回" not in result.output)


def rolled_back(result):
    return (result.logged == 1 and result.code == 1 and result.unit == "old" and result.program == "old" and result.loaded == "old"
            and result.running and result.backup_is_old_unit and not result.leftover and "已换回" in result.output)


def stopped(result, why):
    """停下了，没换过单元也没重启；冻结可能做过。"""
    return (result.code == 1 and result.logged == 1 and result.unit_untouched and not restarted(result)
            and why in result.output and "已换回" not in result.output)


def untouched(result, why):
    """停在前置检查：连 sudo 都没用过。"""
    return stopped(result, why) and "停下：" in result.output and not result.sudo


def stuck(result):
    return result.code == 3 and result.logged == 3 and "换回没成" in result.output


def restart_finished_before_stop(result):
    """信号来的时候重启还没做完：要等它做完，才开始换回。"""
    done = [i for i, call in enumerate(result.sudo) if call == "done:systemctl restart studiolo-agent"]
    stop = [i for i, call in enumerate(result.sudo) if call == "systemctl stop studiolo-agent"]
    return len(done) == 1 and len(stop) == 1 and done[0] < stop[0]


# ---- 一条用例怎么写


def refuses(name, why, setup=None, scenario="good", **initial):
    return name, lambda text: untouched(run(scenario, text, setup=setup, **initial), why)


def halts(name, scenario, why):
    return name, lambda text: stopped(run(scenario, text), why)


def installs(name, scenario="good", code=0, **options):
    return name, lambda text: installed(run(scenario, text, **options), code)


def rolls_back(name, scenario, also=lambda result: True):
    def judge(text):
        result = run(scenario, text)
        return rolled_back(result) and also(result)
    return name, judge


def after_failed_steady(scenario, judge):
    """让「新 agent 不稳定」必然发生，只看换回那一段。"""
    return lambda text: judge(run(scenario, replaced(text, FIRST_STEADY, "false ||")))


# ---- 前置：把现场改坏一处


def hold_lock(bench):
    lock = open(bench.lock, "w")
    fcntl.flock(lock, fcntl.LOCK_EX)
    return lock.close


def backup_differs(bench):
    bench.backup.write_text("别的")


def backup_same(bench):
    bench.backup.write_bytes(bench.unit.read_bytes())


def backup_is_link(bench):
    bench.backup.symlink_to(bench.unit)


def backup_is_dangling(bench):
    bench.backup.symlink_to("没有")


def no_tests_log(bench):
    bench.tests_log.unlink()


def tests_failed(bench):
    bench.tests_log.write_text("RUN-AGENT-TESTS exit=1\n")


def no_replica_state(bench):
    bench.replica_state.unlink()


def stale_replica(bench):
    os.utime(bench.replica_state, (1, 1))


def no_release(bench):
    bench.release.rename(bench.home / "gone")


def sums_changed(bench):
    sums = bench.release / "SHA256SUMS"
    sums.write_text(sums.read_text() + "\n")


def package_changed(bench):
    (bench.release / "other").write_text("z")


def escaped_line(bench):
    """带反斜杠的文件名：169 的 sha256sum 只警告、退出码照样是 0。"""
    sums = bench.release / "SHA256SUMS"
    sums.write_text(sums.read_text() + "\\" + "0" * 64 + "  no\\\\where\n")
    bench.pins["SUMS_SHA"] = sha(sums)


def check_skips_a_line(bench):
    command(bench, "sha256sum", 'if [[ "$*" == *--check* ]]; then /usr/bin/sha256sum "$@" | head -n 1; exit 0; fi\nexec /usr/bin/sha256sum "$@"\n')


def pipe_in_release(bench):
    os.mkfifo(bench.release / "pipe")


def already_switched(bench):
    bench.unit.write_text(bench.unit.read_text().replace(OLD, NEW))


def third_mention(bench):
    bench.unit.write_text(bench.unit.read_text() + f"# {OLD}\n")
    expect_current_unit(bench)


def wrong_new_unit(bench):
    bench.pins["NEW_UNIT_SHA"] = hashlib.sha256(b"").hexdigest()


def older_human(bench):
    assert HUMAN_RELEASE.endswith("-r28")
    bench.human_unit.write_text(human_unit_text(HUMAN_RELEASE.replace("-r28", "-r27")))


def no_log_file(bench):
    command(bench, "mktemp", '[[ "$*" == *-install-* ]] && exit 1\nexec /usr/bin/mktemp "$@"\n')


# ---- 不止看一个结局的用例


def preview_changes_nothing(text):
    result = run("good", text, args=())
    return result.code == 0 and result.unit_untouched and not (result.sudo or result.backup_is_old_unit or result.logs or restarted(result))


def preview_checks_too(text):
    result = run("agent-down", text, args=())
    return result.code == 1 and not result.sudo


def unknown_argument(text):
    result = run("good", text, args=("--force",))
    return result.code == 2 and not result.sudo


def extra_argument(text):
    result = run("good", text, args=("--apply", "x"))
    return result.code == 2 and not result.sudo and not result.logs


def private_argument_on_terminal(text):
    code, shown = run_detached_on_terminal(text)
    return code == 1 and "只给脚本自己用" in shown


def log_cannot_be_made(text):
    result = run("good", text, setup=no_log_file)
    return result.code == 1 and result.unit_untouched and "日志建不出来" in result.output and not (result.logs or result.sudo)


def different_backup_is_kept(text):
    result = run("good", text, setup=backup_differs)
    return untouched(result, "不覆盖") and result.backup_untouched


def same_backup_is_reused(text):
    result = run("good", text, setup=backup_same)
    return installed(result) and result.backup_untouched


def last_print_fails(text):
    return installed(run("good", replaced(text, LAST_LINE, "false")))


def term_while_freezing(text):
    return stopped(run("term-while-freezing", text), "")


def unknown_unit_is_kept(text):
    result = run("garbled-unit", text)
    return rolled_back(result) and result.unexpected_kept


def put_back_fails(result):
    return stuck(result) and not any(call.startswith(("stop", "start")) for call in result.calls)


def slow_answers_go_by_the_clock(text):
    result = run("slow-answers", text)
    asked = result.calls.count("show studiolo-agent -p ActiveState --value")
    return rolled_back(result) and asked < ANSWER_SECONDS


def not_restarted(result):
    return not restarted(result)


CASES = [
    # 前置和参数：不动任何东西
    ("预演不改", preview_changes_nothing),
    ("预演也核", preview_checks_too),
    ("参数不认得", unknown_argument),
    ("参数多了", extra_argument),
    ("自用参数在终端上", private_argument_on_terminal),
    ("日志建不出来", log_cannot_be_made),
    refuses("另一个在跑", "正在跑", hold_lock),
    ("备份已在但不同", different_backup_is_kept),
    refuses("备份是链接", "不覆盖", backup_is_link),
    refuses("备份是断链", "不覆盖", backup_is_dangling),
    refuses("没有日志", "没有随包测试日志", no_tests_log),
    refuses("日志没过", "末行不是", tests_failed),
    refuses("副本状态不在", "状态文件不在", no_replica_state),
    refuses("副本太旧", "没更新", stale_replica),
    refuses("发布目录不在", "发布目录不在", no_release),
    refuses("整包清单变了", "整包清单的摘要", sums_changed),
    refuses("包被改", "整包验签没过", package_changed),
    refuses("清单有转义行", "整包验签没过", escaped_line),
    refuses("验签少数一行", "整包验签没过", check_skips_a_line),
    refuses("目录里有管道", "既不是文件也不是目录", pipe_in_release),
    refuses("单元已换过", "不是预期的旧值", already_switched),
    refuses("旧名三处", "不是恰好两处", third_mention),
    refuses("新单元摘要不对", "换发布名之后的单元摘要", wrong_new_unit),
    refuses("agent没在跑", "studiolo-agent 现在不是", scenario="agent-down"),
    refuses("人侧没在跑", "studiolo-human 现在不是", scenario="human-down"),
    refuses("人侧还是旧发行", "人侧还不是", older_human),
    refuses("起点重启过", "已经自己重启过", restarts=1),
    refuses("mcp不是401", "/mcp 上没有应答 401", scenario="mcp-200"),
    refuses("新路径不是404", "没有应答 404", scenario="connect-401"),
    # 真做：换好
    installs("正常"),
    ("备份已在且相同", same_backup_is_reused),
    installs("换好之后收到TERM", "term-when-done"),
    installs("换好之后打印失败", "print-fails"),
    ("换好之后末行写不出", last_print_fails),
    installs("环境里带着旧标记", env={"T237_DETACHED": "1"}),
    installs("收到INT照跑", "INT"),
    installs("收到QUIT照跑", "QUIT"),
    installs("终端断开照跑", "outer-HUP", -1),
    installs("终端CtrlC照跑", "outer-INT", -2),
    installs("外层被杀照跑", "outer-KILL", -9),
    # 真做：还没换单元就停下
    halts("认不出旧agent", "unknown-old-program", "在跑的不是旧发布的程序"),
    halts("冻结属主不对", "owner", "属主不是"),
    halts("冻结程序权限不对", "program-mode", "程序权限不是"),
    halts("冻结文件权限不对", "file-mode", "权限不是 440"),
    halts("冻结目录权限不对", "directory-mode", "权限不是 550 的目录"),
    halts("冻结时内容被换", "swapped-while-freezing", "冻结后整包验签"),
    ("冻结中TERM", term_while_freezing),
    # 真做：换了单元、没重启就失败，换回而且不白断旧 agent
    rolls_back("写单元报错", "put-new-fails", not_restarted),
    rolls_back("改名报错", "rename-fails", not_restarted),
    rolls_back("重载报错", "reload-fails", not_restarted),
    rolls_back("换单元中间TERM", "term-while-putting", not_restarted),
    # 真做：重启之后失败，换回并起回旧 agent
    rolls_back("重启命令报错", "restart-fails"),
    rolls_back("重启中TERM", "term-while-restarting", restart_finished_before_stop),
    rolls_back("重启中HUP", "hup-while-restarting", restart_finished_before_stop),
    rolls_back("第10秒掉线", "drops-at-10"),
    rolls_back("第14秒掉线", "drops-at-14"),
    rolls_back("崩溃循环但总是active", "crash-loop"),
    rolls_back("进程号不变但重启过", "restarted-same-pid"),
    rolls_back("只换了进程号", "new-pid"),
    rolls_back("状态不是active但进程没变", "deactivating"),
    rolls_back("撞启动上限", "start-limit"),
    rolls_back("一直不应答", "never-answers"),
    rolls_back("应答404", "answers-404"),
    rolls_back("在跑的不是新程序", "wrong-program"),
    rolls_back("收到TERM", "TERM"),
    rolls_back("收到HUP", "HUP"),
    ("应答慢也按钟算", slow_answers_go_by_the_clock),
    ("换上的内容不对", unknown_unit_is_kept),
    ("换回中再来信号", after_failed_steady("signals-while-putting-back", rolled_back)),
    # 真做：换回也没成
    ("旧的也起不来", after_failed_steady("old-never-answers", stuck)),
    ("旧单元写不回去", after_failed_steady("put-back-fails", put_back_fails)),
]
