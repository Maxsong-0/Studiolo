#!/usr/bin/env bash
# 把主人 Mac 上真实 vault 的白名单子树推送到服务器 studiolo-agent 的只读副本目录。
# 用法：bash scripts/sync-agent-vault.sh [--dry-run]
# 只同步 studiolo-agent.service.in 里 --allow 列出的目录；跳过隐藏项；副本目录整棵 0700/0600，
# 不同步 vault 其他内容，不碰服务器上其他目录。副本会陈旧：每次档案变动后重跑一次，或由主人排期。
set -euo pipefail
# 默认库根含撇号；bash 3.2 对 ${var:-...} 内的单引号解析有缺陷，先放进独立变量再取默认值。
STUDIOLO_VAULT_DEFAULT="/Users/maxsong/Documents/Max's Secret Place"
STUDIOLO_VAULT="${STUDIOLO_VAULT:-$STUDIOLO_VAULT_DEFAULT}"
STUDIOLO_HOST="${STUDIOLO_HOST:-maxsong@192.168.100.169}"
STUDIOLO_REMOTE_VAULT='/home/maxsong/.local/share/studiolo-agent-vault'
STUDIOLO_ALLOW=('项目/Studiolo' '_规则')
STUDIOLO_DRY=()
if [[ $# -gt 1 ]]; then
  printf '%s\n' '只接受可选参数 --dry-run。' >&2
  exit 2
fi
case "${1-}" in
  '') ;;
  --dry-run) STUDIOLO_DRY=(--dry-run --verbose) ;;
  *) printf '%s\n' '只接受可选参数 --dry-run。' >&2; exit 2 ;;
esac
if [[ ! -d "$STUDIOLO_VAULT" || -L "$STUDIOLO_VAULT" ]]; then
  printf '%s\n' "vault 根不存在或是符号链接：$STUDIOLO_VAULT" >&2
  exit 1
fi
for STUDIOLO_DIR in "${STUDIOLO_ALLOW[@]}"; do
  STUDIOLO_SRC="$STUDIOLO_VAULT/$STUDIOLO_DIR"
  if [[ ! -d "$STUDIOLO_SRC" || -L "$STUDIOLO_SRC" ]]; then
    printf '%s\n' "白名单目录不存在或是符号链接：$STUDIOLO_SRC" >&2
    exit 1
  fi
  # 只读 Broker 拒绝符号链接与硬链接；副本里出现它们会让服务启动失败，先在源头拦住。
  if [[ -n "$(find "$STUDIOLO_SRC" -name '.*' -prune -o \( -type l -o \( -type f -links +1 \) \) -print -quit)" ]]; then
    printf '%s\n' "白名单目录含符号链接或硬链接，拒绝同步：$STUDIOLO_SRC" >&2
    exit 1
  fi
  # 核心层只接受 NFC 路径；Mac 上若出现 NFD 文件名，服务器副本会被拒绝，同样在源头拦住。
  if ! find "$STUDIOLO_SRC" -name '.*' -prune -o -print0 | python3 -c '
import sys, unicodedata
bad = [p for p in sys.stdin.buffer.read().split(b"\0") if p and unicodedata.normalize("NFC", p.decode("utf-8")) != p.decode("utf-8")]
for p in bad: print("非 NFC 路径：" + p.decode("utf-8"), file=sys.stderr)
sys.exit(1 if bad else 0)'; then
    exit 1
  fi
done
# 副本根与每个白名单目录都由本脚本管理；rsync 目标目录内多余文件按源删除，隐藏项不进副本。
if [[ -z "${STUDIOLO_DRY[*]+x}" ]]; then
  ssh -o BatchMode=yes -o StrictHostKeyChecking=yes "$STUDIOLO_HOST" \
    "umask 077 && mkdir -p '$STUDIOLO_REMOTE_VAULT' && chmod 700 '$STUDIOLO_REMOTE_VAULT'"
fi
for STUDIOLO_DIR in "${STUDIOLO_ALLOW[@]}"; do
  if [[ -z "${STUDIOLO_DRY[*]+x}" ]]; then
    ssh -o BatchMode=yes -o StrictHostKeyChecking=yes "$STUDIOLO_HOST" \
      "umask 077 && mkdir -p '$STUDIOLO_REMOTE_VAULT/$STUDIOLO_DIR'"
  fi
  # bash 3.2 在 set -u 下展开空数组会报未绑定变量，用 ${arr[@]+"${arr[@]}"} 惯用法。
  rsync -rlt --delete --exclude='.*' --exclude='*/.*' ${STUDIOLO_DRY[@]+"${STUDIOLO_DRY[@]}"} \
    -e 'ssh -o BatchMode=yes -o StrictHostKeyChecking=yes' \
    "$STUDIOLO_VAULT/$STUDIOLO_DIR/" "$STUDIOLO_HOST:$STUDIOLO_REMOTE_VAULT/$STUDIOLO_DIR/"
done
if [[ -z "${STUDIOLO_DRY[*]+x}" ]]; then
  # 服务以同一用户读副本；权限收紧到仅属主可读，目录可进入。
  ssh -o BatchMode=yes -o StrictHostKeyChecking=yes "$STUDIOLO_HOST" \
    "chmod -R u=rwX,go= '$STUDIOLO_REMOTE_VAULT' && find '$STUDIOLO_REMOTE_VAULT' -type f | wc -l"
  printf '已同步白名单子树到 %s:%s（上一行为副本内文件数）。\n' "$STUDIOLO_HOST" "$STUDIOLO_REMOTE_VAULT"
else
  printf '试运行结束，未改动服务器。\n'
fi
