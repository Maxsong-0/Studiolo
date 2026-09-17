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
  DroppyCode/Support/Shell.swift DroppyCode/Support/StdioProcess.swift \
  DroppyCode/Support/StdioFramer.swift DroppyCode/Support/JSONValue.swift \
  DroppyCode/Support/JSONRPC.swift Tests/ProcessPerformanceTests.swift \
  -o build.noindex/process-performance-tests
build.noindex/process-performance-tests
