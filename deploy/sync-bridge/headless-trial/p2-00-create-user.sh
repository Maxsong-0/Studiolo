#!/usr/bin/env bash
# T-132 P2：建专用运行身份 studiolo-sync。需要 sudo，由用户本人执行：
#   sudo bash ~/studiolo-p2/p2-00-create-user.sh
set -euo pipefail
if id studiolo-sync >/dev/null 2>&1; then
  echo "studiolo-sync 已存在，停止，不改动现有账号" >&2
  exit 1
fi
if [ -e /var/lib/studiolo-sync ]; then
  echo "/var/lib/studiolo-sync 已存在，停止" >&2
  exit 1
fi
useradd --system --create-home --home-dir /var/lib/studiolo-sync \
  --shell /bin/bash --user-group --comment "Studiolo Obsidian headless (T-132)" studiolo-sync
chmod 0700 /var/lib/studiolo-sync
passwd --lock studiolo-sync >/dev/null
echo "== 结果 =="
id studiolo-sync
stat -c '%U:%G %a %n' /var/lib/studiolo-sync
passwd --status studiolo-sync
