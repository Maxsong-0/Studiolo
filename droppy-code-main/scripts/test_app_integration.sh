#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."
DEVELOPER_DIR_PATH="$(xcode-select -p)"
FRAMEWORKS="$DEVELOPER_DIR_PATH/Platforms/MacOSX.platform/Developer/Library/Frameworks"
MACROS="$DEVELOPER_DIR_PATH/Toolchains/XcodeDefault.xctoolchain/usr/lib/swift/host/plugins/testing/libTestingMacros.dylib"
PRODUCTS="${DROPPY_TEST_PRODUCTS:-$PWD/build.noindex/dev/Build/Products/Debug}"
BINARY_DIR="$PRODUCTS/Droppy Code Dev.app/Contents/MacOS"
xcrun swiftc -parse-as-library -target arm64-apple-macos26.0 \
  -I "$PRODUCTS" -F "$FRAMEWORKS" -framework Testing \
  -load-plugin-library "$MACROS" \
  -Xlinker -rpath -Xlinker "$FRAMEWORKS" -Xlinker -rpath -Xlinker "$BINARY_DIR" \
  "$BINARY_DIR/Droppy Code Dev.debug.dylib" Tests/AppIntegrationTests.swift Tests/RuntimeRegressionTests.swift Tests/CodexRevertTests.swift Tests/TextPerformanceTests.swift \
  -o build.noindex/app-integration-tests
TEST_STORAGE="$(mktemp -d /tmp/droppy-integration.XXXXXX)"
trap 'rm -rf "$TEST_STORAGE"' EXIT
build.noindex/app-integration-tests --website-captures "$TEST_STORAGE"
