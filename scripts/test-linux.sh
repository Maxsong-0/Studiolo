#!/usr/bin/env bash
# 在 Linux 容器内跑与 CI 相同的检查（ADR-008：本机 Docker 作快速回路，CI 结果为准）。
# 路径安全负向测试依赖 Linux 语义（openat2 RESOLVE_BENEATH、magic link），macOS 上会被 cfg 跳过。
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
    cargo --locked fmt --all -- --check
    cargo --locked clippy --workspace --all-targets -- -D warnings
    cargo --locked test --workspace
    cargo --locked test -p studiolo-vault-broker --test linux_path_security
  '
fi
exec docker run --rm \
  -v "$ROOT":/work -w /work \
  -v "$CACHE":/usr/local/cargo/registry \
  -v "$TARGET":/work/target \
  -e CARGO_TERM_COLOR=always \
  "$IMAGE" "$@"
