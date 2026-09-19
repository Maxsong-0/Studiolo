#!/usr/bin/env bash
# 在 Linux x86_64 服务器上**原生**跑与 CI 相同的检查（ADR-008：本机快速回路，CI 结果为准）。
# 2026-09-19（T-081）起不再用 Docker：本机 Docker 回路两次把守护进程拖死、一次把磁盘写满，
# 而 amd64 仿真让一轮构建要几小时。现在把工作树同步到 CI 服务器（githubci，原生 x86_64）后在那里跑，
# 与 CI 用同一台机器、同一套工具链，只是不占 runner 的队列。
#
# 路径安全负向测试依赖 Linux 语义（openat2 RESOLVE_BENEATH、magic link），macOS 上会被 cfg 跳过。
# 秘密驻留面加固在 Linux 上用 mlockall；RLIMIT_MEMLOCK 不足时它按设计失败关闭并终止进程
# （不得改成尽力而为），因此服务器上必须先由 deploy/ci-runner/bootstrap-root.sh 抬高该上限。
# CARGO_PROFILE_DEV_DEBUG=line-tables-only：与 ci.yml 同一设置，只丢完整调试信息，保留 panic 回溯的文件与行号。
# umask 022：捕获端与接收器按设计拒绝组可写节点，而部分测试夹具按默认权限建目录（T-079）。
#
# 用法：scripts/test-linux.sh            # fmt + clippy + test + 路径安全 + 生产构建
#       scripts/test-linux.sh cargo --locked test -p studiolo-vault-broker
# 环境：STUDIOLO_TEST_HOST 覆盖目标主机（默认 maxsong@192.168.100.107）。
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
HOST="${STUDIOLO_TEST_HOST:-maxsong@192.168.100.107}"
# 每个工作树一个远端目录：并发会话互不覆盖，各自的构建缓存也不互相作废。
KEY="$(printf '%s' "$ROOT" | shasum -a 256 | cut -c1-12)"
NAME="$(basename "$ROOT")-$KEY"
REMOTE=".cache/studiolo-tests/$NAME"

if [ "$#" -eq 0 ]; then
  RUN='
    cargo --locked fmt --all -- --check
    cargo --locked clippy --workspace --all-targets -- -D warnings
    cargo --locked test --workspace
    cargo --locked test -p studiolo-vault-broker --test linux_path_security
    cargo --locked build -p studiolo-local-server --features studiolo-auth/production
    cargo --locked build -p studiolo-auth-admin --features production
    python3 crates/auth-admin/tests/support/cli_process.py "$CARGO_TARGET_DIR/debug/studiolo-auth-admin"
  '
else
  # ssh 把多个参数拼成一条远端命令再由远端 shell 重新分词，所以这里先逐个加引号。
  RUN="$(printf '%q ' "$@")"
fi

# rsync 不会建多级父目录，先建好（-p 幂等）。
ssh -o BatchMode=yes -o ConnectTimeout=15 "$HOST" "mkdir -p '$REMOTE'"

# 只同步已跟踪文件加未跟踪且未忽略的改动：构建产物、node_modules、.git 一律不过网。
rsync -a --delete --delete-excluded \
  --exclude '.git/' --exclude 'target/' --exclude 'node_modules/' --exclude 'web/dist/' \
  --exclude '.claude/' --exclude '*.log' \
  -e 'ssh -o BatchMode=yes -o ConnectTimeout=15' \
  "$ROOT"/ "$HOST:$REMOTE/"

# flock：同一工作树的两次调用排队，不并发糟蹋同一个构建目录。
# 构建目录放在同步树之外，--delete 不会清掉它。
# 命令经 %q 整体转成一个安全的 token 传给远端 shell；远端只 bash -c 一次，不做二次求值。
exec ssh -o BatchMode=yes -o ConnectTimeout=15 "$HOST" \
  "STUDIOLO_REMOTE=$(printf '%q' "$REMOTE") STUDIOLO_NAME=$(printf '%q' "$NAME") \
   STUDIOLO_DEV_DEBUG=$(printf '%q' "${STUDIOLO_DEV_DEBUG:-line-tables-only}") \
   STUDIOLO_RUN=$(printf '%q' "$RUN") bash -euo pipefail -s" <<'REMOTE_SCRIPT'
umask 022
export PATH="$HOME/.cargo/bin:$PATH"
export CARGO_TERM_COLOR=always
export CARGO_PROFILE_DEV_DEBUG="$STUDIOLO_DEV_DEBUG"
export CARGO_TARGET_DIR="$HOME/.cache/studiolo-target/$STUDIOLO_NAME"
mkdir -p "$CARGO_TARGET_DIR" "$HOME/.cache/studiolo-locks"
limit="$(ulimit -l)"
if [ "$limit" != unlimited ] && { ! [ "$limit" -gt 65536 ] 2>/dev/null; }; then
  printf '可锁定内存上限过低（ulimit -l = %s KB）：秘密驻留面加固会失败关闭，测试无法判定。\n' "$limit" >&2
  printf '请在服务器上执行一次 sudo bash ~/ci-runner/bootstrap-root.sh 抬高该上限后重试。\n' >&2
  exit 1
fi
command -v rustup >/dev/null || { printf '服务器上没有 rustup；先 curl https://sh.rustup.rs | sh -s -- -y --profile minimal\n' >&2; exit 1; }
cd "$STUDIOLO_REMOTE"
exec flock "$HOME/.cache/studiolo-locks/$STUDIOLO_NAME.lock" bash -euo pipefail -c "$STUDIOLO_RUN"
REMOTE_SCRIPT
