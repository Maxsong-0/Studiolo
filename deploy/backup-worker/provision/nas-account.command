#!/bin/bash
# 用户在自己的 Mac 终端运行；不要通过 agent 工具执行本文件。
# 给备份 worker 在 NAS 的 rest-server 上加一个只追加账号（README「安装前要有的东西」第 5 项的 NAS 一侧）。
#   nas-account.command check   只读：NAS 上备份服务、账号文件、证书的现状，不写任何东西
#   nas-account.command add     设新账号的密码 → 加进 NAS 的账号文件 → 重启备份服务 → 再看一遍现状
# 怎么连 NAS：默认经 108 跳过去；人在家里局域网上可以先 export STUDIOLO_NAS_SSH="maxsong@192.168.100.111" 直连。
# 登录 NAS 由 ssh 自己办：这台 Mac 的钥匙在 NAS 上就直接过；不在，ssh 会在终端里问 NAS 的登录密码（本脚本不经手）。
# 一次运行只登录一回，后面几步共用这一条连接。NAS 上要 root 的几步用 sudo -n：sudo 要密码的话脚本做不了，会在改任何东西之前停下。
# 密码在本机不回显地问两遍，先核过规则再经管道交给 htpasswd（不进任何命令行）；传到 NAS 的只有 bcrypt 散列那一行。
# 帮手脚本和那一行都走 ssh 的标准输入，NAS 上不落任何文件；ssh 一律 -T，不要远端终端（有终端就会把输入回显出来）。
# 清理权限照旧只在 NAS 管理员手里。
set -euo pipefail
umask 077
export PATH=/usr/bin:/bin:/usr/sbin
unset BASH_ENV ENV CDPATH
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
helper="$here/nas_account.py"
account=studiolo-backup-worker
scratch=''
link=''
read -r -a nas <<< "${STUDIOLO_NAS_SSH:--J studiolo-108-ts maxsong@192.168.100.111}"
[[ -f "$helper" ]] || { echo "找不到 $helper"; exit 1; }
# 追加那一步，标准输入里先是帮手脚本、后面跟着账号行：按字节数读出脚本来执行，剩下的那一行留给脚本自己读。
boot='import sys; exec(compile(sys.stdin.buffer.read(int(sys.argv.pop())), "nas_account.py", "exec"))'

# worker 读凭据时拒绝首尾空白和控制字符（crates/backup-worker/src/credentials/file.rs）。这里收得更紧，
# 免得 NAS 上已经加了账号、169 那边却不认这个密码：16 到 1024 个可见 ASCII 字符，不含空格。按字节判断，不看语言环境。
valid_password() {
  local LC_ALL=C
  [[ ${#1} -ge 16 && ${#1} -le 1024 && $1 =~ ^[!-~]+$ ]]
}

# 登录一回，后面共用。控制口的路径有长度上限（macOS 104 字节，ssh 建的时候还要加 17 个字符），所以放在 /tmp 下的短目录里。
connect() {
  scratch=$(mktemp -d /tmp/studiolo-nas.XXXXXXXX)
  trap cleanup EXIT
  link="$scratch/ssh"
  echo "登录 NAS（${nas[*]}）。要是问密码，问的是 NAS 上 maxsong 的登录密码。"
  ssh -M -S "$link" -o ControlPersist=no -f -N "${nas[@]}"
  ssh -T -S "$link" "${nas[@]}" 'sudo -n true' \
    || { echo 'NAS 上 maxsong 的 sudo 要密码，这个脚本做不了要 root 的那几步。什么都没改。'; exit 1; }
}

# remote <模式>：帮手脚本走标准输入。
remote() {
  ssh -T -S "$link" "${nas[@]}" "sudo -n python3 -I -B - $1" < "$helper"
}

cleanup() {
  [[ -n "$scratch" ]] || return 0
  [[ -z "$link" ]] || ssh -S "$link" -O exit "${nas[@]}" 2>/dev/null || true
  rm -f "$scratch/line" "$scratch/ssh"
  rmdir "$scratch" 2>/dev/null || true
}

add() {
  local password again size
  [[ -t 0 && -t 1 && -t 2 ]] || { echo '需要用户自己的终端'; exit 1; }
  connect
  echo "给 NAS 上的新账号 $account 定一个密码（只能追加备份，不能删改）。先存进密码管理器，再到这里输入。"
  echo '要求：16 到 1024 个字符，只用英文字母、数字和常见符号，不含空格。'
  IFS= read -r -s -p '密码（不回显）：' password; echo
  IFS= read -r -s -p '再输一遍：' again; echo
  [[ "$password" == "$again" ]] || { echo '两次输入不一样。什么都没改。'; exit 1; }
  valid_password "$password" || { echo '密码不合上面的要求。什么都没改。'; exit 1; }
  # printf 是 bash 自带的，密码只经管道到 htpasswd，不出现在任何进程的命令行里。
  printf '%s\n' "$password" | /usr/sbin/htpasswd -c -i -B -C 12 "$scratch/line" "$account"
  unset password again
  size=$(( $(wc -c < "$helper") ))
  { cat "$helper"; cat "$scratch/line"; } \
    | ssh -T -S "$link" "${nas[@]}" "sudo -n python3 -I -B -c '$boot' append $size"
  remote restart
  remote check
  echo 'NAS 账号加好了。只回复「NAS 账号加好了」，不要发密码或账号文件的内容。'
}

main() {
  case "${1:-}" in
    check)
      connect
      remote check
      ;;
    add) add ;;
    *)
      echo '用法：nas-account.command check | add'
      exit 2
      ;;
  esac
}

# 被 source 时只给出上面的函数（用例要单独调 valid_password），不执行。
[[ "${BASH_SOURCE[0]}" != "$0" ]] || main "$@"
