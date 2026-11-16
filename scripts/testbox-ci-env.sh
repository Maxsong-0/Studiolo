# shellcheck shell=bash
# 测试箱步骤单（.testbox.toml）每一步开头 source 这一份，把原来 GitHub CI runner 给的环境在沙盒里补齐。
# 用法：source scripts/testbox-ci-env.sh [Node 版本]（默认 22.17.0；告警 Worker 那几步传 24.22.2）。
# 只在测试箱沙盒里用：它会换掉 HOME、TMPDIR 和构建目录。
#
# 2026-11-16 在 108 上实测的沙盒和 CI runner 的差别，以及这里怎么补（不改任何测试的语义）：
# - umask 是 002：捕获端拒绝组可写节点，夹具按默认权限建目录 → 设回 022（同 test-linux.sh）。
# - 家目录 /home/testbox、临时目录 /var/tmp 都是 775 → 换成本次任务私有的目录（家 755、临时 700）。
#   /var/tmp 每个任务一份（任务结束随沙盒删掉），和 /work 同一块盘，Broker 的同设备检查照常成立。
#   告警状态目录、M4 恢复工具、副本管线要求路径上每一级祖先都不可被组写（root 所有带粘滞位的除外），
#   所以 /var/tmp 自己也收成 755（它是本任务私有的）。
# - 默认构建目录 /cache/cargo-target 是跨任务共用的缓存，并发任务的产物会互相覆盖 → 换成本次任务自己的。
#   同一个任务的各步共用它，所以 clippy、测试、构建之间照样增量。
# - 自带 Node 是 26 → 前端照 CI 用 22.17.0，告警 Worker 用 24.22.2，每个任务解压到自己的私有目录。
#   /cache 跨任务共用、别的任务改得了，所以那里只存下载的压缩包，每次用之前复制到私有目录再核对下面钉住的
#   SHA-256（取自 nodejs.org/dist/v<版本>/SHASUMS256.txt，2026-11-16）；对不上就从 nodejs.org 重新下载。
# - python3 指到 3.12（CI 跑的也是 3.12）。
# - 原 workflow 要自己装 Rust 工具链和 rustfmt、clippy；测试箱的工具链由平台只读提供（/opt/testbox/toolchains/rust，
#   2026-11-16 实测 1.97.1 带 rustfmt、clippy，`rustup toolchain install` 会因为目录只读而报错），这里不装。
#   缺了组件时 fmt、clippy 那两步自己会失败，不会被放过。
# - 没有 sudo：CI 用 prlimit 抬 RLIMIT_MEMLOCK，沙盒本来就是 unlimited；不是就直接失败——秘密驻留面加固
#   失败关闭是正确行为，不降级。

if [ -z "${TESTBOX_APT_ROOT:-}" ] || [ ! -d /work ] || [ ! -d /cache ]; then
  echo "testbox-ci-env.sh 只在测试箱沙盒里用（它会换掉 HOME、TMPDIR 和构建目录）" >&2
  return 1
fi
if [ "$(ulimit -l)" != unlimited ]; then
  echo "ulimit -l 是 $(ulimit -l)，不是 unlimited：秘密驻留面加固会失败关闭，先换一台测试节点" >&2
  return 1
fi

umask 022
export CARGO_TERM_COLOR=never PYTHONDONTWRITEBYTECODE=1 CARGO_PROFILE_DEV_DEBUG=line-tables-only CARGO_INCREMENTAL=0

chmod go-w /var/tmp || { echo "收不了 /var/tmp 的组写权限：私有目录的祖先检查会失败" >&2; return 1; }
studiolo_ci_base=/var/tmp/studiolo-ci
mkdir -p "$studiolo_ci_base"
chmod 700 "$studiolo_ci_base"
mkdir -p -m 755 "$studiolo_ci_base/home" "$studiolo_ci_base/bin"
mkdir -p -m 700 "$studiolo_ci_base/tmp" "$studiolo_ci_base/target"
ln -sfn "$(command -v python3.12)" "$studiolo_ci_base/bin/python3"
export HOME="$studiolo_ci_base/home" TMPDIR="$studiolo_ci_base/tmp" CARGO_TARGET_DIR="$studiolo_ci_base/target"

# 本任务私有的 Node 目录；同一任务里前面的步骤解好了就直接用。缓存里的压缩包只当省一次下载：
# 先复制到私有目录再核对摘要（核对完到解压之间别的任务换不了它），对不上就重新下载。
studiolo_ci_node() {
  local version="$1" arch sum name dir tarball staging
  case "$(uname -m)" in
    x86_64) arch=x64 ;;
    aarch64) arch=arm64 ;;
    *) echo "不认识的架构 $(uname -m)" >&2; return 1 ;;
  esac
  case "$version-$arch" in
    22.17.0-x64) sum=325c0f1261e0c61bcae369a1274028e9cfb7ab7949c05512c5b1e630f7e80e12 ;;
    22.17.0-arm64) sum=140aee84be6774f5fb3f404be72adbe8420b523f824de82daeb5ab218dab7b18 ;;
    24.22.2-x64) sum=48cc0e21e048790e54c0cf5749c2725005313d652651c40b7a21feb9697aaf75 ;;
    24.22.2-arm64) sum=eb187837a2230d4047161ac0be008584e16b46f726e9c9490ee35dc6b17d9ba8 ;;
    *) echo "没有钉住 Node $version（$arch）的摘要" >&2; return 1 ;;
  esac
  name="node-v$version-linux-$arch"
  dir="$studiolo_ci_base/$name"
  if [ ! -x "$dir/bin/node" ]; then
    tarball="$(mktemp "$studiolo_ci_base/.node-tar-XXXXXX")" || return 1
    cp "/cache/studiolo-node/$name.tar.xz" "$tarball" 2>/dev/null || true
    if ! echo "$sum  $tarball" | sha256sum -c --quiet - >/dev/null 2>&1; then
      if ! curl -fsSL "https://nodejs.org/dist/v$version/$name.tar.xz" -o "$tarball" \
        || ! echo "$sum  $tarball" | sha256sum -c --quiet -; then
        rm -f "$tarball"
        echo "Node $version 下载或核对失败" >&2
        return 1
      fi
      # 核对过的压缩包放回缓存给后面的任务省一次下载；放不回去不影响这一步。
      if mkdir -p /cache/studiolo-node && staging="$(mktemp /cache/studiolo-node/.download-XXXXXX)"; then
        cp "$tarball" "$staging" && mv -f "$staging" "/cache/studiolo-node/$name.tar.xz" || rm -f "$staging"
      fi
    fi
    staging="$(mktemp -d "$studiolo_ci_base/.node-XXXXXX")" || { rm -f "$tarball"; return 1; }
    tar -xJf "$tarball" -C "$staging" && mv -T "$staging/$name" "$dir" 2>/dev/null
    rm -rf "$staging" "$tarball"
    # 并行的步骤先解好了也算：内容来自同一个核对过的压缩包。
    [ -x "$dir/bin/node" ] || { echo "Node $version 解压失败" >&2; return 1; }
  fi
  printf '%s\n' "$dir"
}

studiolo_ci_node_dir="$(studiolo_ci_node "${1:-22.17.0}")" || return 1
export PATH="$studiolo_ci_base/bin:$studiolo_ci_node_dir/bin:$PATH"
