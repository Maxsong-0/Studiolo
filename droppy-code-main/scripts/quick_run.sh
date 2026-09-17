#!/bin/bash
# Dev loop for Droppy Code: build Debug, install it as the single
# /Applications copy of the dev app, and relaunch it.
#
#   scripts/quick_run.sh
#
# A Debug build is "Droppy Code Dev" (bundle id iordv.droppycode.dev): it runs
# beside the released Droppy Code with its own library, settings, keychain items
# and worktrees (~/.droppy-code-dev), and never touches the release app's. Before
# each relaunch scripts/sync_dev_data.sh mirrors the release app's library,
# settings and API keys into it, so Dev opens on the real data as of that moment;
# set DROPPY_DEV_SYNC=0 to skip the mirror.
#
# Builds under build.noindex (never ~/Library/Developer/Xcode/DerivedData),
# because the ".noindex" suffix keeps Spotlight and Launchpad from listing
# the build folder as a second copy of the app. There must only ever be one
# dev app: /Applications/Droppy Code Dev.app.
#
# Safe to run from an agent inside Droppy Code itself: quitting the app may
# interrupt the calling session, but this script keeps going and the fresh
# build picks up the new changes on relaunch.
set -euo pipefail

cd "$(dirname "$0")/.."
APP_NAME="Droppy Code Dev"
DERIVED="build.noindex/dev"
PRODUCT="$DERIVED/Build/Products/Debug/$APP_NAME.app"
TARGET="/Applications/$APP_NAME.app"

step() { printf '\n==> %s\n' "$1"; }

step "Building $APP_NAME (Debug)"
xcodebuild \
  -project DroppyCode.xcodeproj \
  -scheme DroppyCode \
  -configuration Debug \
  -destination 'platform=macOS' \
  -derivedDataPath "$DERIVED" \
  -skipPackagePluginValidation \
  build 2>&1 | tail -n 5

step "Installing the single copy to $TARGET"
# Atomic swap: move running bundle aside so ditto installs the fresh build immediately
rm -rf "$TARGET.old"
mv "$TARGET" "$TARGET.old" 2>/dev/null || true
ditto "$PRODUCT" "$TARGET"
rm -rf "$TARGET.old" 2>/dev/null || true

# Keep Xcode's Development signature (Team NARHG44L48). macOS TCC ties granted
# permissions to the app's signed identity, so re-signing ad-hoc here gave every
# build a new identity with no Team ID and macOS forgot all approvals on each
# run. Only fall back to ad-hoc when the fresh build has no valid signature.
if codesign --verify --deep --strict "$TARGET" 2>/dev/null; then
  xattr -dr com.apple.quarantine "$TARGET" 2>/dev/null || true
else
  codesign --force --deep --sign - "$TARGET"
fi

step "Relaunching"
osascript -e "tell application \"$APP_NAME\" to quit" 2>/dev/null || true
for _ in $(seq 1 20); do
  ps aux | grep -F "$APP_NAME.app/Contents/MacOS" | grep -v grep >/dev/null || break
  sleep 1
done
if [ "${DROPPY_DEV_SYNC:-1}" != "0" ]; then
  step "Mirroring the release app's data into $APP_NAME"
  scripts/sync_dev_data.sh
fi
open "$TARGET"
sleep 4
ps aux | grep -F "$APP_NAME.app/Contents/MacOS" | grep -v grep | head -n 3
