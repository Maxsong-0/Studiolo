#!/bin/bash
# 用户在自己的 Mac 终端运行；不要通过 agent 工具执行本文件。
# NAS 上备份服务（rest-server）的两件事：给备份 worker 加一个只追加账号（README「安装前要有的东西」第 5 项的 NAS 一侧）；
# 换上开机后会接着试的服务单元（原来那版在开机时存储卷还没挂上就被跳过，之后一直不起来）。
#   nas-account.command check     只读：NAS 上备份服务、账号文件、证书、这次开机和已装单元的现状，不写任何东西
#   nas-account.command add       设新账号的密码 → 加进 NAS 的账号文件 → 重启备份服务 → 再看一遍现状
#   nas-account.command restart   重启备份服务、等它开始监听 → 再看一遍现状（add 把账号加上了、服务却没起来时用这个，不要再跑 add）
#   nas-account.command unit      换服务单元：先在用完即删的临时单元上演一遍「没准备好就隔一会儿再试」→ 换上 → 重启备份服务
#                                 → 再看一遍现状；服务没起来就换回旧的。不问任何密码（登录 NAS 那一下除外）
# check 只读；add、restart、unit 一跑就动手，没有预演开关。
# 怎么连 NAS：默认经 108 跳过去；人在家里局域网上可以先 export STUDIOLO_NAS_SSH="maxsong@192.168.100.111" 直连。
# NAS 的主机钥匙钉在同目录的 nas_known_hosts 里：只认那一把，不认识或对不上就连不上，不会在终端里问「要不要信任」。
# 登录 NAS 由 ssh 自己办：这台 Mac 的钥匙在 NAS 上就直接过；不在，ssh 会在终端里问 NAS 的登录密码（本脚本不经手）。
# 一次运行只登录一回，后面几步共用这一条连接。NAS 上要 root 的几步用 sudo -n：sudo 要密码的话脚本做不了，会在改任何东西之前停下。
# 密码在本机不回显地问两遍，先核过规则再经管道交给 htpasswd（不进任何命令行）；htpasswd 把 bcrypt 散列那一行打到标准输出，
# 本机不落任何文件；传到 NAS 的只有那一行。帮手脚本和那一行都走 ssh 的标准输入，NAS 上也不落文件；
# ssh 一律 -T，不要远端终端（有终端就会把输入回显出来）。清理权限照旧只在 NAS 管理员手里。
set -euo pipefail
umask 077
export PATH=/usr/bin:/bin:/usr/sbin
unset BASH_ENV ENV CDPATH
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
helper="$here/nas_account.py"
unit_file="$here/../../sync-bridge/recovery/studiolo-rest-backup.service"
known_hosts="$here/nas_known_hosts"
account=studiolo-backup-worker
htpasswd=/usr/sbin/htpasswd
scratch=''
link=''
read -r -a nas <<< "${STUDIOLO_NAS_SSH:--J studiolo-108-ts maxsong@192.168.100.111}"
# 每一次 ssh 都带这几项：NAS 的主机钥匙只认仓库里钉住的那一把（不看这台 Mac 自己记的，也不往任何地方记新的）。
# 最后两项关掉另外两个钥匙来源：这台 Mac 的 ssh 配置里要是写了 KnownHostsCommand（由一条命令给出钥匙）或
# VerifyHostKeyDNS（信 DNS 里登记的指纹），不关的话它们给的钥匙也会被认。
# 默认经 108 跳：跳板那一段照旧用这台 Mac 自己的 ssh 配置和已知主机（命令行上的 -o 管不到跳板）。
pin=(-o StrictHostKeyChecking=yes -o "UserKnownHostsFile=\"$known_hosts\"" -o GlobalKnownHostsFile=/dev/null
     -o HostKeyAlias=studiolo-nas -o UpdateHostKeys=no -o KnownHostsCommand=none -o VerifyHostKeyDNS=no)
[[ -f "$helper" ]] || { echo "找不到 $helper"; exit 1; }
[[ -f "$known_hosts" ]] || { echo "找不到 $known_hosts"; exit 1; }
# 追加账号、换单元这两步，标准输入里先是帮手脚本、后面跟着要交给它的内容（账号行，或者单元文件）：
# 按字节数读出脚本来执行，剩下的留给脚本自己读。
boot='import sys; exec(compile(sys.stdin.buffer.read(int(sys.argv.pop())), "nas_account.py", "exec"))'

# worker 读凭据时拒绝首尾空白和控制字符（crates/backup-worker/src/credentials/file.rs）。这里收得更紧，
# 免得 NAS 上已经加了账号、169 那边却不认这个密码：16 到 72 个可见 ASCII 字符，不含空格。按字节判断，不看语言环境。
# 上限是 72：bcrypt 只用密码的前 72 个字节，再长的部分不算数（htpasswd 自己到 255 个字符才报错）。
valid_password() {
  local LC_ALL=C
  [[ ${#1} -ge 16 && ${#1} -le 72 && $1 =~ ^[!-~]+$ ]]
}

# htpasswd 给的得正好是「账号名:bcrypt 散列（cost 12）」这一行，和 NAS 上帮手脚本认的形状一致（nas_account.py 的 LINE）。
account_line() {
  local LC_ALL=C shape='^studiolo-backup-worker:[$]2[aby][$]12[$][./A-Za-z0-9]{53}$'
  [[ $1 =~ $shape ]]
}

# 登录一回，后面共用。控制口的路径有长度上限（macOS 104 字节，ssh 建的时候还要加 17 个字符），所以放在 /tmp 下的短目录里。
connect() {
  trap cleanup EXIT
  # 被 Ctrl-C、终端断开、要求结束时也走一遍收尾。
  trap 'exit 130' INT
  trap 'exit 129' HUP
  trap 'exit 143' TERM
  scratch=$(mktemp -d /tmp/studiolo-nas.XXXXXXXX)
  link="$scratch/ssh"
  echo "登录 NAS（${nas[*]}）。要是问密码，问的是 NAS 上 maxsong 的登录密码。"
  # 主连接闲着超过 10 分钟自己退出：这个脚本被强杀、来不及收尾时，已经登录的连接不会一直留在后台。
  ssh "${pin[@]}" -M -S "$link" -o ControlPersist=10m -f -N "${nas[@]}"
  ssh "${pin[@]}" -T -S "$link" "${nas[@]}" 'sudo -n true' \
    || { echo 'NAS 上 maxsong 的 sudo 要密码，这个脚本做不了要 root 的那几步。什么都没改。'; exit 1; }
}

# remote <模式>：帮手脚本走标准输入。
remote() {
  ssh "${pin[@]}" -T -S "$link" "${nas[@]}" "sudo -n python3 -I -B - $1" < "$helper"
}

# send <模式> [文件]：帮手脚本后面跟着要交给它的内容，走同一条标准输入。内容取自给的文件；不给文件就取本函数的标准输入。
# 文件不用「send … < 文件」的写法给：那样整个函数的标准输入都换成了文件，这一步失败时收尾也是在那个状态下跑的，
# 收尾里看「标准输入是不是终端」就看错了。
send() {
  local size
  size=$(( $(wc -c < "$helper") ))
  { cat "$helper"; cat -- "${2:--}"; } \
    | ssh "${pin[@]}" -T -S "$link" "${nas[@]}" "sudo -n python3 -I -B -c '$boot' $1 $size"
}

cleanup() {
  # 问密码时被打断的话，把终端的回显开回来。
  if [[ -t 0 ]]; then stty echo 2>/dev/null || true; fi
  [[ -n "$scratch" ]] || return 0
  [[ -z "$link" ]] || ssh "${pin[@]}" -S "$link" -O exit "${nas[@]}" 2>/dev/null || true
  rm -f "$scratch/ssh"
  rmdir "$scratch" 2>/dev/null || true
}

add() {
  local password again line
  [[ -t 0 && -t 1 && -t 2 ]] || { echo '需要用户自己的终端'; exit 1; }
  connect
  echo "给 NAS 上的新账号 $account 定一个密码（只能追加备份，不能删改）。先存进密码管理器，再到这里输入。"
  echo '要求：16 到 72 个字符，只用英文字母、数字和常见符号，不含空格。'
  IFS= read -r -s -p '密码（不回显）：' password; echo
  IFS= read -r -s -p '再输一遍：' again; echo
  [[ "$password" == "$again" ]] || { echo '两次输入不一样。什么都没改。'; exit 1; }
  valid_password "$password" || { echo '密码不合上面的要求。什么都没改。'; exit 1; }
  # printf 是 bash 自带的，密码只经管道到 htpasswd，不出现在任何进程的命令行里。-n：结果只打到标准输出，
  # htpasswd 不写任何文件（也不建它自己的临时文件）；那一行只留在这个 shell 的变量里。
  line=$(printf '%s\n' "$password" | "$htpasswd" -n -i -B -C 12 "$account") \
    || { echo 'htpasswd 没成功。什么都没改。'; exit 1; }
  unset password again
  account_line "$line" || { echo 'htpasswd 给出的不是预期的那一行。什么都没改。'; exit 1; }
  # 输密码花的时间要是超过了主连接的空闲上限，连接已经自己断了：这里停下，不让后面几步各自再去登录一次。
  ssh "${pin[@]}" -S "$link" -O check "${nas[@]}" 2>/dev/null \
    || { echo '登录 NAS 的那条连接已经断了（闲着超过 10 分钟会自己断）。什么都没改，重跑一遍。'; exit 1; }
  printf '%s\n' "$line" | send append
  unset line
  remote restart \
    || { echo '账号已经加上了，但备份服务重启之后没起来。不要再跑 add（账号已经在，会被拒绝）：查明原因之后跑 nas-account.command restart'; exit 1; }
  remote check
  echo 'NAS 账号加好了。只回复「NAS 账号加好了」，不要发密码或账号文件的内容。'
}

restart() {
  connect
  remote restart
  remote check
  echo '备份服务起来了。'
}

# 单元文件里没有秘密，这一步不问任何东西；帮手脚本只认仓库里这一版的内容。
replace_unit() {
  [[ -f "$unit_file" ]] || { echo "找不到 $unit_file"; exit 1; }
  connect
  send unit "$unit_file"
  remote check
  echo '服务单元换好了。下一次重启 NAS 之后再跑一遍 check：「现在」应该是 active，端口有人监听。'
}

main() {
  case "${1:-}" in
    check)
      connect
      remote check
      ;;
    add) add ;;
    restart) restart ;;
    unit) replace_unit ;;
    *)
      echo '用法：nas-account.command check | add | restart | unit'
      exit 2
      ;;
  esac
}

# 被 source 时只给出上面的函数（用例要单独调它们），不执行。
[[ "${BASH_SOURCE[0]}" != "$0" ]] || main "$@"
