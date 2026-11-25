#!/usr/bin/env bash
# T-237（连 T-241）：冻结 agent 发布目录 → 换 studiolo-agent 单元里的发布名（两处）→ 重启 agent。
# 对应 docs/m3/server-agent.md「安装到生产」第 3 步和「切换单元」。
#
# 用法：bash t237-install-agent.sh            只预演：做全部前置检查，不用 sudo、不改任何东西
#       bash t237-install-agent.sh --apply    真做：下面标着 sudo 的几行才会执行
# 任何一步不对就停。单元换上之后的任何失败（重载或重启报错、60 秒没开始应答、起来后掉线或重启过、
# 被 kill）都会自动换回旧单元，重启过就起回旧 agent，再核它稳定。
# 真做那一段脱离终端跑，输出写进日志再显示：关终端、断线、Ctrl-C 都打断不了它，重连之后看日志末行。
# 退出码（日志末行 T237-INSTALL exit=N 同一个数）：0 换好了，或预演全过；1 停下了（没换，或换过又换回、旧 agent 在跑）；
# 2 参数不对；3 换回也没成，agent 可能停着，要人处理。
set -euo pipefail
export LC_ALL=C

OLD=t215-116f5d7-20261120
NEW=t237-af83f0e-20261125
ROOT="$HOME/.local/share/studiolo-agent-releases"
D="$ROOT/$NEW"
U=/etc/systemd/system/studiolo-agent.service
B="$HOME/studiolo-agent.service.bak-$OLD-to-$NEW"
LOG="$HOME/$NEW-agent-tests.log"
BASE=http://127.0.0.1:4400
# 人侧要先是哪个发行、单元在哪；副本多久没更新算旧（agent 的线是 300 秒）。
HUMAN_RELEASE=m5m6-board-20260920-r28
HUMAN_UNIT=/etc/systemd/system/studiolo-human.service
STATE="$HOME/.local/share/studiolo-agent-replica/control/state.json"
REPLICA_SECONDS=240
RUN_AS=maxsong
LOCK="$HOME/.t237-install-agent.lock"
# 换之前、换之后单元文件的 sha256；前一个是人侧 r28 装配里钉的 AGENT_SHA（r28 要先激活），后一个留给下一发行重算。
OLD_UNIT_SHA=394ffe6f55823e7ac8cf7cad02fcb2c34cbee0d284afd3c706d13afe54f50117
NEW_UNIT_SHA=f55dd063461e77945eef22ea8b4f33be782d3487c50147c5d4551d5441ebb445
# 108 打包时记下的整包清单摘要。
SUMS_SHA=8c395c0c48c0664c6a0ba75389432c9559f1f08c017bac2e909f9a4c08b5df53
# 开始应答的上限（秒，按钟算）；应答之后要连续稳定的秒数：单元 RestartSec=3、60 秒内起 3 次就放弃，15 秒够看出崩溃循环。
ANSWER_SECONDS=60
STEADY_SECONDS=15

# --detached 只给脚本自己用：脱离终端之后的那一半。
DETACHED=0
case "${1:-}" in
  '') APPLY=0 ;;
  --apply) APPLY=1 ;;
  --detached) APPLY=1; DETACHED=1 ;;
  *) echo '用法：bash t237-install-agent.sh [--apply]' >&2; exit 2 ;;
esac
[[ $# -le 1 ]] || { echo '用法：bash t237-install-agent.sh [--apply]' >&2; exit 2; }

digest() { sha256sum "$1" | cut -c1-64; }
die() { echo "停下：$*" >&2; exit 1; }
step() { echo "== $*"; }

# 真做：另起一个脱离终端的自己，这边只跟着看日志。
if [[ "$APPLY" == 1 && "$DETACHED" == 0 ]]; then
  OUT="$(mktemp "$HOME/$NEW-install-$(date +%Y%m%dT%H%M%S)-XXXXXX.log")" || die '日志建不出来，什么都没做'
  echo "输出在 $OUT；关终端、Ctrl-C 都不影响它跑完。"
  setsid --wait bash "$0" --detached < /dev/null > "$OUT" 2>&1 &
  INNER=$!
  tail -n +1 -s 0.5 --pid="$INNER" -f "$OUT" || true
  wait "$INNER"
  exit
fi
[[ "$DETACHED" == 0 || ! -t 1 ]] || die '--detached 只给脚本自己用，真做请用 --apply'

# 整包验签：退出码之外再数 OK 的行数（169 的 sha256sum 碰到带反斜杠的文件名只警告、退出码照样是 0）。
verify() {
  local out
  out="$(cd "$D" && sha256sum --check SHA256SUMS 2>/dev/null)" || return 1
  [[ "$(grep -c ': OK$' <<< "$out")" == "$(wc -l < "$D/SHA256SUMS")" ]]
}

# 单元原子换上：先写旁边的临时名，再改名。任何时刻正式的名字都是完整的一份。
put_unit() {
  sudo install -m 0644 -o root -g root "$1" "$U.t237-writing" && sudo mv -T "$U.t237-writing" "$U"
}

show() { systemctl show studiolo-agent -p "$1" --value; }

# 没带令牌的应答码；和激活脚本探同一个回环口的写法一致：不读配置、不走代理、不跟跳转。
answer() {
  curl --disable --silent --output /dev/null --write-out '%{http_code}' --noproxy '*' --proxy '' \
    --proto '=http' --max-redirs 0 --max-time 2 "$BASE$1" || true
}

# 发布 $1 的 agent 在 $2 上拒绝没带令牌的请求（401），并且同一个进程连续稳定、一次没重启过。
steady() {
  local release=$1 path=$2 pid='' code='' i deadline
  deadline=$(( $(date +%s) + ANSWER_SECONDS ))
  while (( $(date +%s) < deadline )); do
    [[ "$(show ActiveState)" != failed ]] || { echo '服务进了 failed' >&2; return 1; }
    code="$(answer "$path")"
    if [[ "$code" == 401 ]]; then pid="$(show MainPID)"; break; fi
    sleep 1
  done
  [[ "$pid" =~ ^[1-9][0-9]*$ ]] || { echo "$ANSWER_SECONDS 秒内 $path 没有应答 401（最后一次 ${code:-无}）" >&2; return 1; }
  for (( i = 0; i < STEADY_SECONDS; i++ )); do
    sleep 1
    if [[ "$(show ActiveState)" != active || "$(show MainPID)" != "$pid" || "$(show NRestarts)" != 0 ]]; then
      echo "不稳定：$(show ActiveState)，进程 $(show MainPID)（原来 $pid），重启 $(show NRestarts) 次" >&2
      return 1
    fi
  done
  runs "$pid" "$release" || { echo '在跑的不是这个发布的程序' >&2; return 1; }
}

# 进程 $1 跑的是不是发布 $2 的程序。进程设成了不可检视，普通账号读不到它的 exe。
runs() { [[ "$(sudo readlink "/proc/$1/exe")" == "$ROOT/$2/bin/studiolo-agent" ]]; }

put_back() {
  echo '没成：换回旧单元。' >&2
  sudo rm -f "$U.t237-writing"
  # 现场既不是新单元也不是旧单元：先留一份再换回，不悄悄丢掉。
  case "$(digest "$U")" in
    "$OLD_UNIT_SHA" | "$NEW_UNIT_SHA") ;;
    *) sudo cp --no-clobber "$U" "$B.unexpected"; echo "单元内容对不上，已另存到 $B.unexpected" >&2 ;;
  esac
  if [[ "$(digest "$U")" != "$OLD_UNIT_SHA" ]]; then
    put_unit "$B" || { echo '旧单元写不回去。' >&2; stuck; return 1; }
  fi
  sudo systemctl daemon-reload
  # 还没重启过就是旧 agent 一直在跑，不白断它一次。
  if [[ "$RESTARTED" == 1 ]]; then
    sudo systemctl stop studiolo-agent
    # 崩溃循环会撞上启动次数上限：不清掉，旧的也起不来。
    sudo systemctl reset-failed studiolo-agent
    sudo systemctl start studiolo-agent
  fi
  if [[ "$(digest "$U")" == "$OLD_UNIT_SHA" ]] && steady "$OLD" /mcp; then
    echo '已换回：单元是旧的，旧 agent 在跑、稳定。查清之后可以再跑本脚本。' >&2
    return 0
  fi
  stuck
  return 1
}

stuck() {
  echo "换回没成：单元摘要 $(digest "$U")，状态 $(show ActiveState)。agent 可能停着，把上面的输出贴给会话，别再跑本脚本。" >&2
}

NEW_UNIT=''
CHANGED=0
RESTARTED=0
DONE=0
finish() {
  local code=$?
  # 收尾不许再被打断；先忽略信号，再摘掉 EXIT。
  trap '' TERM HUP PIPE
  trap - EXIT
  set +e
  [[ -z "$NEW_UNIT" ]] || rm -f "$NEW_UNIT"
  if [[ "$DONE" == 1 ]]; then
    code=0
  elif [[ "$CHANGED" == 1 ]]; then
    code=1
    put_back || code=3
  elif [[ "$code" != 0 ]]; then
    code=1
  fi
  [[ "$APPLY" == 0 ]] || echo "T237-INSTALL exit=$code"
  exit "$code"
}
trap finish EXIT
if [[ "$APPLY" == 1 ]]; then trap '' INT QUIT; fi
trap 'exit 143' TERM
trap 'exit 129' HUP

# 同一时间只许一个在真做。
if [[ "$APPLY" == 1 ]]; then exec 9> "$LOCK"; fi
[[ "$APPLY" == 0 ]] || flock -n 9 || die '另一个 t237-install-agent.sh 正在跑'

step '前置检查（不改东西）'
[[ "$(id -un)" == "$RUN_AS" ]] || die "要以 $RUN_AS 身份运行"
[[ -d "$D" && ! -L "$D" ]] || die "发布目录不在：$D"
[[ -f "$LOG" && ! -L "$LOG" ]] || die "没有随包测试日志：$LOG"
[[ "$(tail -n 1 "$LOG")" == 'RUN-AGENT-TESTS exit=0' ]] || die '随包测试日志的末行不是 RUN-AGENT-TESTS exit=0'
[[ "$(digest "$D/SHA256SUMS")" == "$SUMS_SHA" ]] || die '整包清单的摘要和打包时记的不一样'
verify || die '整包验签没过'
[[ -z "$(find "$D" ! -type f ! -type d -print -quit)" ]] || die '发布目录里有既不是文件也不是目录的东西'
[[ "$(digest "$U")" == "$OLD_UNIT_SHA" ]] || die '单元现在的摘要不是预期的旧值（已经换过，或被别处改过）'
[[ "$(grep -o "$OLD" "$U" | wc -l)" == 2 ]] || die '单元里旧发布名不是恰好两处'
# 上一次换过又换回会留下备份：内容正好是旧单元就接着用，别的一律不碰。
if [[ -e "$B" || -L "$B" ]]; then
  [[ -f "$B" && ! -L "$B" && "$(digest "$B")" == "$OLD_UNIT_SHA" ]] || die "单元备份已存在，内容不是旧单元，不覆盖：$B"
fi
systemctl is-active --quiet studiolo-agent || die 'studiolo-agent 现在不是 active'
# 稳不稳看重启次数有没有离开 0：起点就不是 0 的话分不清。
[[ "$(show NRestarts)" == 0 ]] || die "studiolo-agent 已经自己重启过 $(show NRestarts) 次，先查清"
# 探测本身能用：旧 agent 在 /mcp 上拒绝没带令牌的请求，在新路径上回 404。
[[ "$(answer /mcp)" == 401 ]] || die '旧 agent 在 /mcp 上没有应答 401'
[[ "$(answer /connect/v1/board)" == 404 ]] || die '旧 agent 在 /connect/v1/board 上没有应答 404'
systemctl is-active --quiet studiolo-human || die 'studiolo-human 现在不是 active'
grep -q "/studiolo-human-releases/$HUMAN_RELEASE/launch-human.py" "$HUMAN_UNIT" || die "人侧还不是 $HUMAN_RELEASE：先激活它再换 agent"
[[ -f "$STATE" ]] || die "AI 副本的状态文件不在：$STATE"
AGE=$(( $(date +%s) - $(stat -c %Y "$STATE") ))
[[ "$AGE" -le "$REPLICA_SECONDS" ]] || die "AI 副本 $AGE 秒没更新（线是 300 秒），等它推完再换"
NEW_UNIT="$(mktemp)"
sed "s#$OLD#$NEW#g" "$U" > "$NEW_UNIT"
[[ "$(digest "$NEW_UNIT")" == "$NEW_UNIT_SHA" ]] || die '换发布名之后的单元摘要不是预期值'
echo '前置检查全过。单元会有这两行变化：'
diff "$U" "$NEW_UNIT" || true

if [[ "$APPLY" == 0 ]]; then
  echo '预演结束：什么都没改。真做请加 --apply。'
  exit 0
fi

step '1/4 冻结发布目录（root:maxsong，目录 550，文件 440，只有程序 550）'
sudo chown -R root:maxsong "$D"
sudo find "$D" -type d -exec chmod 0550 {} +
sudo find "$D" -type f -exec chmod 0440 {} +
sudo chmod 0550 "$D/bin/studiolo-agent"
[[ -z "$(find "$D" \( ! -user root -o ! -group maxsong \) -print -quit)" ]] || die '冻结后还有属主不是 root:maxsong 的'
[[ -z "$(find "$D" -type d ! -perm 0550 -print -quit)" ]] || die '冻结后还有权限不是 550 的目录'
[[ -z "$(find "$D" -type f ! -perm 0440 ! -path "$D/bin/studiolo-agent" -print -quit)" ]] || die '冻结后还有权限不是 440 的文件'
[[ "$(stat -c %a "$D/bin/studiolo-agent")" == 550 ]] || die '程序权限不是 550'
verify || die '冻结后整包验签没过'
echo '冻结完成，核对无例外。'

step '2/4 备份旧单元并换上新单元'
# 换回之后也靠这条认旧 agent：现在就认不出来的话，什么都还没换，停在这里。
runs "$(show MainPID)" "$OLD" || die '在跑的不是旧发布的程序'
[[ -e "$B" ]] || sudo install -m 0644 -o root -g root "$U" "$B"
[[ "$(digest "$B")" == "$OLD_UNIT_SHA" ]] || die '单元备份的摘要不对，没换单元'
# 从这里起，脚本不管怎么停都会换回旧单元（见 finish）。
CHANGED=1
put_unit "$NEW_UNIT"
[[ "$(digest "$U")" == "$NEW_UNIT_SHA" ]] || die '换上之后单元摘要不对'
echo "单元已换，备份在 $B"

step '3/4 重载并重启 studiolo-agent'
sudo systemctl daemon-reload
RESTARTED=1
sudo systemctl restart studiolo-agent

step "4/4 等新 agent 应答（最多 $ANSWER_SECONDS 秒），再看 $STEADY_SECONDS 秒稳不稳"
# 旧 agent 在这条路径上回 404：应答 401 就证明是新程序在答。
steady "$NEW" /connect/v1/board || die '新 agent 没起来或不稳定'
# 已经换好：后面只是打印，失败或被打断都不改结论（见 finish）。
DONE=1
echo "单元摘要 $(digest "$U")"
systemctl show studiolo-agent -p ActiveState -p NRestarts -p LimitMEMLOCK -p ActiveEnterTimestamp -p MainPID
echo 'T-237 agent 已换上并在运行。'
