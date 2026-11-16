# shellcheck shell=bash
# 测试箱步骤单（.testbox.toml）每一步开头 source 这一份，把原来 GitHub CI runner 给的环境在沙盒里补齐。
# 用法：source scripts/testbox-ci-env.sh [Node 版本]（默认 22.17.0；告警 Worker 那几步传 24.22.2）。
# 只在测试箱沙盒里用：它会换掉 HOME、TMPDIR 和构建目录。
#
# 2026-11-16 在 108 上实测的沙盒和 CI runner 的差别，以及这里怎么补（不改任何测试的语义）：
# - umask 是 002：捕获端拒绝组可写节点，夹具按默认权限建目录 → 设回 022（同 test-linux.sh）。
# - 家目录 /home/testbox、临时目录 /var/tmp 都是 775 → 换成本次任务私有的目录（家 755、临时 700）。
#   /var/tmp 每个任务一份（任务结束随沙盒删掉），和 /work 同一块盘，Broker 的同设备检查照常成立。
# - 默认构建目录 /cache/cargo-target 是跨任务共用的缓存，并发任务的产物会互相覆盖 → 换成本次任务自己的。
#   同一个任务的各步共用它，所以 clippy、测试、构建之间照样增量。
# - 自带 Node 是 26 → 前端照 CI 用 22.17.0，告警 Worker 用 24.22.2；没装过就从 nodejs.org 下载，
#   核对下面钉住的 SHA-256（取自 nodejs.org/dist/v<版本>/SHASUMS256.txt，2026-11-16）再装进 /cache。
# - python3 指到 3.12（CI 跑的也是 3.12）。
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

studiolo_ci_base=/var/tmp/studiolo-ci
mkdir -p "$studiolo_ci_base"
chmod 700 "$studiolo_ci_base"
mkdir -p -m 755 "$studiolo_ci_base/home" "$studiolo_ci_base/bin"
mkdir -p -m 700 "$studiolo_ci_base/tmp" "$studiolo_ci_base/target"
ln -sfn "$(command -v python3.12)" "$studiolo_ci_base/bin/python3"
export HOME="$studiolo_ci_base/home" TMPDIR="$studiolo_ci_base/tmp" CARGO_TARGET_DIR="$studiolo_ci_base/target"

# 装好的 Node 目录；没装过就下载、核对摘要、解压后原子改名（并发任务先装好的就用它的）。
studiolo_ci_node() {
  local version="$1" arch sum dir staging
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
  dir="/cache/studiolo-node/node-v$version-linux-$arch"
  if [ ! -x "$dir/bin/node" ]; then
    mkdir -p /cache/studiolo-node
    staging="$(mktemp -d /cache/studiolo-node/.staging-XXXXXX)" || return 1
    if ! curl -fsSL "https://nodejs.org/dist/v$version/node-v$version-linux-$arch.tar.xz" -o "$staging/node.tar.xz" \
      || ! echo "$sum  $staging/node.tar.xz" | sha256sum -c --quiet - \
      || ! tar -xJf "$staging/node.tar.xz" -C "$staging"; then
      rm -rf "$staging"
      echo "Node $version 下载或核对失败" >&2
      return 1
    fi
    mv -T "$staging/node-v$version-linux-$arch" "$dir" 2>/dev/null || true
    rm -rf "$staging"
    [ -x "$dir/bin/node" ] || { echo "Node $version 没装上" >&2; return 1; }
  fi
  printf '%s\n' "$dir"
}

studiolo_ci_node_dir="$(studiolo_ci_node "${1:-22.17.0}")" || return 1
export PATH="$studiolo_ci_base/bin:$studiolo_ci_node_dir/bin:$PATH"
