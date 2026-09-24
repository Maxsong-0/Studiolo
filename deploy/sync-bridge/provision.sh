#!/usr/bin/env bash
# T-132 P1（ADR-025 D2／D7）：一次置备可信 launcher 的全部固定物——专用身份 studiolo-sync、
# 4 GiB loop 镜像盘与它的 mount 单元、owner 的 idmapped 视图 mount 单元、launcher 二进制、
# 固定配置 /etc/studiolo/sync-launcher.json、root 系统服务。由用户本人在服务器以 root 执行：
#   sudo bash <仓库>/deploy/sync-bridge/provision.sh /ABS/PATH/bin/studiolo-sync-launcher
# 幂等：已存在的镜像、配置、身份不动；单元与二进制按内容比对后才替换。本脚本不调用 sudo、
# 不重启别的服务、不写任何凭据、不安装 Node／obsidian-headless（运行时闭包另装，见 README）。
set -Eeuo pipefail
umask 022
export PATH=/usr/sbin:/usr/bin:/sbin:/bin
unset CDPATH ENV BASH_ENV PYTHONPATH PYTHONHOME

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)
owner=maxsong
payload=studiolo-sync
image=/var/lib/studiolo-sync.img
image_size=4G
mount_point=/var/lib/studiolo-sync
view_point=/var/lib/studiolo-owner/mirror
launcher=/usr/local/sbin/studiolo-sync-launcher
config_dir=/etc/studiolo
config=$config_dir/sync-launcher.json
unit_dir=/etc/systemd/system
service=studiolo-sync-launcher.service
socket_dir=/run/studiolo-sync-launcher
socket=$socket_dir/launcher.sock

fail() { printf '%s\n' "$1" >&2; exit 1; }
digest() { sha256sum -- "$1" | cut -d ' ' -f 1; }

[[ $EUID == 0 ]] || fail '请以 root 运行：sudo bash provision.sh <launcher 二进制绝对路径>。'
[[ $# == 1 && $1 == /* && -f $1 && ! -L $1 ]] || fail '唯一参数须是 launcher 二进制的绝对路径（普通文件）。'
source_binary=$1
command -v systemd-escape fallocate mkfs.ext4 install python3 findmnt useradd >/dev/null ||
    fail '缺 systemd-escape／fallocate／mkfs.ext4／install／python3／findmnt／useradd。'
id -u "$owner" >/dev/null 2>&1 || fail "owner 账号 $owner 不存在。"
owner_uid=$(id -u "$owner"); owner_gid=$(id -g "$owner")
[[ $(stat -c '%u' "$source_binary") == 0 || $(stat -c '%u' "$source_binary") == "$owner_uid" ]] ||
    fail '二进制须属 root 或 owner。'
(( (8#$(stat -c '%a' "$source_binary") & 022) == 0 )) || fail '二进制不能组或其他用户可写。'
loop_unit=$(systemd-escape -p --suffix=mount "$mount_point")
view_unit=$(systemd-escape -p --suffix=mount "$view_point")
for template in "$service.in" "$loop_unit.in" "$view_unit.in" sync-launcher.example.json; do
    [[ -f "$here/$template" && ! -L "$here/$template" ]] || fail "缺模板：$here/$template"
done

# 1 专用身份：系统账号、家目录就是镜像挂载点（不创建）、不可登录、独立主组、无补充组。
if ! id -u "$payload" >/dev/null 2>&1; then
    useradd --system --no-create-home --home-dir "$mount_point" --shell /usr/sbin/nologin --user-group "$payload"
fi
payload_uid=$(id -u "$payload"); payload_gid=$(id -g "$payload")
[[ $(id -G "$payload") == "$payload_gid" ]] || fail "$payload 不得有补充组。"
[[ $payload_uid != 0 && $payload_uid != "$owner_uid" && $payload_gid != "$owner_gid" ]] || fail '专用身份与 owner 重合。'

# 2 loop 镜像：固定 4 GiB ext4，root 0600；已存在就不动（配额就是镜像大小）。
if [[ ! -e $image ]]; then
    fallocate -l "$image_size" "$image"
    chmod 0600 "$image"
    mkfs.ext4 -q -F -L studiolo-sync "$image"
fi
[[ -f $image && ! -L $image && $(stat -c '%u:%a:%h' "$image") == 0:600:1 ]] || fail "$image 不是 root 0600 单链接普通文件。"
install -d -m 0755 -o root -g root "$mount_point" "$(dirname "$view_point")" "$view_point"

# 3 单元：模板替换身份数字与二进制路径后按内容比对，变了才落盘。
render() {
    local rendered
    rendered=$(sed -e "s|@LAUNCHER@|$launcher|g" -e "s|@PAYLOAD_UID@|$payload_uid|g" -e "s|@PAYLOAD_GID@|$payload_gid|g" \
        -e "s|@OWNER_UID@|$owner_uid|g" -e "s|@OWNER_GID@|$owner_gid|g" "$1")
    if [[ ! -f $2 ]] || [[ $(cat "$2") != "$rendered" ]]; then
        printf '%s\n' "$rendered" > "$2.tmp"
        chmod 0644 "$2.tmp"
        mv -f "$2.tmp" "$2"
    fi
}
render "$here/$loop_unit.in" "$unit_dir/$loop_unit"
render "$here/$view_unit.in" "$unit_dir/$view_unit"
render "$here/$service.in" "$unit_dir/$service"

# 4 二进制：按摘要比对；替换用同目录临时文件 + mv 保持原子。
if [[ ! -f $launcher ]] || [[ $(digest "$launcher") != $(digest "$source_binary") ]]; then
    install -o root -g root -m 0755 "$source_binary" "$launcher.tmp"
    mv -f "$launcher.tmp" "$launcher"
fi

# 5 固定配置：只在不存在时从样例生成（身份数字改成本机真实值）；已存在的配置不覆盖。
install -d -m 0755 -o root -g root "$config_dir"
if [[ ! -e $config ]]; then
    python3 - "$here/sync-launcher.example.json" "$config.tmp" "$owner_uid" "$owner_gid" "$payload_uid" "$payload_gid" <<'PY'
import json
import sys
source, target, owner_uid, owner_gid, payload_uid, payload_gid = sys.argv[1:]
with open(source, encoding='utf-8') as handle:
    document = json.load(handle)
document.update(owner_uid=int(owner_uid), owner_gid=int(owner_gid),
                payload_uid=int(payload_uid), payload_gid=int(payload_gid))
with open(target, 'w', encoding='utf-8') as handle:
    json.dump(document, handle, ensure_ascii=False, indent=2)
    handle.write('\n')
PY
    chown root:root "$config.tmp"
    chmod 0600 "$config.tmp"
    mv -f "$config.tmp" "$config"
fi
[[ -f $config && ! -L $config && $(stat -c '%u:%a:%h' "$config") == 0:600:1 ]] || fail "$config 须为 root 0600 单链接普通文件。"

# 6 挂载与目录：先起镜像盘，再在盘上建镜像根与配置根（payload 专属 0700），再起视图，最后起服务。
systemctl daemon-reload
systemctl enable --now -- "$loop_unit"
install -d -m 0700 -o "$payload" -g "$payload" "$mount_point/mirror" "$mount_point/config"
systemctl enable --now -- "$view_unit"
systemctl enable --now -- "$service"

# 7 逐项核验并打印；任一项不符即失败（单元已启用，可按 README 回滚）。
loop_source=$(findmnt -no SOURCE -- "$mount_point") || fail '镜像盘未挂载。'
[[ $loop_source == /dev/loop* ]] || fail "镜像盘不是 loop 设备：$loop_source"
[[ $(cat "/sys/block/${loop_source##*/}/loop/backing_file") == "$image" ]] || fail 'loop 设备的 backing_file 不是固定镜像。'
findmnt -no OPTIONS -- "$view_point" | grep -q idmapped || fail '视图不是 idmapped 挂载。'
[[ $(stat -c '%u:%g' "$view_point") == "$owner_uid:$owner_gid" ]] || fail '视图根没有映射成 owner 身份。'
[[ $(stat -c '%u:%g:%a' "$mount_point/mirror") == "$payload_uid:$payload_gid:700" ]] || fail '镜像根不是 payload 专属 0700。'
systemctl is-active --quiet -- "$service" || fail 'launcher 服务未运行。'
for attempt in $(seq 1 40); do
    [[ -S $socket ]] && break
    sleep 0.25
done
[[ -S $socket && $(stat -c '%u:%g:%a' "$socket") == "0:$owner_gid:660" ]] || fail 'socket 未就绪或权限不对。'
[[ $(stat -c '%u:%g:%a' "$socket_dir") == "0:$owner_gid:750" ]] || fail 'socket 目录权限不对。'
printf '置备完成：\n'
printf '  身份   %s uid=%s gid=%s（owner %s uid=%s gid=%s）\n' "$payload" "$payload_uid" "$payload_gid" "$owner" "$owner_uid" "$owner_gid"
printf '  镜像盘 %s → %s（%s，backing_file 已核对）\n' "$image" "$mount_point" "$loop_source"
printf '  视图   %s（idmapped，显示为 owner）\n' "$view_point"
printf '  程序   %s sha256=%s\n' "$launcher" "$(digest "$launcher")"
printf '  配置   %s（0600 root；runtime 路径与 closure_digest 请按 README 第 2 节核对后再重启服务）\n' "$config"
printf '  服务   %s active；socket %s（0660 root:%s）\n' "$service" "$socket" "$owner_gid"
