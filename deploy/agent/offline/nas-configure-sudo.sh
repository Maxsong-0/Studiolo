#!/bin/sh
# T-093：仅在本人首次 sudo、私有副本固定 SHA-256 校验后执行。
set -eu
PATH=/usr/sbin:/usr/bin:/sbin:/bin
export PATH
umask 077

fail() {
    printf '%s\n' "T-093 failed: $1" >&2
    exit 1
}

[ "$(/usr/bin/id -u)" = 0 ] || fail root_required
for directory in /etc /etc/sudoers.d; do
    [ -d "$directory" ] && [ ! -L "$directory" ] || fail unsafe_directory
    case "$(/bin/stat -c '%u:%g:%a' "$directory")" in
        0:0:755|0:0:750|0:0:700) ;;
        *) fail unsafe_directory_metadata ;;
    esac
done
[ -f /etc/sudoers ] && [ ! -L /etc/sudoers ] || fail unsafe_sudoers
[ "$(/bin/stat -c '%u:%g:%a:%h' /etc/sudoers)" = 0:0:440:1 ] || fail unsafe_sudoers_metadata
/usr/bin/awk '
    $1 ~ /^([#@]includedir)$/ && $2 == "/etc/sudoers.d" && NF == 2 {found=1}
    END {exit !found}
' /etc/sudoers || fail includedir_not_confirmed

target=/etc/sudoers.d/zz-studiolo-t093-maxsong
[ ! -e "$target" ] && [ ! -L "$target" ] || fail target_exists
visudo=$(command -v visudo 2>/dev/null || :)
if [ -n "$visudo" ]; then
    [ -f "$visudo" ] && [ ! -L "$visudo" ] || fail unsafe_visudo
    case "$(/bin/stat -c '%u:%g:%a' "$visudo")" in
        0:0:755|0:0:750|0:0:700|0:0:555) ;;
        *) fail unsafe_visudo_metadata ;;
    esac
    "$visudo" -c -q || fail existing_policy_invalid
fi

tmp=
identity=
installed=0
verified=0
remove_owned() {
    current=$(/bin/stat -c '%d:%i' "$1" 2>/dev/null || :)
    if [ -n "$2" ] && [ "$current" = "$2" ]; then
        /bin/rm "$1"
    fi
}
cleanup() {
    result=$?
    trap - 0 1 2 15
    if [ -n "$tmp" ]; then
        remove_owned "$tmp" "$identity" || result=1
    fi
    if [ "$result" != 0 ] || [ "$verified" != 1 ]; then
        if [ "$installed" = 1 ]; then
            remove_owned "$target" "$identity" || result=1
        fi
        [ "$result" != 0 ] || result=1
    fi
    exit "$result"
}
trap cleanup 0
trap 'exit 1' 1 2 15

tmp=$(/usr/bin/mktemp /etc/sudoers.d/.studiolo-t093-XXXXXX)
identity=$(/bin/stat -c '%d:%i' "$tmp")
printf '%s\n' 'maxsong ALL=(ALL:ALL) NOPASSWD: ALL' > "$tmp"
/bin/chown 0:0 "$tmp"
/bin/chmod 0440 "$tmp"
[ "$(/bin/stat -c '%u:%g:%a:%h' "$tmp")" = 0:0:440:1 ] || fail unsafe_candidate
if [ -n "$visudo" ]; then
    "$visudo" -c -q -f "$tmp" || fail candidate_invalid
fi
# 同目录硬链接是原子且不覆盖的发布；点号临时名不被 includedir 加载。
installed=1
/bin/ln "$tmp" "$target" || fail publish_failed
[ "$(/bin/stat -c '%d:%i' "$target")" = "$identity" ] || fail target_changed
if [ -n "$visudo" ]; then
    "$visudo" -c -q || fail resulting_policy_invalid
fi
# 外层降为 maxsong；内层 -k 忽略缓存，真实解析 NAS 策略并检查目标 uid。
/usr/bin/sudo -n -u maxsong /usr/bin/sudo -n -k /bin/sh -c 'test "$(id -u)" = 0' || fail policy_or_elevation_failed
verified=1
printf '%s\n' 'T-093 NOPASSWD ALL parsed and verified for maxsong.'
