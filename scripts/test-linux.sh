#!/usr/bin/env bash
# 在 Linux 容器内跑与 CI 相同的检查（ADR-008：本机 Docker 作快速回路，CI 结果为准）。
# 路径安全负向测试依赖 Linux 语义（openat2 RESOLVE_BENEATH、magic link），macOS 上会被 cfg 跳过。
# --ulimit memlock=-1：T-050/T-059 的秘密驻留面加固在 Linux 上用 mlockall；容器默认上限不足时
# 加固会失败关闭（正确行为），composition 用例会集体红。
# 用法：scripts/test-linux.sh            # fmt + clippy + test
#       scripts/test-linux.sh cargo --locked test -p studiolo-vault-broker
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
IMAGE="${STUDIOLO_RUST_IMAGE:-rust:1.97.1-bookworm}"
CACHE="${STUDIOLO_CARGO_CACHE:-studiolo-cargo-cache}"
TARGET="${STUDIOLO_TARGET_VOLUME:-studiolo-target-linux}"
if [ "$#" -eq 0 ]; then
  set -- bash -euo pipefail -c '
    rustup component add rustfmt clippy
    # 大文件用例用 /usr/bin/time 在隔离子进程里量峰值 RSS；rust:*-bookworm 基础镜像
    # 不带 time 包，缺了它第一个 large 用例会以 NotFound panic，并毒化 blob::LARGE_IO
    # 这把共享锁，导致后续 12 个用例全报 PoisonError（实际只有 1 个真失败）。
    apt-get update -qq && apt-get install -y -qq time
    /usr/bin/time --version
    python3 --version
    cargo --locked fmt --all -- --check
    cargo --locked clippy --workspace --all-targets -- -D warnings
    cargo --locked test --workspace
    cargo --locked test -p studiolo-vault-broker --test linux_path_security
    cargo --locked build -p studiolo-local-server --features studiolo-auth/production
    cargo --locked build -p studiolo-auth-admin --features production
    python3 crates/auth-admin/tests/support/cli_process.py target/debug/studiolo-auth-admin
  '
fi
exec docker run --rm \
  --ulimit memlock=-1 \
  -v "$ROOT":/work -w /work \
  -v "$CACHE":/usr/local/cargo/registry \
  -v "$TARGET":/work/target \
  -e CARGO_TERM_COLOR=always \
  "$IMAGE" "$@"
