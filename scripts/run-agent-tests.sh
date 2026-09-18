#!/usr/bin/env bash
# 在发布包记录的真实源码目录运行 Linux x86_64 测试；需要系统 Python 3，不需要 Rust、Node 或 Docker。
# 用法：cd /home/maxsong/.local/share/studiolo-agent-releases/<发布名>/source
#       bash ../run-agent-tests.sh
set -euo pipefail
export PATH=/usr/sbin:/usr/bin:/sbin:/bin
export PYTHONDONTWRITEBYTECODE=1 PYTHONNOUSERSITE=1
unset PYTHONPATH PYTHONHOME
STUDIOLO_PAYLOAD="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
if [[ $# != 0 || "$EUID" == 0 || "$(uname -s)" != Linux || "$(uname -m)" != x86_64 ]]; then
  printf '%s\n' '只支持由普通用户在原生 Linux x86_64 服务器上无参数运行。' >&2
  exit 2
fi
if [[ ! -f "$STUDIOLO_PAYLOAD/expected-source-root" || ! -s "$STUDIOLO_PAYLOAD/test-list" ]]; then
  printf '%s\n' '运行包缺少源码位置或测试清单。' >&2
  exit 1
fi
STUDIOLO_EXPECTED="$(< "$STUDIOLO_PAYLOAD/expected-source-root")"
if [[ ! "$STUDIOLO_EXPECTED" =~ ^/home/maxsong/\.local/share/studiolo-agent-releases/[A-Za-z0-9][A-Za-z0-9._-]{0,79}/source$ \
  || ! -d "$STUDIOLO_EXPECTED" || -L "$STUDIOLO_EXPECTED" \
  || "$(pwd -P)" != "$STUDIOLO_EXPECTED" || "$STUDIOLO_PAYLOAD" != "${STUDIOLO_EXPECTED%/source}" \
  || ! -f "$STUDIOLO_EXPECTED/Cargo.toml" || ! -d "$STUDIOLO_EXPECTED/crates" ]]; then
  printf '%s\n' '请先将 source.tar.gz 解压到记录的实体 source 目录，并从该目录执行测试；不得绕过架构测试。' >&2
  exit 1
fi
# T-065：studiolo-agent 与本包内的 process_residency 测试都会起真实进程跑 mlockall，
# RLIMIT_MEMLOCK 不足时它们按设计失败关闭并终止（不得改成尽力而为）。这里先拦一道，
# 免得整片红被误读成代码坏了；systemd 常见默认 8M，远不够长驻服务的常驻集。
STUDIOLO_MEMLOCK="$(ulimit -l)"
if [[ "$STUDIOLO_MEMLOCK" != unlimited && ( ! "$STUDIOLO_MEMLOCK" =~ ^[0-9]+$ || "$STUDIOLO_MEMLOCK" -le 65536 ) ]]; then
  printf '可锁定内存上限过低（ulimit -l = %s KB）：秘密驻留面加固会失败关闭，测试无法判定。\n' "$STUDIOLO_MEMLOCK" >&2
  printf '请由管理员抬高该上限后重跑，例如 sudo prlimit --pid $$ --memlock=unlimited:unlimited。\n' >&2
  exit 1
fi
# 校验的是整个发布包，不把尚未运行的测试误写为通过。
(cd "$STUDIOLO_PAYLOAD" && sha256sum --check SHA256SUMS)
if [[ ! -x /usr/bin/python3 || ! -s "$STUDIOLO_PAYLOAD/source-files" \
  || ! -s "$STUDIOLO_PAYLOAD/SOURCE_SHA256SUMS" \
  || ! -f "$STUDIOLO_PAYLOAD/bin/studiolo-agent" || -L "$STUDIOLO_PAYLOAD/bin/studiolo-agent" \
  || ! -x "$STUDIOLO_PAYLOAD/bin/studiolo-agent" ]]; then
  printf '%s\n' '缺少系统 Python 3、源码验签清单或发布二进制。' >&2
  exit 1
fi
# 标准库隔离模式校验源码集合；拒绝额外文件、符号链接及遗漏，防止验签后引用旁路源码。
/usr/bin/python3 -I -B - "$STUDIOLO_EXPECTED" "$STUDIOLO_PAYLOAD/source-files" <<'PY_SOURCE'
import os
import stat
import sys

root, manifest = sys.argv[1:]
try:
    with open(manifest, "rb") as stream:
        raw = stream.read()
    if not raw.endswith(b"\0"):
        raise ValueError()
    names = raw[:-1].decode("utf-8").split("\0")
    expected = set(names)
    if not names or len(expected) != len(names):
        raise ValueError()
    actual = set()
    for directory, directories, files in os.walk(root, followlinks=False):
        for name in directories + files:
            entry = os.path.join(directory, name)
            mode = os.lstat(entry).st_mode
            if name in directories:
                if not stat.S_ISDIR(mode):
                    raise ValueError()
            elif stat.S_ISREG(mode):
                actual.add(os.path.relpath(entry, root))
            else:
                raise ValueError()
    if actual != expected:
        raise ValueError()
except (OSError, UnicodeError, ValueError):
    sys.exit("解压源码与发布清单不一致，拒绝运行测试。")
PY_SOURCE
sha256sum --check --status "$STUDIOLO_PAYLOAD/SOURCE_SHA256SUMS"
printf '%s\n' '解压源码集合与逐文件 SHA256 校验通过。'
# 类型 token 的 TTY 门禁契约要在真实 PTY 下驱动实际产品二进制。运行包里二进制在 bin/ 下，
# 而测试程序内置的回退路径是构建机的 target/release/，在服务器源码目录下不存在——
# 不显式给出绝对路径，该契约会以「找不到二进制」失败（设计如此，不允许据此跳过）。
export STUDIOLO_TEST_AGENT_BIN="$STUDIOLO_PAYLOAD/bin/studiolo-agent"
export STUDIOLO_TEST_AGENT_TERMINAL_SCRIPT="$STUDIOLO_EXPECTED/crates/agent-server/tests/support/admin_terminal.py"
STUDIOLO_TEST_COUNT=0
declare -A STUDIOLO_SEEN_TESTS=()
while IFS= read -r STUDIOLO_TEST || [[ -n "$STUDIOLO_TEST" ]]; do
  if [[ ! "$STUDIOLO_TEST" =~ ^tests/[A-Za-z0-9][A-Za-z0-9._-]*$ \
    || ! -f "$STUDIOLO_PAYLOAD/$STUDIOLO_TEST" || ! -x "$STUDIOLO_PAYLOAD/$STUDIOLO_TEST" \
    || -L "$STUDIOLO_PAYLOAD/$STUDIOLO_TEST" || -n "${STUDIOLO_SEEN_TESTS[$STUDIOLO_TEST]:-}" ]]; then
    printf '%s\n' '测试清单包含非法、缺失或重复的可执行程序。' >&2
    exit 1
  fi
  printf '原生执行：%s\n' "$STUDIOLO_TEST"
  "$STUDIOLO_PAYLOAD/$STUDIOLO_TEST" --test-threads=1
  STUDIOLO_SEEN_TESTS[$STUDIOLO_TEST]=1
  STUDIOLO_TEST_COUNT=$((STUDIOLO_TEST_COUNT + 1))
done < "$STUDIOLO_PAYLOAD/test-list"
if [[ "$STUDIOLO_TEST_COUNT" == 0 ]]; then
  printf '%s\n' '没有执行测试，不能判定通过。' >&2
  exit 1
fi
printf '原生测试通过：%s 个 Rust 测试程序全部成功。\n' "$STUDIOLO_TEST_COUNT"
# 单元与端到端管线均使用虚构资料、隔离身份库和随机回环端口，不读取线上资料或凭据。
STUDIOLO_PYTHON_TESTS=(
  test-check-agent.py test-check-agent-replica.py test-check-agent-http.py
  test-sync-agent-vault.py test-prepare-agent-sync.py test-replica-receiver.py test-replica-boundaries.py
  test-client-projection-schema.py test-client-projection-fetch.py test-client-projection-store.py
)
for STUDIOLO_TEST in "${STUDIOLO_PYTHON_TESTS[@]}"; do
  printf '原生 Python 契约：%s\n' "$STUDIOLO_TEST"
  /usr/bin/python3 -B -s "$STUDIOLO_EXPECTED/scripts/$STUDIOLO_TEST"
done
for STUDIOLO_TEST in check-replica-pipeline.py check-client-projection-pipeline.py; do
  printf '原生真实管线：%s\n' "$STUDIOLO_TEST"
  /usr/bin/python3 -B -s "$STUDIOLO_EXPECTED/scripts/$STUDIOLO_TEST" "$STUDIOLO_PAYLOAD/bin/studiolo-agent"
done
printf '发布验收通过：%s 个 Rust 程序、%s 组 Python 契约、2 条虚构资料真实管线。\n' \
  "$STUDIOLO_TEST_COUNT" "${#STUDIOLO_PYTHON_TESTS[@]}"
