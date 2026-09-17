#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."
DEVELOPER_DIR_PATH="$(xcode-select -p)"
FRAMEWORKS="$DEVELOPER_DIR_PATH/Platforms/MacOSX.platform/Developer/Library/Frameworks"
MACROS="$DEVELOPER_DIR_PATH/Toolchains/XcodeDefault.xctoolchain/usr/lib/swift/host/plugins/testing/libTestingMacros.dylib"
mkdir -p build.noindex
xcrun swiftc -parse-as-library -F "$FRAMEWORKS" -framework Testing \
  -load-plugin-library "$MACROS" -Xlinker -rpath -Xlinker "$FRAMEWORKS" \
  DroppyCode/Support/RecentCache.swift DroppyCode/Support/ThumbnailCache.swift \
  DroppyCode/Models/Provider.swift DroppyCode/Support/JSONValue.swift \
  DroppyCode/Git/RevertPreview.swift \
  Tests/ComposerRegressionTests.swift -o build.noindex/composer-tests
build.noindex/composer-tests
