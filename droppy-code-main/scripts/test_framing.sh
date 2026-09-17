#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."
DEVELOPER_DIR_PATH="$(xcode-select -p)"
FRAMEWORKS="$DEVELOPER_DIR_PATH/Platforms/MacOSX.platform/Developer/Library/Frameworks"
MACROS="$DEVELOPER_DIR_PATH/Toolchains/XcodeDefault.xctoolchain/usr/lib/swift/host/plugins/testing/libTestingMacros.dylib"
mkdir -p build.noindex
xcrun swiftc -swift-version 6 -O -parse-as-library -target arm64-apple-macos26.0 \
  -F "$FRAMEWORKS" -framework Testing -load-plugin-library "$MACROS" \
  -Xlinker -rpath -Xlinker "$FRAMEWORKS" \
  DroppyCode/Support/StdioFramer.swift Tests/FramingRegressionTests.swift \
  -o build.noindex/framing-tests
build.noindex/framing-tests
