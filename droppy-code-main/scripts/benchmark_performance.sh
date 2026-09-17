#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."
SOURCE_ROOT="${1:-$PWD}"
BENCHMARK_DIR="$(mktemp -d "${TMPDIR:-/tmp}/droppy-benchmark.XXXXXX")"
trap 'rm -rf "$BENCHMARK_DIR"' EXIT
python3 - "$SOURCE_ROOT" "$BENCHMARK_DIR" <<'PY'
import pathlib, sys
source = pathlib.Path(sys.argv[1]) / 'DroppyCode/Support/StdioProcess.swift'
output = pathlib.Path(sys.argv[2]) / 'StdioProcess.swift'
output.write_text(source.read_text().replace('private func consume(_ data:', 'func consume(_ data:'))
text = (pathlib.Path(sys.argv[1]) / 'DroppyCode/Support/Text.swift').read_text()
cleanup = text[text.index('enum TextCleanup {'):text.index('\nenum SimpleDiff {')]
(pathlib.Path(sys.argv[2]) / 'TextCleanup.swift').write_text('import Foundation\n' + cleanup)
PY
BENCHMARK_SOURCES=("$BENCHMARK_DIR/StdioProcess.swift" "$BENCHMARK_DIR/TextCleanup.swift")
if [[ -f "$SOURCE_ROOT/DroppyCode/Support/StdioFramer.swift" ]]; then
  BENCHMARK_SOURCES+=("$SOURCE_ROOT/DroppyCode/Support/StdioFramer.swift")
fi
xcrun swiftc -swift-version 6 -O -parse-as-library -target arm64-apple-macos26.0 \
  "${BENCHMARK_SOURCES[@]}" \
  "$SOURCE_ROOT/DroppyCode/Support/JSONValue.swift" "$SOURCE_ROOT/DroppyCode/Support/Shell.swift" \
  Tests/PerformanceBenchmarks.swift -o "$BENCHMARK_DIR/benchmarks"
"$BENCHMARK_DIR/benchmarks"
