#!/usr/bin/env bash
# 单次捕获与推送；--dry-run 完全在本地检查，launchd 可每 60 秒运行。
set -euo pipefail
export PYTHONDONTWRITEBYTECODE=1
STUDIOLO_SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
exec "${STUDIOLO_PYTHON:-python3}" "$STUDIOLO_SCRIPT_DIR/replica_client.py" "$@"
