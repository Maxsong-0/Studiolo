#!/usr/bin/env bash
# T-036 / source c9cc85b：仅迁移 agent 供给服务，进程始终以 maxsong 运行。
# 在服务器由用户显式 sudo bash 本文件；脚本本身不调用 sudo。
set -Eeuo pipefail
umask 077
export PATH=/usr/sbin:/usr/bin:/sbin:/bin
unset CDPATH ENV BASH_ENV PYTHONPATH PYTHONHOME

release=/home/maxsong/.local/share/studiolo-agent-releases/m3-t036-20260910-replica01
binary=$release/bin/studiolo-agent
binary_sha=6c574ea719cde129549354b44b015e144c1ffb9b581adf65aa51d5c8f9979a08
source_unit=/home/maxsong/studiolo-t036-system.service
unit_sha=217bdceda8b1e4cbc166973e0c14bc0fe7c7f8aee9493a1d11b3798bd086b565
unit=studiolo-agent.service
installed=/etc/systemd/system/$unit
data=/home/maxsong/.local/share/studiolo-agent
replica=/home/maxsong/.local/share/studiolo-agent-replica
token=/home/maxsong/.config/studiolo-agent/claude.token
scratch=
stage=
stage_binding=
installed_binding=
user_changed=0
committed=0

fail() { printf '%s\n' "$1" >&2; exit 1; }
digest() { sha256sum -- "$1" | cut -d ' ' -f 1; }
identity() { stat -c '%d:%i' -- "$1"; }
userctl() { runuser -u maxsong -- env XDG_RUNTIME_DIR=/run/user/1000 systemctl --user "$@"; }

installed_is_ours() {
    [[ -n "$installed_binding" && -f "$installed" && ! -L "$installed" ]] &&
        [[ $(identity "$installed") == "$installed_binding" && $(digest "$installed") == "$unit_sha" ]]
}

stop_stage() {
    [[ -n "$stage" ]] || return 0
    local fragment=/run/systemd/transient/$stage
    # 临时单元名含 root 私有 mktemp 的随机后缀；仅停止仍指向本轮文件的单元。
    if [[ -f "$fragment" && ! -L "$fragment" ]]; then
        if [[ -z "$stage_binding" ]]; then
            [[ $(systemctl show "$stage" -p User --value) == maxsong ]] || return 1
            [[ $(systemctl show "$stage" -p Group --value) == maxsong ]] || return 1
            [[ $(systemctl show "$stage" -p FragmentPath --value) == "$fragment" ]] || return 1
            stage_binding=$(identity "$fragment")
        fi
        [[ $(identity "$fragment") == "$stage_binding" ]] || return 1
        systemctl stop "$stage" || return 1
    fi
    stage=
    stage_binding=
}

cleanup() {
    local result=$? rollback_failed=0
    trap - EXIT INT TERM HUP
    set +e
    stop_stage || { printf '%s\n' '临时阶段单元绑定发生变化或停止失败，请人工检查。' >&2; rollback_failed=1; }
    if [[ "$committed" != 1 ]]; then
        if [[ -n "$installed_binding" ]]; then
            if installed_is_ours; then
                systemctl stop "$unit" || rollback_failed=1
                if installed_is_ours; then systemctl disable "$unit" || rollback_failed=1; fi
                if installed_is_ours; then
                    rm -- "$installed" || rollback_failed=1
                    systemctl daemon-reload || rollback_failed=1
                else
                    printf '%s\n' '系统单元已被其他操作替换，未删除替换后的文件。' >&2
                    rollback_failed=1
                fi
            else
                printf '%s\n' '系统单元已被其他操作替换，未停止、禁用或删除该单元。' >&2
                rollback_failed=1
            fi
        fi
        if [[ "$user_changed" == 1 ]]; then
            userctl enable "$unit" || rollback_failed=1
            userctl start "$unit" || rollback_failed=1
            userctl is-active --quiet "$unit" || rollback_failed=1
            userctl is-enabled --quiet "$unit" || rollback_failed=1
            if [[ "$rollback_failed" == 0 ]]; then
                printf '%s\n' '切换未完成，原用户服务已恢复 enabled + active。' >&2
            fi
        fi
    fi
    if [[ -n "$scratch" && -d "$scratch" && ! -L "$scratch" ]]; then
        rm -f -- "$scratch/studiolo-agent.service" || rollback_failed=1
        rmdir -- "$scratch" || rollback_failed=1
    fi
    if [[ "$rollback_failed" != 0 ]]; then
        printf '%s\n' '清理或恢复未完全成功，请检查系统服务和原用户服务；未覆盖第三方替换。' >&2
        result=1
    fi
    exit "$result"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
trap 'exit 129' HUP

wait_ready() {
    local port=$1 attempt code
    for attempt in {1..40}; do
        code=$(curl --silent --output /dev/null --write-out '%{http_code}' --max-time 2 \
            "http://127.0.0.1:$port/mcp") || code=000
        [[ "$code" == 401 ]] && return 0
        sleep 0.5
    done
    fail "回环端口 $port 未按期返回匿名 401。"
}

check_process() {
    local service=$1 pid
    systemctl is-active --quiet "$service"
    pid=$(systemctl show "$service" -p MainPID --value)
    [[ "$pid" =~ ^[0-9]+$ && "$pid" -gt 1 ]] || fail '服务未提供有效 MainPID。'
    [[ $(readlink -f "/proc/$pid/exe") == "$binary" ]] || fail 'MainPID 未运行指定发布二进制。'
    [[ $(digest "/proc/$pid/exe") == "$binary_sha" ]] || fail '运行中二进制摘要不符。'
    [[ $(awk '/^Uid:/ {print $2, $3, $4, $5}' "/proc/$pid/status") == '1000 1000 1000 1000' ]] ||
        fail '运行中服务 UID 不符合 maxsong。'
    # systemd 可以创建保持身份映射的新命名空间；比较 inode 会误拒绝这种安全配置。
    # 必须同时保留 root 和 maxsong 的 UID，拒绝仅映射当前用户或改写任一身份。
    awk '''{ if ($1 <= 0 && 0 < $1 + $3 && $2 - $1 == 0) root_ok=1;
             if ($1 <= 1000 && 1000 < $1 + $3 && $2 + 1000 - $1 == 1000) user_ok=1 }
         END { exit !(root_ok && user_ok) }''' "/proc/$pid/uid_map" || fail '服务 UID 映射未保留 root 与 maxsong 身份。'
    [[ $(systemctl show "$service" -p User --value) == maxsong &&
       $(systemctl show "$service" -p Group --value) == maxsong &&
       $(systemctl show "$service" -p PrivateUsers --value) == no ]] || fail '服务身份或用户命名空间不符。'
}

check_supply() {
    local port=$1
    # 不让 root 解释发布目录中可由用户改写的 Python 源码；token 只传路径。
    runuser -u maxsong -- /usr/bin/python3 -B "$release/source/scripts/check-agent.py" \
        "http://127.0.0.1:$port/mcp" "$token"
    runuser -u maxsong -- /usr/bin/python3 -B "$release/source/scripts/check-agent.py" \
        "http://127.0.0.1:$port/mcp" "$token" --no-token
    runuser -u maxsong -- /usr/bin/python3 -B "$release/source/scripts/check-agent.py" \
        "http://127.0.0.1:$port/mcp" "$token" --bad-token
    runuser -u maxsong -- /usr/bin/python3 -B "$release/source/scripts/check-agent-replica.py" \
        "http://127.0.0.1:$port/mcp" "$token" --expected ready
}

[[ "$EUID" == 0 && $# == 0 ]] || fail '请在目标服务器以 sudo bash 执行本脚本，不接受额外参数。'
[[ $(id -u maxsong) == 1000 && $(id -g maxsong) == 1000 ]] || fail 'maxsong 的 UID/GID 不符合服务器约定。'
[[ -f "$binary" && ! -L "$binary" && $(digest "$binary") == "$binary_sha" ]] || fail '发布二进制或 SHA256 不符。'
[[ -f "$replica/control/state.json" && -f "$token" ]] || fail '副本回执或既有 token 文件缺失。'
[[ -d /run/user/1000 ]] || fail '原用户 systemd 运行目录不存在。'
userctl is-active --quiet "$unit"
userctl is-enabled --quiet "$unit"

[[ $(systemctl show "$unit" -p LoadState --value 2>/dev/null || true) == not-found ]] ||
    fail '系统管理器已存在同名服务，拒绝覆盖。'
for base in /etc/systemd/system /run/systemd/system /usr/local/lib/systemd/system /usr/lib/systemd/system /lib/systemd/system; do
    for existing in "$base/$unit" "$base/$unit.d" "$base/multi-user.target.wants/$unit"; do
        [[ ! -e "$existing" && ! -L "$existing" ]] || fail '已有系统单元、覆盖配置或启用链接，拒绝覆盖。'
    done
done
[[ -d /etc/systemd/system && ! -L /etc/systemd/system && $(stat -c '%u' /etc/systemd/system) == 0 ]] ||
    fail '系统单元目录身份不符。'
[[ $(ss -H -ltn 'sport = :4401' | wc -l) == 0 ]] || fail '4401 已占用，拒绝启动阶段单元。'

[[ -f "$source_unit" && ! -L "$source_unit" && $(stat -c '%u:%h' "$source_unit") == 1000:1 ]] ||
    fail '源单元必须为 maxsong 所有的普通单链接文件。'
source_mode=$(stat -c '%a' "$source_unit")
(( (8#$source_mode & 0022) == 0 )) || fail '源单元不能允许组或其他用户写入。'
source_identity=$(stat -c '%d:%i:%f:%s:%u:%h:%Y:%Z' "$source_unit")
scratch=$(mktemp -d /etc/systemd/system/.studiolo-t036.XXXXXXXX)
# GNU cp 不解引用最终源链接；私有副本逐字节校验后才可能进入系统管理器。
cp --no-dereference -- "$source_unit" "$scratch/studiolo-agent.service"
[[ -f "$scratch/studiolo-agent.service" && ! -L "$scratch/studiolo-agent.service" && $(stat -c '%u:%h' "$scratch/studiolo-agent.service") == 0:1 &&
   $(stat -c '%d:%i:%f:%s:%u:%h:%Y:%Z' "$source_unit") == "$source_identity" &&
   $(digest "$scratch/studiolo-agent.service") == "$unit_sha" ]] || fail '源单元身份变化或私有副本摘要不符。'
chmod 0644 "$scratch/studiolo-agent.service"
systemd-analyze verify "$scratch/studiolo-agent.service"

stage_candidate=studiolo-t036-stage-${scratch##*.}.service
[[ $(systemctl show "$stage_candidate" -p LoadState --value 2>/dev/null || true) == not-found ]] ||
    fail '阶段单元名已存在，拒绝复用。'
stage=$stage_candidate
printf '%s\n' '开始系统管理器下的 4401 阶段验收。'
systemd-run --unit="$stage" --collect --service-type=simple \
    --description='Studiolo T-036 system stage (loopback 4401)' \
    --property=StartLimitIntervalSec=60 --property=StartLimitBurst=3 \
    --property=User=maxsong --property=Group=maxsong --property=PrivateUsers=no \
    --property="WorkingDirectory=$release" --property=Restart=on-failure --property=RestartSec=3 \
    --property=KillSignal=SIGTERM --property=TimeoutStopSec=20 --property=UMask=0077 \
    --property=NoNewPrivileges=yes --property=ProtectSystem=strict --property=ProtectHome=read-only \
    --property="ReadWritePaths=$data" --property=PrivateTmp=yes --property=RestrictSUIDSGID=no \
    --property=LockPersonality=yes --property='RestrictAddressFamilies=AF_UNIX AF_INET' \
    --property=SystemCallArchitectures=native --property=MemoryMax=512M --property=TasksMax=64 \
    -- "$binary" serve --replica-root "$replica" --replica-max-age-secs 300 \
    --allow 项目/Studiolo --allow _规则 --data "$data" --port 4401
stage_binding=$(identity "/run/systemd/transient/$stage")
wait_ready 4401
check_process "$stage"
check_supply 4401
check_process "$stage"
stop_stage

# 硬链接原子创建目标，不覆盖已存在文件；保留 inode 以限定之后回滚的对象。
installed_binding=$(identity "$scratch/studiolo-agent.service")
ln -- "$scratch/studiolo-agent.service" "$installed"
systemctl daemon-reload
[[ $(systemctl show "$unit" -p FragmentPath --value) == "$installed" &&
   -z $(systemctl show "$unit" -p DropInPaths --value) ]] || fail '新系统单元加载位置或覆盖配置不符。'
installed_is_ours || fail '新系统单元绑定变化。'
user_changed=1
userctl stop "$unit"
userctl disable "$unit"
systemctl enable "$unit"
systemctl start "$unit"
wait_ready 4400
check_process "$unit"
check_supply 4400
check_process "$unit"
installed_is_ours || fail '最终系统单元绑定变化。'
systemctl is-enabled --quiet "$unit"
if userctl is-active --quiet "$unit"; then fail '旧用户服务仍在运行。'; fi
if userctl is-enabled --quiet "$unit"; then fail '旧用户服务仍然启用。'; fi
committed=1
printf '%s\n' 'T-036 上线完成：系统服务 enabled + active，UID 1000，指定二进制已验签，五处副本新鲜度与四工具链通过。'
