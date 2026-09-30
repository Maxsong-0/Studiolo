#!/bin/bash
# 用户在自己的 Mac 终端运行；不要通过 agent 工具执行本文件。
set -euo pipefail
[[ -t 0 && -t 1 && -t 2 ]] || { echo '需要用户私有终端'; exit 1; }
umask 077
export PATH=/opt/homebrew/bin:/usr/bin:/bin
unset BASH_ENV ENV CDPATH
scratch=$(mktemp -d "${TMPDIR:-/tmp}/studiolo-backup.XXXXXXXX")
trap 'rm -f "$scratch/backup.htpasswd" "$scratch/ca.pem"; rmdir "$scratch"' EXIT
echo '设置 NAS 追加备份账号密码，请存入密码管理器。清理权限保留给现有 NAS 管理员，不给这个账号。'
/opt/homebrew/bin/htpasswd -c -B -C 12 "$scratch/backup.htpasswd" studiolo-backup
ssh -o BatchMode=yes maxsong@192.168.100.111 \
  'sudo -n python3 -I -B /usr/local/libexec/studiolo-p3/nas_credentials.py install-auth' < "$scratch/backup.htpasswd"
ssh -t maxsong@192.168.100.111 \
  'sudo -n python3 -I -B /usr/local/libexec/studiolo-p3/nas_credentials.py activate'
# 仅公开 TLS 证书跨机传输；私钥留在 NAS。
ssh -o BatchMode=yes maxsong@192.168.100.111 \
  'sudo -n cat /volume1/studiolo-dr/rest-server/tls.crt' > "$scratch/ca.pem"
ssh -o BatchMode=yes maxsong@192.168.100.169 \
  'umask 077; set -C; cat > /home/maxsong/.config/studiolo-backup/ca.pem' < "$scratch/ca.pem"
ssh -t maxsong@192.168.100.169 \
  'python3 -I -B /usr/local/libexec/studiolo-p3/client_credentials.py'
echo '备份身份配置完成。只回复“备份配置完成”，不要发送口令、证书或配置文件。'
