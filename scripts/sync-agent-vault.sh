#!/usr/bin/env bash
# bash 直接发起 SSH；Python 只做捕获与回执核验，整批 fd 锁在入口退出前保持。
set -euo pipefail
umask 077
export PYTHONDONTWRITEBYTECODE=1
STUDIOLO_SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PYTHON="${STUDIOLO_PYTHON:-python3}"
CLIENT="$STUDIOLO_SCRIPT_DIR/replica_client.py"
case "$#:${1:-}" in
  0:) ;;
  1:--dry-run|1:--help|1:-h) exec "$PYTHON" "$CLIENT" "$@" ;;
  *) printf 'usage: sync-agent-vault.sh [--dry-run]\n' >&2; exit 2 ;;
esac

TEMP_BASE=/tmp
if [ "$(uname -s)" = Darwin ]; then TEMP_BASE=/private/tmp; fi
WORK="$(mktemp -d "$TEMP_BASE/studiolo-sync.XXXXXXXX")"
CHILD=
WATCHDOG=
cleanup() {
  trap - EXIT ALRM HUP INT TERM
  if [ -n "$CHILD" ]; then kill "$CHILD" 2>/dev/null || :; wait "$CHILD" 2>/dev/null || :; fi
  if [ -n "$WATCHDOG" ]; then kill "$WATCHDOG" 2>/dev/null || :; wait "$WATCHDOG" 2>/dev/null || :; fi
  rm -rf -- "$WORK"
}
failed_signal() {
  printf '{"status":"failed","last_error":"transfer_interrupted"}\n' >&2
  exit 1
}
trap cleanup EXIT
trap failed_signal HUP INT TERM
trap 'printf '\''{"status":"failed","last_error":"transfer_timeout"}\n'\'' >&2; exit 1' ALRM
# 截止覆盖捕获、SSH、回执三个阶段。每个子命令用 wait，确保 trap 可即时打断。
(
  TIMER=
  trap 'if [ -n "$TIMER" ]; then kill "$TIMER" 2>/dev/null || :; wait "$TIMER" 2>/dev/null || :; fi; exit 0' TERM
  sleep 120 & TIMER=$!
  wait "$TIMER"
  TIMER=
  kill -ALRM "$$"
) & WATCHDOG=$!
run_child() {
  "$@" & CHILD=$!
  local result=0
  wait "$CHILD" || result=$?
  CHILD=
  return "$result"
}
run_child "$PYTHON" "$CLIENT" --lock-path > "$WORK/lock-path"
IFS= read -r LOCK_PATH < "$WORK/lock-path"
# 不带 O_CREAT、不截断；Python 在持锁前核对安全路径与 fd 身份。
exec 9< "$LOCK_PATH"
run_child "$PYTHON" "$CLIENT" --prepare "$WORK"
HOST="$(< "$WORK/host")"
COMMAND="$(< "$WORK/command")"
# 输出限额防止恶意/失控远端填满磁盘；stderr 不反射正文。最终只报告数字退出码。
(
  ulimit -f 16
  exec 9<&-
  exec ssh -o BatchMode=yes -o StrictHostKeyChecking=yes -o ConnectTimeout=15 \
    -- "$HOST" "$COMMAND"
) < "$WORK/archive.tar" > "$WORK/response.json" 2>/dev/null & CHILD=$!
SSH_STATUS=0
wait "$CHILD" 2>/dev/null || SSH_STATUS=$?
CHILD=
run_child "$PYTHON" "$CLIENT" --finish "$WORK" --ssh-status "$SSH_STATUS"
