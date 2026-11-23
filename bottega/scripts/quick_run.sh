#!/bin/bash
# Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
# Dev loop for this build: regenerate the project, build Debug, install it as the
# single /Applications copy, and relaunch it.
#
#   scripts/quick_run.sh
#
# A Debug build is "Bottega Dev" (bundle id me.studiolo.bottega.dev). It
# runs beside BOTH the upstream Droppy Code installed from getdroppycode.app and
# this build's own Release, each with its own library, settings, Keychain items
# and worktrees: storage is keyed on AppInfo.storageName ("Bottega Dev" for Debug,
# "Bottega" for Release) and AppInfo.stateDirectoryName (~/.bottega-dev for Debug),
# never on the display name.
#
# xcode-select on this Mac points at the Command Line Tools, so DEVELOPER_DIR is set
# per-invocation (found below) rather than switching the machine's toolchain.
#
# Builds under build.noindex (never ~/Library/Developer/Xcode/DerivedData), because
# the ".noindex" suffix keeps Spotlight and Launchpad from listing the build folder
# as a second copy of the app. There must only ever be one dev app:
# /Applications/Bottega Dev.app.
#
# Safe to run from an agent inside the app itself: quitting the app may interrupt
# the calling session, but this script keeps going and the fresh build picks up the
# new changes on relaunch.
set -euo pipefail

cd "$(dirname "$0")/.."
# Xcode is found rather than named: the beta gets renamed to Xcode.app when it goes
# final, and a hard-coded path broke the build the day that happened. An explicit
# DEVELOPER_DIR still wins.
if [ -z "${DEVELOPER_DIR:-}" ]; then
  for candidate in /Applications/Xcode.app /Applications/Xcode-beta.app /Applications/Xcode*.app; do
    if [ -d "$candidate/Contents/Developer" ]; then export DEVELOPER_DIR="$candidate/Contents/Developer"; break; fi
  done
fi
[ -n "${DEVELOPER_DIR:-}" ] || { echo "No Xcode found under /Applications; set DEVELOPER_DIR." >&2; exit 1; }
APP_NAME="Bottega Dev"
BUNDLE_ID="me.studiolo.bottega.dev"
DERIVED="build.noindex/dev"
PRODUCT="$DERIVED/Build/Products/Debug/$APP_NAME.app"
TARGET="/Applications/$APP_NAME.app"
LOG="build.noindex/quick_run.log"

step() { printf '\n==> %s\n' "$1"; }

step "Regenerating the project from project.yml"
# The project is generated, not edited: a source file added since the last run is
# invisible to xcodebuild until this has run. A failure here (a resource listed in
# project.yml that is gone from disk, say) used to end the script with two quiet
# validation lines and the old app still running; say so loudly instead.
if ! xcodegen generate; then
  echo
  echo "xcodegen failed: the project was NOT regenerated, nothing was built or installed,"
  echo "and the app that is running is still the previous build. Fix the errors above."
  exit 1
fi

step "Building $APP_NAME (Debug)"
# Full output goes to a log and only the tail is shown; on failure the log is
# grepped for the actual errors, which a bare `tail` would scroll past.
if ! xcodebuild \
  -project Bottega.xcodeproj \
  -scheme Bottega \
  -configuration Debug \
  -destination 'platform=macOS' \
  -derivedDataPath "$DERIVED" \
  -clonedSourcePackagesDirPath build.noindex/spm \
  -skipPackagePluginValidation \
  -skipMacroValidation \
  build > "$LOG" 2>&1; then
    echo "Build failed. Errors:"
    grep "error:" "$LOG" | sort -u | head -n 20
    echo "(full log: $LOG)"
    exit 1
fi
tail -n 3 "$LOG"

step "Installing the single copy to $TARGET"
# Atomic swap: move running bundle aside so ditto installs the fresh build immediately
rm -rf "$TARGET.old"
mv "$TARGET" "$TARGET.old" 2>/dev/null || true
ditto "$PRODUCT" "$TARGET"
rm -rf "$TARGET.old" 2>/dev/null || true

# Keep the signature the build produced (the self-signed "AI Quota Flow Signing"
# certificate). macOS ties granted TCC permissions to the app's signed identity,
# and a certificate's designated requirement stays the same across builds, so the
# approvals survive. Re-signing ad-hoc here would make the requirement a cdhash,
# which changes on every build and makes macOS forget every approval.
if codesign --verify --deep --strict "$TARGET" 2>/dev/null; then
  # macOS 27's xattr dropped -r, and a locally built bundle carries no quarantine flag
  # anyway; clear it per file only when something actually set one.
  if xattr -l "$TARGET" 2>/dev/null | grep -q com.apple.quarantine; then
    find "$TARGET" -exec xattr -d com.apple.quarantine {} + 2>/dev/null || true
  fi
else
  codesign --force --deep --sign - "$TARGET"
fi

step "Relaunching"
# The running copy is found by its executable path and told to quit by bundle id, never
# by name: `tell application "Bottega Dev"` once resolved to the build product
# beside the installed copy, the installed app never got the quit, the wait ran its five
# minutes, and `open` then fronted the stale instance as if it had relaunched.
EXEC="$TARGET/Contents/MacOS/$APP_NAME"
running() { pgrep -f "^$EXEC" || true; }
OLD_PID="$(running | head -n 1)"
if [ -n "$OLD_PID" ]; then
  echo "quitting pid $OLD_PID"
  osascript -e "tell application id \"$BUNDLE_ID\" to quit" >/dev/null 2>&1 || true
  # A polite quit first, then TERM, then KILL. The app holds a quit while a Crew merge
  # is going out, so do not run this mid-merge: the escalation would cut it short.
  for i in $(seq 1 60); do
    [ -z "$(running)" ] && break
    if [ "$i" -eq 30 ]; then echo "still running after 30s, sending TERM"; kill -TERM "$OLD_PID" 2>/dev/null || true; fi
    if [ "$i" -eq 45 ]; then echo "still running after 45s, sending KILL"; kill -KILL "$OLD_PID" 2>/dev/null || true; fi
    sleep 1
  done
  if [ -n "$(running)" ]; then echo "ERROR: $APP_NAME (pid $OLD_PID) would not quit" >&2; exit 1; fi
fi
open "$TARGET"
# Loud when the relaunch did not happen: no process, or still the one we asked to quit.
NEW_PID=""
for _ in $(seq 1 20); do
  NEW_PID="$(running | head -n 1)"
  [ -n "$NEW_PID" ] && break
  sleep 0.5
done
if [ -z "$NEW_PID" ]; then echo "ERROR: $APP_NAME did not start" >&2; exit 1; fi
if [ -n "$OLD_PID" ] && [ "$NEW_PID" = "$OLD_PID" ]; then echo "ERROR: still the old process (pid $OLD_PID)" >&2; exit 1; fi
echo "running: pid $NEW_PID, started $(ps -o lstart= -p "$NEW_PID" | sed 's/^ *//')"
