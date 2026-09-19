#!/usr/bin/env bash
# 以 ghrunner 身份（无特权）为一个仓库准备 runner 实例：下载并验签官方 runner、装 Rust 工具链、写作业钩子。
# 用法：bash install-runner.sh <实例名> <runner 版本> <该版本 linux-x64 包的 sha256>
# 版本与 sha256 取自 https://github.com/actions/runner/releases 的发布说明；不验签不解包。
set -Eeuo pipefail
umask 077
[[ "$(id -un)" == ghrunner && $# == 3 && "$1" =~ ^[a-z0-9][a-z0-9-]{0,39}$ && "$2" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ && "$3" =~ ^[0-9a-f]{64}$ ]] \
  || { echo '须以 ghrunner 执行：install-runner.sh <实例名> <版本> <sha256>' >&2; exit 2; }
name=$1 version=$2 sha=$3
dir=$HOME/runners/$name
[[ ! -e "$dir/.runner" ]] || { echo "实例 $name 已注册，先注销再重装。" >&2; exit 1; }
mkdir -p "$dir" "$HOME/hooks"
tarball=$(mktemp)
trap 'rm -f "$tarball"' EXIT
curl --fail --silent --show-error --location --proto '=https' --tlsv1.2 \
  "https://github.com/actions/runner/releases/download/v$version/actions-runner-linux-x64-$version.tar.gz" -o "$tarball"
printf '%s  %s\n' "$sha" "$tarball" | sha256sum --check --status || { echo 'runner 包摘要不符，拒绝解包。' >&2; exit 1; }
tar -xzf "$tarball" -C "$dir"

# 容器作业以 root 写工作区（node_modules、dist），下一个宿主作业的 checkout 清不掉它们。作业开始前把属主改回来。
cat > "$HOME/hooks/job-started.sh" <<'HOOK'
#!/usr/bin/env bash
set -euo pipefail
work="${GITHUB_WORKSPACE:-}"
[[ -n "$work" && -d "$work" ]] || exit 0
if find "$work" -xdev ! -user "$(id -u)" -print -quit | grep -q .; then
  docker run --rm -v "$work":/w busybox:stable chown -R "$(id -u):$(id -g)" /w
fi
HOOK
chmod 700 "$HOME/hooks/job-started.sh"
# 构建目录常驻在工作区之外：checkout 每次都会清工作区，放里面等于每次全量重编。
mkdir -p "$HOME/cache/$name"
# 作业的 PATH 也写在 .env 里：runner 进程启动时自己读 .env 并设进环境。官方的 .path 文件只有 bin/runsvc.sh 会读，
# 而本机的 systemd 单元直接起 run.sh（2026-09-19 头两轮 CI 因此 `rustup: command not found`）。
printf 'ACTIONS_RUNNER_HOOK_JOB_STARTED=%s\nCARGO_TARGET_DIR=%s\nPATH=%s\n' "$HOME/hooks/job-started.sh" "$HOME/cache/$name/target" \
  "$HOME/.cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin" > "$dir/.env"
docker pull -q busybox:stable >/dev/null

# Rust：工作流自己会 rustup toolchain install 固定版本，这里只需要 rustup 本体在 PATH 上。
if [[ ! -x "$HOME/.cargo/bin/rustup" ]]; then
  curl --fail --silent --show-error --proto '=https' --tlsv1.2 https://sh.rustup.rs -o "$HOME/rustup-init.sh"
  sh "$HOME/rustup-init.sh" -y --no-modify-path --profile minimal --default-toolchain none >/dev/null
  rm -f "$HOME/rustup-init.sh"
fi
cat > "$dir/register.sh" <<'REG'
#!/usr/bin/env bash
# 注册令牌只从 stdin 读、不回显、不落盘：
#   gh api -X POST repos/<owner>/<repo>/actions/runners/registration-token --jq .token | ssh ghrunner@<本机> '~/runners/<实例>/register.sh <owner>/<repo>'
set -euo pipefail
[[ $# == 1 && "$1" =~ ^[A-Za-z0-9._-]+/[A-Za-z0-9._-]+$ ]] || { echo '用法：register.sh <owner>/<repo>（令牌走 stdin）' >&2; exit 2; }
cd "$(dirname "$0")"
IFS= read -r token
[[ -n "$token" ]] || { echo 'stdin 没有令牌。' >&2; exit 1; }
name=$(basename "$PWD")
./config.sh --unattended --url "https://github.com/$1" --token "$token" \
  --name "$(hostname)-$name" --labels "self-hosted,linux,x64,$name" --work _work --replace >/dev/null
unset token
echo "已注册：$(hostname)-$name → $1（标签 self-hosted,linux,x64,$name）"
REG
chmod 700 "$dir/register.sh"
echo "实例 $name 已就绪：runner v$version 已验签解包，钩子与 PATH 已写好。等待注册。"
