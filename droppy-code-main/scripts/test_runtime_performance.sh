#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."
DEVELOPER_DIR_PATH="$(xcode-select -p)"
FRAMEWORKS="$DEVELOPER_DIR_PATH/Platforms/MacOSX.platform/Developer/Library/Frameworks"
MACROS="$DEVELOPER_DIR_PATH/Toolchains/XcodeDefault.xctoolchain/usr/lib/swift/host/plugins/testing/libTestingMacros.dylib"
PRODUCTS="${DROPPY_TEST_PRODUCTS:-$PWD/build.noindex/dev/Build/Products/Debug}"
BINARY_DIR="$PRODUCTS/Droppy Code Dev.app/Contents/MacOS"
TEMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TEMP_DIR"' EXIT
for file in DroppyCode/Runtime/{ThreadRuntime,AutoContinue}.swift DroppyCode/Providers/{DeepSeekSession,MetaSession,NativeFileTools}.swift DroppyCode/Support/Shell.swift; do
    { echo '@testable import DroppyCode'; cat "$file"; } > "$TEMP_DIR/$(basename "$file")"
done
xcrun swiftc -parse-as-library -whole-module-optimization -target arm64-apple-macos26.0 -swift-version 6 \
  -Xfrontend -disable-access-control -I "$PRODUCTS" -F "$FRAMEWORKS" -framework Testing \
  -load-plugin-library "$MACROS" \
  -Xlinker -rpath -Xlinker "$FRAMEWORKS" -Xlinker -rpath -Xlinker "$BINARY_DIR" \
  "$BINARY_DIR/Droppy Code Dev.debug.dylib" "$TEMP_DIR"/*.swift Tests/RuntimePerformanceTests.swift \
  -o "$TEMP_DIR/runtime-performance-tests"
"$TEMP_DIR/runtime-performance-tests" --website-captures "$TEMP_DIR/captures"
