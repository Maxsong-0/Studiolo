#!/bin/sh
# 用户本人在私密终端运行；首次 NAS sudo 密码由 sudo 直接读取。
exec ssh -t -o ConnectTimeout=5 maxsong@192.168.100.111 '/usr/bin/sudo /bin/sh -c '"'"'set -eu
umask 077
[ "$(/usr/bin/id -u)" = 0 ]
[ -d /etc ] && [ ! -L /etc ]
[ "$(/bin/stat -c "%u:%g:%a" /etc)" = "0:0:755" ]
p=/volume1/homes/maxsong/studiolo-nas-sudo-t093-r2.sh
[ -f "$p" ] && [ ! -L "$p" ] || exit 1
t=$(/usr/bin/mktemp /etc/.studiolo-t093-bootstrap-XXXXXX)
i=$(/bin/stat -c "%d:%i" "$t")
cleanup() { s=$?; trap - 0 1 2 15; a=$(/bin/stat -c "%d:%i" "$t" 2>/dev/null || :); if [ "$a" = "$i" ]; then /bin/rm "$t" || s=1; fi; exit "$s"; }
trap cleanup 0
trap "exit 1" 1 2 15
/usr/bin/cat "$p" > "$t"
/bin/chmod 0600 "$t"
[ "$(/bin/stat -c "%u:%g:%a:%h" "$t")" = "0:0:600:1" ]
h=$(/usr/bin/sha256sum "$t")
[ "${h%% *}" = "293010a2adb579d5605798b02b819cd525395666bda722d80a4afd0b5a204dae" ] || { printf "%s\n" "STOP: SHA-256 mismatch." >&2; exit 1; }
/bin/sh "$t"
'"'"''
