#!/bin/bash
# Builds, signs, notarizes and packages Droppy Code as a disk image.
#
#   scripts/release.sh
#
# Needs Xcode, XcodeGen, a Developer ID Application certificate for the team below
# and a notarytool keychain profile (NOTARY_PROFILE, "Droppy-Notarize" by default).
set -euo pipefail

cd "$(dirname "$0")/.."
ROOT="$PWD"
TEAM_ID="NARHG44L48"
NOTARY_PROFILE="${NOTARY_PROFILE:-Droppy-Notarize}"
APP_NAME="Droppy Code"
# ".noindex" keeps Spotlight and Launchpad from listing the build copies of the app.
BUILD="$ROOT/build.noindex"
ARCHIVE="$BUILD/DroppyCode.xcarchive"
EXPORT="$BUILD/export"
VERSION=$(sed -nE 's/^[[:space:]]*MARKETING_VERSION:[[:space:]]*"([^"]+)".*/\1/p' project.yml | head -1)
DMG="$BUILD/Droppy-Code-$VERSION.dmg"

step() { printf '\n==> %s\n' "$1"; }

notarize() {
  local file="$1" result status id
  result=$(xcrun notarytool submit "$file" --keychain-profile "$NOTARY_PROFILE" --wait --output-format json)
  status=$(printf '%s' "$result" | plutil -extract status raw - 2>/dev/null || true)
  if [ "$status" != "Accepted" ]; then
    printf '%s\n' "$result"
    id=$(printf '%s' "$result" | plutil -extract id raw - 2>/dev/null || true)
    [ -n "$id" ] && xcrun notarytool log "$id" --keychain-profile "$NOTARY_PROFILE"
    exit 1
  fi
  echo "Notarization accepted for $(basename "$file")"
}

step "Generating the Xcode project"
xcodegen generate --quiet

step "Archiving $APP_NAME $VERSION"
rm -rf "$BUILD"
mkdir -p "$BUILD"
if ! xcodebuild archive \
  -project DroppyCode.xcodeproj \
  -scheme DroppyCode \
  -configuration Release \
  -destination 'generic/platform=macOS' \
  -archivePath "$ARCHIVE" \
  -derivedDataPath "$BUILD/DerivedData" \
  -skipPackagePluginValidation \
  -skipMacroValidation \
  ARCHS=arm64 \
  EXCLUDED_ARCHS=x86_64 \
  ONLY_ACTIVE_ARCH=NO > "$BUILD/archive.log" 2>&1; then
  grep -E "error:" "$BUILD/archive.log" | head -40 || tail -40 "$BUILD/archive.log"
  exit 1
fi

step "Exporting with Developer ID"
cat > "$BUILD/ExportOptions.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>method</key>
  <string>developer-id</string>
  <key>teamID</key>
  <string>$TEAM_ID</string>
  <key>signingStyle</key>
  <string>manual</string>
  <key>signingCertificate</key>
  <string>Developer ID Application</string>
</dict>
</plist>
PLIST
if ! xcodebuild -exportArchive \
  -archivePath "$ARCHIVE" \
  -exportPath "$EXPORT" \
  -exportOptionsPlist "$BUILD/ExportOptions.plist" > "$BUILD/export.log" 2>&1; then
  tail -40 "$BUILD/export.log"
  exit 1
fi
APP="$EXPORT/$APP_NAME.app"

step "Verifying the signature"
codesign --verify --deep --strict --verbose=2 "$APP"
details=$(codesign -dv --verbose=4 "$APP" 2>&1)
grep -q "Authority=Developer ID Application" <<< "$details"
grep -q "flags=.*runtime" <<< "$details"
grep -q "Timestamp=" <<< "$details"
archs=$(lipo -archs "$APP/Contents/MacOS/$APP_NAME")
if [ "$archs" != "arm64" ]; then
  echo "Expected an Apple silicon binary, got: $archs"
  exit 1
fi

step "Notarizing the app"
ditto -c -k --keepParent "$APP" "$BUILD/DroppyCode.zip"
notarize "$BUILD/DroppyCode.zip"
xcrun stapler staple "$APP"

step "Building the disk image"
STAGING="$BUILD/dmg"
mkdir -p "$STAGING"
ditto "$APP" "$STAGING/$APP_NAME.app"
ln -s /Applications "$STAGING/Applications"
hdiutil create -volname "$APP_NAME" -srcfolder "$STAGING" -ov -format UDZO "$DMG" > /dev/null
IDENTITY=$(security find-identity -v -p codesigning | sed -nE "s/.*\"(Developer ID Application: .*\($TEAM_ID\))\".*/\1/p" | head -1)
codesign --sign "$IDENTITY" --timestamp "$DMG"

step "Notarizing the disk image"
notarize "$DMG"
xcrun stapler staple "$DMG"

step "Checking Gatekeeper"
spctl --assess --type execute --ignore-cache --no-cache --verbose "$APP"
spctl --assess --type open --context context:primary-signature --ignore-cache --no-cache --verbose "$DMG"
xcrun stapler validate "$DMG"

step "Cleaning up"
# Only the disk image stays, so Spotlight and Launchpad never list a second copy of the app.
rm -rf "$ARCHIVE" "$EXPORT" "$STAGING" "$BUILD/DroppyCode.zip" "$BUILD/DerivedData"

printf '\nReady: %s\nPublish it with scripts/publish_release.sh once ReleaseNotes/%s.md is written.\n' "$DMG" "$VERSION"
