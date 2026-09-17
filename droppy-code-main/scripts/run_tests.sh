#!/bin/bash
# Builds the Debug app (Droppy Code Dev) without installing or launching it, then runs
# every suite under scripts/test_*.sh against that build.
#
#   scripts/run_tests.sh              # all suites
#   scripts/run_tests.sh runtime_performance composer
#
# The build lands in build.noindex/dev, the folder scripts/quick_run.sh uses, so a run
# after quick_run.sh skips straight to the tests. Set DROPPY_TEST_PRODUCTS to test another
# build's Products/Debug folder.
set -euo pipefail
cd "$(dirname "$0")/.."
mkdir -p build.noindex/dev
if [[ -z "${DROPPY_TEST_PRODUCTS:-}" ]]; then
  xcodebuild -project DroppyCode.xcodeproj -scheme DroppyCode -configuration Debug \
    -destination 'platform=macOS' -derivedDataPath build.noindex/dev \
    -skipPackagePluginValidation CODE_SIGNING_ALLOWED=NO build > build.noindex/dev-test-build.log 2>&1 || {
      tail -n 40 build.noindex/dev-test-build.log
      exit 1
    }
fi
suites=("$@")
if [[ ${#suites[@]} -eq 0 ]]; then
  for script in scripts/test_*.sh; do
    name="$(basename "$script" .sh)"
    suites+=("${name#test_}")
  done
fi
failed=()
for suite in "${suites[@]}"; do
  printf '\n==> %s\n' "$suite"
  bash "scripts/test_$suite.sh" || failed+=("$suite")
done
if [[ ${#failed[@]} -gt 0 ]]; then
  printf '\nFailed: %s\n' "${failed[*]}"
  exit 1
fi
