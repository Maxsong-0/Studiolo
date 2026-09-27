#!/usr/bin/env bash
# 在 Cursor Cloud VM 上原生跑检查；对应 scripts/test-linux.sh 带参数的用法。
# VM 是原生 Linux x86_64，但连不到 githubci（test-linux.sh 不可用），且有两处与 CI 机器不同：
# - 根文件系统是各层不在同一文件系统上的 overlayfs，目录与普通文件报告不同的 st_dev。
#   仓库里「文件须与父目录同设备」的检查（vault-broker 的 blob 身份、Python 副本／快照／发行工具）
#   对建在 /tmp 或 $HOME 下的测试夹具因此失败关闭。这里在私有 mount 命名空间里挂一个临时 ext4
#   （与 CI 机器同类；tmpfs 上文件操作用例并发时会偶发失败），/tmp 绑定到其中，HOME 指向其中的
#   空目录（祖先目录 root 所有、0755，满足发行工具的祖先检查）。只对本命令及其子进程可见，
#   VM 上其他进程看到的 /tmp 与家目录不变；镜像挂载后即删除，命名空间结束时随回环设备释放。
# - RLIMIT_MEMLOCK 默认 8 MiB，秘密驻留面加固（mlockall）会失败关闭；与 CI 一样抬到 unlimited。
# 命令以调用者身份、umask 022 执行；/tmp 与 HOME 都是空的，命令不要引用 VM 上这两处的文件。
#
# 用法：scripts/test-cursor-cloud.sh cargo --locked test --workspace
#       scripts/test-cursor-cloud.sh python3 scripts/test-sync-agent-vault.py
set -euo pipefail
if [ "$#" -eq 0 ]; then
  printf 'usage: scripts/test-cursor-cloud.sh <命令> [参数...]\n' >&2
  exit 2
fi
cd "$(dirname "${BASH_SOURCE[0]}")/.."
export CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}" RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}"
exec sudo --preserve-env unshare --mount -- /bin/sh -euc '
  uid=$1 gid=$2 path=$3
  shift 3
  image=$(mktemp /var/tmp/studiolo-scratch.XXXXXXXX)
  trap "rm -f \"\$image\"" EXIT
  truncate -s 16G "$image"
  mkfs.ext4 -q -F -m 0 "$image"
  mount -o loop "$image" /mnt
  rm -f "$image"
  install -d -m 1777 /mnt/tmp
  install -d -o "$uid" -g "$gid" -m 0700 /mnt/home
  mount --bind /mnt/tmp /tmp
  ulimit -l unlimited
  umask 022
  exec setpriv --reuid="$uid" --regid="$gid" --init-groups -- env PATH="$path" HOME=/mnt/home "$@"
' sh "$(id -u)" "$(id -g)" "$PATH" "$@"
