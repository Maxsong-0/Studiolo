#!/bin/bash
# Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
# One-off import of an installed upstream Droppy Code's data into this build.
#
#   scripts/import_from_upstream.sh            # dry run: prints what it would do
#   scripts/import_from_upstream.sh --apply    # actually does it
#   scripts/import_from_upstream.sh --apply --release   # target the Release build
#
# This build keeps upstream's display name but stores everything under its own
# names (AppInfo.storageName / stateDirectoryName / the bundle id), so it starts
# with an empty library. This copies the upstream library over once.
#
# What it copies:
#   ~/Library/Application Support/Droppy Code/  → this build's support folder
#       (library.json, threads, attachments, mcp)
#   defaults domain iordv.droppycode            → this build's domain
#       (settings: theme, shortcuts, provider toggles, panel sizes)
#
# What it does NOT copy, on purpose:
#   Keychain items. Provider accounts are credentials; re-sign in from the app
#   rather than having a script move secrets around. Most providers keep their
#   session in their own CLI anyway (codex login, claude auth login, …), which
#   both builds already share.
#   ~/.droppy-code (worktrees and checkpoints). Those are throwaway state tied to
#   threads that are mid-flight; importing them would point this build at
#   worktrees the other app still owns.
#
# The upstream app is left completely untouched: this only ever reads from it.
set -euo pipefail

APPLY=0
VARIANT="dev"
for arg in "$@"; do
  case "$arg" in
    --apply) APPLY=1 ;;
    --release) VARIANT="release" ;;
    --dev) VARIANT="dev" ;;
    *) echo "unknown argument: $arg" >&2; exit 2 ;;
  esac
done

SRC_SUPPORT="$HOME/Library/Application Support/Droppy Code"
SRC_DOMAIN="iordv.droppycode"

if [ "$VARIANT" = "release" ]; then
  DST_SUPPORT="$HOME/Library/Application Support/Bottega"
  DST_DOMAIN="me.studiolo.bottega"
  APP_NAME="Bottega"
else
  DST_SUPPORT="$HOME/Library/Application Support/Bottega Dev"
  DST_DOMAIN="me.studiolo.bottega.dev"
  APP_NAME="Bottega Dev"
fi

say() { printf '%s\n' "$1"; }
run() { if [ "$APPLY" = "1" ]; then eval "$1"; else say "  would run: $1"; fi; }

say "Source:      $SRC_SUPPORT  (defaults: $SRC_DOMAIN)"
say "Destination: $DST_SUPPORT  (defaults: $DST_DOMAIN)"
say ""

if [ ! -d "$SRC_SUPPORT" ]; then
  say "No upstream data at $SRC_SUPPORT — nothing to import."
  exit 0
fi

# The target app must be closed: it holds the library in memory and writes it back
# on change, which would overwrite whatever this copies in.
if ps aux | grep -F "$APP_NAME.app/Contents/MacOS" | grep -qv grep; then
  say "ERROR: $APP_NAME is running. Quit it first — it would write its in-memory"
  say "       library back over the imported one."
  exit 1
fi

if [ "$APPLY" = "0" ]; then
  say "DRY RUN. Nothing is changed. Re-run with --apply to do it."
  say ""
fi

# Existing destination data is moved aside rather than deleted, so a bad import
# is one `mv` away from being undone.
if [ -d "$DST_SUPPORT" ]; then
  STAMP=$(date +%Y%m%d-%H%M%S)
  say "Destination already has data; moving it aside first:"
  run "mv \"$DST_SUPPORT\" \"$DST_SUPPORT.before-import-$STAMP\""
  say ""
fi

say "Copying the library:"
run "ditto \"$SRC_SUPPORT\" \"$DST_SUPPORT\""
say ""

say "Copying settings:"
if [ "$APPLY" = "1" ]; then
  # Round-tripping through a plist file keeps every type intact; piping `defaults
  # export` into `defaults import` directly drops on some keys.
  TMP=$(mktemp -t droppy-import)
  defaults export "$SRC_DOMAIN" "$TMP"
  defaults import "$DST_DOMAIN" "$TMP"
  rm -f "$TMP"
  say "  imported $SRC_DOMAIN → $DST_DOMAIN"
else
  say "  would run: defaults export $SRC_DOMAIN <tmp> && defaults import $DST_DOMAIN <tmp>"
fi
say ""

if [ "$APPLY" = "1" ]; then
  say "Done. Open $APP_NAME and check the thread list."
  say "The upstream app was not modified."
else
  say "DRY RUN finished. Re-run with --apply to do it."
fi
