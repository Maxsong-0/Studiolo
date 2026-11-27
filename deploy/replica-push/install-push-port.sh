#!/bin/bash
# T-215 ④（2026-11-27）：给 Mac Studio 的 AI 副本推送在 108 开专用入口。
# Tailscale SSH 只拦 tailnet 地址的 22 端口；这里让 108 自己的 sshd 在 tailnet 地址再听 22169，
# 只认一把专用钥匙、只许转到 169 的 22 端口，不给 shell、不给别的转发。人连 108 照旧走 Tailscale SSH。
# 用法：sudo bash ~/t215-push-port.sh        装上（先校验，不过就什么都不改）
#       sudo bash ~/t215-push-port.sh --rollback   撤回
set -euo pipefail
umask 022
PORT=22169
ADDR=100.84.85.68
TARGET=192.168.100.169:22
KEYS_DIR=/etc/ssh/studiolo-replica-push
KEYS=$KEYS_DIR/authorized_keys
SSHD_DROPIN=/etc/ssh/sshd_config.d/60-studiolo-replica-push.conf
SOCKET_DIR=/etc/systemd/system/ssh.socket.d
SOCKET_DROPIN=$SOCKET_DIR/60-studiolo-replica-push.conf
PUB=/home/maxsong/t215-push.pub
SPEC_22="lport=22,user=maxsong,host=lan,addr=192.168.100.2"
SPEC_PUSH="lport=$PORT,laddr=$ADDR,user=maxsong,host=mac-studio,addr=100.64.0.2"
[[ $(id -u) = 0 ]] || { echo '要用 sudo 跑'; exit 1; }

remove_files() { rm -f -- "$SSHD_DROPIN" "$SOCKET_DROPIN" "$KEYS"; rmdir -- "$KEYS_DIR" "$SOCKET_DIR" 2>/dev/null || :; }

if [[ ${1:-} = --rollback ]]; then
  remove_files
  sshd -t
  systemctl daemon-reload
  systemctl restart ssh.socket
  ss -ltn | grep -q ":$PORT " && { echo "22169 还在听，撤回没成"; exit 1; }
  echo '已撤回：22169 不再听，三个文件已删'; exit 0
fi
[[ $# = 0 ]] || { echo '只认 --rollback 或不带参数'; exit 2; }
for f in "$SSHD_DROPIN" "$SOCKET_DROPIN" "$KEYS_DIR"; do
  [[ ! -e $f ]] || { echo "$f 已经在了：先看清楚再决定，本次什么都没改"; exit 1; }
done
[[ $(wc -l < "$PUB") = 1 ]] || { echo '钥匙文件应只有一行'; exit 1; }
read -r type blob comment < "$PUB"
[[ $type = ssh-ed25519 && $blob =~ ^AAAAC3NzaC1lZDI1NTE5[A-Za-z0-9+/]+=*$ && $comment = studiolo-replica-push@mac-studio ]] \
  || { echo '钥匙文件不对，本次什么都没改'; exit 1; }
before=$(sshd -T -C "$SPEC_22")

install -d -o root -g root -m 0755 "$KEYS_DIR" "$SOCKET_DIR"
printf 'restrict,port-forwarding,permitopen="%s" %s %s %s\n' "$TARGET" "$type" "$blob" "$comment" > "$KEYS"
chown root:root "$KEYS"; chmod 0644 "$KEYS"
cat > "$SSHD_DROPIN" <<EOF
# T-215 ④（2026-11-27）：Mac Studio 推 AI 副本的专用入口，只认一把钥匙、只许转到 169 的 22。
# 端口由 $SOCKET_DROPIN 只在 tailnet 地址上开；撤回用 ~/t215-push-port.sh --rollback。
Match LocalPort $PORT
    AllowUsers maxsong
    PermitRootLogin no
    AuthenticationMethods publickey
    PubkeyAuthentication yes
    PasswordAuthentication no
    KbdInteractiveAuthentication no
    AuthorizedKeysFile $KEYS
    AuthorizedKeysCommand none
    AuthorizedPrincipalsFile none
    AllowTcpForwarding local
    PermitOpen $TARGET
    PermitListen none
    AllowAgentForwarding no
    AllowStreamLocalForwarding no
    X11Forwarding no
    PermitTTY no
    PermitTunnel no
    PermitUserRC no
    GatewayPorts no
    ForceCommand /usr/sbin/nologin
EOF
chmod 0644 "$SSHD_DROPIN"
cat > "$SOCKET_DROPIN" <<EOF
# T-215 ④（2026-11-27）：AI 副本推送专用入口，只听 tailnet 地址（Tailscale SSH 不拦 22 以外的端口）。
[Socket]
ListenStream=$ADDR:$PORT
EOF
chmod 0644 "$SOCKET_DROPIN"

fail() { remove_files; echo "$1；新文件已删，什么都没生效"; exit 1; }
sshd -t || fail 'sshd -t 不过'
after=$(sshd -T -C "$SPEC_22")
[[ "$before" = "$after" ]] || { diff <(echo "$before") <(echo "$after") || :; fail '22 端口的设置变了'; }
echo '== 22 端口：设置和改前一字不差'
push=$(sshd -T -C "$SPEC_PUSH")
echo '== 22169 端口的设置'
for want in "allowusers maxsong" "authenticationmethods publickey" "passwordauthentication no" "kbdinteractiveauthentication no" \
  "authorizedkeysfile $KEYS" "allowtcpforwarding local" "permitopen $TARGET" "permittty no" "x11forwarding no" \
  "allowagentforwarding no" "permittunnel no" "forcecommand /usr/sbin/nologin"; do
  grep -qxF "$want" <<< "$push" || fail "22169 上缺「$want」"
  echo "  $want"
done
systemctl daemon-reload
systemctl restart ssh.socket
sleep 1
systemctl is-active ssh.socket
ss -ltn | grep -E ":(22|$PORT) " | awk '{print "  听 " $4}'
ss -ltn | grep -q "$ADDR:$PORT " || { echo '22169 没听起来'; exit 1; }
rm -f -- "$PUB"
echo '装好了'
