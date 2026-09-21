#!/usr/bin/env bash
# 以 ghrunner 身份（无特权）为一个仓库准备 runner 实例：下载并验签官方 runner、装 Rust 工具链、写作业钩子。
# 用法：bash install-runner.sh <实例名> <runner 版本> <该版本 linux-x64 包的 sha256> [<标签实例名>]
# 版本与 sha256 取自 https://github.com/actions/runner/releases 的发布说明；不验签不解包。
# 第 4 个参数只在**同一台机器给同一个仓库多开实例**时给：实例名必须唯一（它决定 runner 名 <hostname>-<实例名>），
# 但标签要和第一个实例相同，作业才会派进同一个池。省略时标签沿用实例名，与原行为一致。
set -Eeuo pipefail
umask 077
[[ "$(id -un)" == ghrunner && ( $# == 3 || $# == 4 ) && "$1" =~ ^[a-z0-9][a-z0-9-]{0,39}$ && "$2" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ && "$3" =~ ^[0-9a-f]{64}$ && ( $# == 3 || "$4" =~ ^[a-z0-9][a-z0-9-]{0,39}$ ) ]] \
  || { echo '须以 ghrunner 执行：install-runner.sh <实例名> <版本> <sha256> [<标签实例名>]' >&2; exit 2; }
name=$1 version=$2 sha=$3 label=${4:-$1}
dir=$HOME/runners/$name
[[ ! -e "$dir/.runner" ]] || { echo "实例 $name 已注册，先注销再重装。" >&2; exit 1; }
mkdir -p "$dir" "$HOME/hooks"
tarball=$(mktemp)
trap 'rm -f "$tarball"' EXIT
curl --fail --silent --show-error --location --proto '=https' --tlsv1.2 \
  "https://github.com/actions/runner/releases/download/v$version/actions-runner-linux-x64-$version.tar.gz" -o "$tarball"
printf '%s  %s\n' "$sha" "$tarball" | sha256sum --check --status || { echo 'runner 包摘要不符，拒绝解包。' >&2; exit 1; }
tar -xzf "$tarball" -C "$dir"

# T-081 起工作流没有容器作业，工作区不会出现 root 属主文件，因此不再需要作业开始钩子。
# 构建目录常驻在工作区之外：checkout 每次都会清工作区，放里面等于每次全量重编。
mkdir -p "$HOME/cache/$name"
printf 'CARGO_TARGET_DIR=%s\n' "$HOME/cache/$name/target" > "$dir/.env"
# systemd 单元走官方服务入口 runsvc.sh（svc.sh 也是这样从 bin/ 拷出来用的）。
install -m 700 "$dir/bin/runsvc.sh" "$dir/runsvc.sh"

# Rust：工作流自己会 rustup toolchain install 固定版本，这里只需要 rustup 本体在 PATH 上。
if [[ ! -x "$HOME/.cargo/bin/rustup" ]]; then
  curl --fail --silent --show-error --proto '=https' --tlsv1.2 https://sh.rustup.rs -o "$HOME/rustup-init.sh"
  sh "$HOME/rustup-init.sh" -y --no-modify-path --profile minimal --default-toolchain none >/dev/null
  rm -f "$HOME/rustup-init.sh"
fi
{
printf '%s\n' '#!/usr/bin/env bash'
# 标签在装实例时就定死：注册时不再接受参数，免得同一个池里混进派不到活的标签。
printf 'STUDIOLO_RUNNER_LABEL=%q\n' "$label"
cat <<'REG'
# 注册令牌只从 stdin 读、不回显、不落盘：
#   gh api -X POST repos/<owner>/<repo>/actions/runners/registration-token --jq .token | ssh ghrunner@<本机> '~/runners/<实例>/register.sh <owner>/<repo>'
set -euo pipefail
[[ $# == 1 && "$1" =~ ^[A-Za-z0-9._-]+/[A-Za-z0-9._-]+$ ]] || { echo '用法：register.sh <owner>/<repo>（令牌走 stdin）' >&2; exit 2; }
cd "$(dirname "$0")"
IFS= read -r token
[[ -n "$token" ]] || { echo 'stdin 没有令牌。' >&2; exit 1; }
name=$(basename "$PWD")
label=${STUDIOLO_RUNNER_LABEL:?装实例时未写入标签，请重新运行 install-runner.sh}
./config.sh --unattended --url "https://github.com/$1" --token "$token" \
  --name "$(hostname)-$name" --labels "self-hosted,linux,x64,$label" --work _work --replace >/dev/null
unset token
# 作业的 PATH：只有 runsvc.sh 会读 .path；而 config.sh 里的 env.sh 会无条件 `echo $PATH>.path` 覆盖它，所以必须在注册**之后**写。
# （实测 2026-09-19：.env 里写 PATH 对作业步骤不生效。工作流自己另用 GITHUB_PATH 兜底。）
printf '%s\n' "$HOME/.cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin" > .path
echo "已注册：$(hostname)-$name → $1（标签 self-hosted,linux,x64,$label）"
REG
} > "$dir/register.sh"
chmod 700 "$dir/register.sh"
echo "实例 $name 已就绪：runner v$version 已验签解包，标签 self-hosted,linux,x64,$label，PATH 已写好。等待注册。"
