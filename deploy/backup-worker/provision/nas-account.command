#!/bin/bash
# 用户在自己的 Mac 终端运行；不要通过 agent 工具执行本文件。
# 给备份 worker 在 NAS 的 rest-server 上加一个只追加账号（README「安装前要有的东西」第 5 项的 NAS 一侧）。
#   nas-account.command check   只读：NAS 上备份服务、账号文件、证书的现状，不写任何东西
#   nas-account.command add     设新账号的密码 → 加进 NAS 的账号文件 → 重启备份服务 → 再看一遍现状
# 怎么连 NAS：默认经 108 跳过去；人在家里局域网上可以先 export STUDIOLO_NAS_SSH="maxsong@192.168.100.111" 直连。
# 密码只在本机由 htpasswd 不回显地问两遍，传到 NAS 的只有 bcrypt 散列那一行；清理权限照旧只在 NAS 管理员手里。
set -euo pipefail
umask 077
export PATH=/usr/bin:/bin:/usr/sbin
unset BASH_ENV ENV CDPATH
here=$(cd "$(dirname "$0")" && pwd)
helper="$here/nas_account.py"
account=studiolo-backup-worker
read -r -a nas <<< "${STUDIOLO_NAS_SSH:--J studiolo-108-ts maxsong@192.168.100.111}"
[[ -f "$helper" ]] || { echo "找不到 $helper"; exit 1; }

case "${1:-}" in
  check)
    # 帮手脚本走标准输入，NAS 上不落文件。
    ssh -o BatchMode=yes "${nas[@]}" 'sudo -n python3 -I -B - check' < "$helper"
    ;;
  add)
    [[ -t 0 && -t 1 && -t 2 ]] || { echo '需要用户自己的终端'; exit 1; }
    scratch=$(mktemp -d "${TMPDIR:-/tmp}/studiolo-backup-account.XXXXXXXX")
    staged=studiolo-backup-nas_account.py
    cleanup() {
      rm -f "$scratch/line"
      rmdir "$scratch" 2>/dev/null || true
      ssh -o BatchMode=yes "${nas[@]}" "rm -f $staged" 2>/dev/null || true
    }
    trap cleanup EXIT
    echo "给 NAS 上的新账号 $account 定一个密码（只能追加备份，不能删改），先存进密码管理器。"
    /usr/sbin/htpasswd -c -B -C 12 "$scratch/line" "$account"
    # 追加那一步要把标准输入留给账号行，所以帮手脚本先放到 NAS 上该账号的家目录，结束时删掉。
    ssh -o BatchMode=yes "${nas[@]}" "umask 077; cat > $staged" < "$helper"
    ssh -o BatchMode=yes "${nas[@]}" "sudo -n python3 -I -B $staged append" < "$scratch/line"
    ssh -o BatchMode=yes "${nas[@]}" "sudo -n python3 -I -B $staged restart"
    ssh -o BatchMode=yes "${nas[@]}" "sudo -n python3 -I -B $staged check"
    echo 'NAS 账号加好了。只回复「NAS 账号加好了」，不要发密码或账号文件的内容。'
    ;;
  *)
    echo '用法：nas-account.command check | add'
    exit 2
    ;;
esac
