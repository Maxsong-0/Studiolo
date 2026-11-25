"""改错核对：把脚本改坏一处，指定的那条用例必须红。用例名是 None 的是等价改错，写明为什么。

每处改错的原文必须在脚本里恰好出现一次：脚本改了而这里没跟上，test_install_agent.py 就红。
"""

from .bench import TEXT

# 整行换成 true 的检查：行里的一段话 → 拦它的用例。
DROPPED_CHECKS = {
    "发布目录不在：": "发布目录不在",
    "没有随包测试日志": "没有日志",
    "末行不是": "日志没过",
    "整包清单的摘要": "整包清单变了",
    "|| die '整包验签没过'": "包被改",
    "既不是文件": "目录里有管道",
    "不是预期的旧值": "单元已换过",
    "不是恰好两处": "旧名三处",
    "studiolo-agent 现在不是": "agent没在跑",
    "studiolo-human 现在不是": "人侧没在跑",
    "先激活它": "人侧还是旧发行",
    "自己重启过": "起点重启过",
    "旧 agent 在 /mcp 上没有": "mcp不是401",
    "/connect/v1/board 上没有": "新路径不是404",
    "状态文件不在": "副本状态不在",
    "秒没更新": "副本太旧",
    "换发布名之后": "新单元摘要不对",
    "冻结后还有属主": "冻结属主不对",
    "550 的目录": "冻结目录权限不对",
    "440 的文件": "冻结文件权限不对",
    "程序权限不是": "冻结程序权限不对",
    "冻结后整包验签没过": "冻结时内容被换",
    "在跑的不是旧发布的程序": "认不出旧agent",
    "换上之后单元摘要不对": "换上的内容不对",
}

# (名字, 原文, 改成, 拦它的用例)
EDITS = [
    ("不武装换回", "CHANGED=1\nput_unit", "put_unit", "重启命令报错"),
    ("不记换好了", "DONE=1\necho", "echo", "正常"),
    ("换好后退出码不钉", '"$DONE" == 1 ]]; then\n    code=0\n', '"$DONE" == 1 ]]; then\n    true\n', "换好之后末行写不出"),
    ("不记重启过", "RESTARTED=1\nsudo", "sudo", "第10秒掉线"),
    ("没重启也断", 'if [[ "$RESTARTED" == 1 ]]', "if [[ 1 == 1 ]]", "写单元报错"),
    ("换回不停", "    sudo systemctl stop studiolo-agent\n", "", "只换了进程号"),
    ("不清启动上限", "    sudo systemctl reset-failed studiolo-agent\n", "", "撞启动上限"),
    ("换回不起", "    sudo systemctl start studiolo-agent\n", "", "第10秒掉线"),
    ("换回不重载", "  sudo systemctl daemon-reload\n  # 还没", "  # 还没", "第10秒掉线"),
    ("换上不重载", "sudo systemctl daemon-reload\nRESTARTED", "RESTARTED", "正常"),
    ("换回不写旧单元", '    put_unit "$B" ||', "    true ||", "第10秒掉线"),
    ("写不回去接着做", "; stuck; return 1; }\n  fi", "; }\n  fi", "旧单元写不回去"),
    ("不清临时单元", '  sudo rm -f "$U.t237-writing"\n', "", "改名报错"),
    ("不看重启次数", ' || "$(show NRestarts)" != 0 ]]', " ]]", "进程号不变但重启过"),
    ("不看进程号", ' || "$(show MainPID)" != "$pid"', "", "只换了进程号"),
    ("不看active", '"$(show ActiveState)" != active || ', "", "状态不是active但进程没变"),
    ("稳定窗口0秒", "STEADY_SECONDS=15", "STEADY_SECONDS=0", "第10秒掉线"),
    ("稳定窗口12秒", "STEADY_SECONDS=15", "STEADY_SECONDS=12", "第14秒掉线"),
    ("数圈不按钟", "while (( $(date +%s) < deadline )); do", "for (( i = 0; i < ANSWER_SECONDS; i++ )); do", "应答慢也按钟算"),
    ("应答不看401", '[[ "$code" == 401 ]]; then', '[[ "$code" != 000 ]]; then', "应答404"),
    ("不核程序", '  runs "$pid" "$release" ||', "  true ||", "在跑的不是新程序"),
    ("curl读配置", "--disable --silent", "--silent", "正常"),
    ("curl走环境代理", " --noproxy '*'", "", "正常"),
    ("curl走配置代理", " --proxy ''", "", "正常"),
    ("curl不限协议", "--proto '=http' ", "", "正常"),
    ("curl跟跳转", "--max-redirs 0 ", "", "正常"),
    ("curl无时限", "--max-time 2 ", "", "正常"),
    ("没有EXIT陷阱", "trap finish EXIT\n", "", "重启命令报错"),
    ("没有TERM陷阱", "trap 'exit 143' TERM\n", "", "重启中TERM"),
    ("没有HUP陷阱", "trap 'exit 129' HUP\n", "", "重启中HUP"),
    ("收尾不忽略信号", "  trap '' TERM HUP PIPE\n  trap - EXIT", "  trap - EXIT", "换回中再来信号"),
    ("退出码不归一", "    code=1\n    put_back", "    put_back", "收到TERM"),
    ("停下不归一", '  elif [[ "$code" != 0 ]]; then\n    code=1\n', "", "冻结中TERM"),
    ("换回失败当成功", "put_back || code=3", "put_back || true", "旧的也起不来"),
    ("换回没核稳定", ' && steady "$OLD" /mcp; then', "; then", "旧的也起不来"),
    ("备份不比内容",
     ' && "$(digest "$B")" == "$OLD_UNIT_SHA" ]] || die "单元备份已存在', ' ]] || die "单元备份已存在', "备份已在但不同"),
    ("备份不看断链", '[[ -e "$B" || -L "$B" ]]; then', '[[ -e "$B" ]]; then', "备份是断链"),
    ("备份不看链接", '[[ -f "$B" && ! -L "$B" && ', '[[ -f "$B" && ', "备份是链接"),
    ("验签不数行", "  [[ \"$(grep -c", "  [[ 1 == 1 || \"$(grep -c", "验签少数一行"),
    ("预演也做", 'if [[ "$APPLY" == 0 ]]; then\n  echo', 'if [[ "$APPLY" == 9 ]]; then\n  echo', "预演不改"),
    ("不另存意外内容", 'sudo cp --no-clobber "$U" "$B.unexpected";', "true;", "换上的内容不对"),
    ("不脱离终端", "setsid --wait bash", "bash", "终端断开照跑"),
    ("外层不取退出码", '  wait "$INNER"\n', "  true\n", "重启命令报错"),
    ("外层不显示日志", '  tail -n +1 -s 0.5 --pid="$INNER" -f "$OUT" || true\n', "", "正常"),
    ("跟日志不跟到结束", ' --pid="$INNER" -f "$OUT"', ' "$OUT"', "正常"),
    ("不上锁", "flock -n 9 || die", "true || die", "另一个在跑"),
    ("多参数放过", "[[ $# -le 1 ]] ||", "true ||", "参数多了"),
    ("自用参数放过", '[[ "$DETACHED" == 0 || ! -t 1 ]] || die', "true || die", "自用参数在终端上"),
    ("建日志失败不管", " || die '日志建不出来，什么都没做'", " || OUT=/dev/null", "日志建不出来"),
    ("末行不写退出码", '[[ "$APPLY" == 0 ]] || echo "T237-INSTALL', 'true || echo "T237-INSTALL', "正常"),
    # 等价：里层从后台起，INT、QUIT 一开始就是忽略的；那一行只防有人绕过外层直接跑。
    ("收到INT也停", "if [[ \"$APPLY\" == 1 ]]; then trap '' INT QUIT; fi\n", "", None),
    # 等价：清单的摘要钉死，任何一行没过，OK 的行数就对不上。
    ("验签不看退出码", '2>/dev/null)" || return 1\n  [[', '2>/dev/null)" || true\n  [[', None),
]


def mutants():
    """名字 → (改坏的脚本, 拦它的用例)。"""
    found = {}
    lines = TEXT.splitlines(keepends=True)
    for part, case in DROPPED_CHECKS.items():
        hits = [i for i, line in enumerate(lines) if part in line and "|| die" in line]
        assert len(hits) == 1, (part, hits)
        found["去掉：" + part] = ("".join(lines[: hits[0]] + ["true\n"] + lines[hits[0] + 1 :]), case)
    for name, old, new, case in EDITS:
        assert TEXT.count(old) == 1, (name, TEXT.count(old))
        assert name not in found, name
        found[name] = (TEXT.replace(old, new), case)
    return found
