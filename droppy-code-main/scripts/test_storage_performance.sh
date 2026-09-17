#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."
FIXTURES="$(mktemp -d /tmp/droppy-storage-tests.XXXXXX)"
trap 'rm -rf "$FIXTURES"' EXIT
DEVELOPER_DIR_PATH="$(xcode-select -p)"
FRAMEWORKS="$DEVELOPER_DIR_PATH/Platforms/MacOSX.platform/Developer/Library/Frameworks"
MACROS="$DEVELOPER_DIR_PATH/Toolchains/XcodeDefault.xctoolchain/usr/lib/swift/host/plugins/testing/libTestingMacros.dylib"
PRODUCTS="${DROPPY_TEST_PRODUCTS:-$PWD/build.noindex/dev/Build/Products/Debug}"
BINARY_DIR="$PRODUCTS/Droppy Code Dev.app/Contents/MacOS"
xcrun swiftc -swift-version 6 -parse-as-library -target arm64-apple-macos26.0 \
  -I "$PRODUCTS" -F "$FRAMEWORKS" -framework Testing \
  -load-plugin-library "$MACROS" \
  -Xlinker -rpath -Xlinker "$FRAMEWORKS" -Xlinker -rpath -Xlinker "$BINARY_DIR" \
  "$BINARY_DIR/Droppy Code Dev.debug.dylib" Tests/StoragePerformanceTests.swift \
  -o build.noindex/storage-performance-tests
build.noindex/storage-performance-tests --website-captures "$FIXTURES"
