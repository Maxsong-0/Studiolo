#!/usr/bin/env bash
# 在发布包记录的真实源码目录运行 Linux x86_64 测试；需要 Python3，不需要 Rust、Node 或 Docker。
# 用法：cd /home/maxsong/.local/share/studiolo-preview/releases/<发布名>/source
#       bash ../run-preview-tests.sh
set -euo pipefail
STUDIOLO_PAYLOAD="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
if [[ $# != 0 || "$(uname -s)" != Linux || "$(uname -m)" != x86_64 ]]; then
  printf '%s\n' '只支持在原生 Linux x86_64 服务器上无参数运行。' >&2
  exit 2
fi
if [[ ! -f "$STUDIOLO_PAYLOAD/expected-source-root" || ! -s "$STUDIOLO_PAYLOAD/test-list" ]]; then
  printf '%s\n' '运行包缺少源码位置或测试清单。' >&2
  exit 1
fi
STUDIOLO_EXPECTED="$(< "$STUDIOLO_PAYLOAD/expected-source-root")"
if [[ ! "$STUDIOLO_EXPECTED" =~ ^/home/maxsong/\.local/share/studiolo-preview/releases/[A-Za-z0-9][A-Za-z0-9._-]{0,79}/source$ \
  || ! -d "$STUDIOLO_EXPECTED" || -L "$STUDIOLO_EXPECTED" \
  || "$(pwd -P)" != "$STUDIOLO_EXPECTED" || "$STUDIOLO_PAYLOAD" != "${STUDIOLO_EXPECTED%/source}" \
  || ! -f "$STUDIOLO_EXPECTED/Cargo.toml" || ! -d "$STUDIOLO_EXPECTED/crates" ]]; then
  printf '%s\n' '请先将 source.tar.gz 解压到记录的实体 source 目录，并从该目录执行测试；不得绕过架构测试。' >&2
  exit 1
fi
# 校验的是整个发布包，不把尚未运行的测试误写为通过。
(cd "$STUDIOLO_PAYLOAD" && sha256sum --check SHA256SUMS)
# 进程契约使用随包校验的生产程序，不依赖构建机的 /build-target 绝对路径。
for STUDIOLO_BINARY in studiolo-local-server studiolo-auth-admin; do
  if [[ ! -f "$STUDIOLO_PAYLOAD/bin/$STUDIOLO_BINARY" \
    || ! -x "$STUDIOLO_PAYLOAD/bin/$STUDIOLO_BINARY" \
    || -L "$STUDIOLO_PAYLOAD/bin/$STUDIOLO_BINARY" ]]; then
    printf '%s\n' '运行包缺少可执行的生产产品程序。' >&2
    exit 1
  fi
done
export STUDIOLO_TEST_SERVER_BIN="$STUDIOLO_PAYLOAD/bin/studiolo-local-server"
export STUDIOLO_TEST_AUTH_ADMIN_BIN="$STUDIOLO_PAYLOAD/bin/studiolo-auth-admin"
python3 --version
STUDIOLO_TEST_COUNT=0
while IFS= read -r STUDIOLO_TEST || [[ -n "$STUDIOLO_TEST" ]]; do
  if [[ ! "$STUDIOLO_TEST" =~ ^tests/[A-Za-z0-9][A-Za-z0-9._-]*$ \
    || ! -f "$STUDIOLO_PAYLOAD/$STUDIOLO_TEST" || ! -x "$STUDIOLO_PAYLOAD/$STUDIOLO_TEST" \
    || -L "$STUDIOLO_PAYLOAD/$STUDIOLO_TEST" ]]; then
    printf '%s\n' '测试清单包含非法或缺失的可执行程序。' >&2
    exit 1
  fi
  printf '原生执行：%s\n' "$STUDIOLO_TEST"
  "$STUDIOLO_PAYLOAD/$STUDIOLO_TEST" --test-threads=1
  STUDIOLO_TEST_COUNT=$((STUDIOLO_TEST_COUNT + 1))
done < "$STUDIOLO_PAYLOAD/test-list"
if [[ "$STUDIOLO_TEST_COUNT" == 0 ]]; then
  printf '%s\n' '没有执行测试，不能判定通过。' >&2
  exit 1
fi
printf '原生测试通过：%s 个 Rust 测试程序全部成功。\n' "$STUDIOLO_TEST_COUNT"
