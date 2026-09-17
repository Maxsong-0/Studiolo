#!/bin/bash
# Mirror the released Droppy Code into Droppy Code Dev: copy its library,
# settings and API keys over, one way, never writing to the release side.
#
#   scripts/sync_dev_data.sh
#
# Dev's own changes since the last mirror are replaced, so it opens on the
# real data as of this moment. Head worktrees and patches under ~/.droppy-code
# are left alone; Dev keeps its own under ~/.droppy-code-dev.
#
# macOS may ask once per key to let `security` read the release item; choose
# Always Allow and it will not ask again.
set -euo pipefail

RELEASE_NAME="Droppy Code"
DEV_NAME="Droppy Code Dev"
RELEASE_LIB="$HOME/Library/Application Support/$RELEASE_NAME"
DEV_LIB="$HOME/Library/Application Support/$DEV_NAME"
RELEASE_DOMAIN=iordv.droppycode
DEV_DOMAIN=iordv.droppycode.dev
DEV_APP="/Applications/$DEV_NAME.app"

step() { printf '\n==> %s\n' "$1"; }

[ -d "$RELEASE_LIB" ] || { echo "No release library at $RELEASE_LIB; nothing to mirror."; exit 0; }

step "Quitting $DEV_NAME"
osascript -e "tell application \"$DEV_NAME\" to quit" 2>/dev/null || true
for _ in $(seq 1 20); do
  ps aux | grep -F "$DEV_NAME.app/Contents/MacOS" | grep -v grep >/dev/null || break
  sleep 1
done

step "Mirroring the library"
mkdir -p "$DEV_LIB"
rsync -a --delete "$RELEASE_LIB/" "$DEV_LIB/"
echo "$(find "$DEV_LIB/threads" -name '*.json' 2>/dev/null | wc -l | tr -d ' ') threads mirrored"

step "Mirroring settings"
if defaults read "$RELEASE_DOMAIN" >/dev/null 2>&1; then
  defaults delete "$DEV_DOMAIN" >/dev/null 2>&1 || true
  defaults export "$RELEASE_DOMAIN" - | defaults import "$DEV_DOMAIN" -
else
  echo "No release settings yet."
fi

step "Mirroring API keys"
keys=0
for account in "DeepSeek API Key" "Meta API Key" "Z.ai API Key" "Command Code API Key"; do
  secret=$(security find-generic-password -s "$RELEASE_NAME" -a "$account" -w 2>/dev/null) || continue
  [ -n "$secret" ] || continue
  security add-generic-password -U -s "$DEV_NAME" -a "$account" -w "$secret" -T "$DEV_APP" -T /usr/bin/security >/dev/null
  echo "  $account"
  keys=$((keys + 1))
done
if [ "$keys" -gt 0 ]; then
  echo "$keys keys mirrored"
else
  echo "No API keys in the release keychain."
fi

echo
echo "$DEV_NAME now mirrors $RELEASE_NAME. Worktrees stay separate (~/.droppy-code vs ~/.droppy-code-dev)."
