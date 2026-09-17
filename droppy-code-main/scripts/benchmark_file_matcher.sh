#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."
MATCHER_TMP="$(mktemp -d "${TMPDIR:-/tmp}/droppy-matcher.XXXXXX")"
trap 'rm -rf "$MATCHER_TMP"' EXIT
python3 - "$MATCHER_TMP/Matcher.swift" <<'PY'
from pathlib import Path
import re
import sys
source = Path('DroppyCode/Views/Composer/ComposerView.swift').read_text()
match = re.search(r'    nonisolated static func isSubsequence\([^\n]+\n.*?\n    }\n', source, re.S)
if not match:
    raise SystemExit('Production FileIndex matcher not found')
Path(sys.argv[1]).write_text('import Foundation\nenum Matcher {\n' + match.group(0) + '}\n')
PY
cat >> "$MATCHER_TMP/Matcher.swift" <<'SWIFT'

let fixtures = [
    ("fswt", "sources/file.swift", true), ("none", "sources/file.swift", false),
    ("e", "e\u{301}", false), ("é", "e\u{301}", true), ("i", "i\u{307}", false),
    ("中", "文中文", true), ("👩", "👩‍💻", false), ("👩‍💻", "src/👩‍💻.swift", true),
    ("\n", "a\r\nb", false), ("\r", "a\r\nb", false), ("\r\n", "a\r\nb", true),
    ("ab", "a\r\nb", true), ("", "anything", true), ("a", "", false)
]
for (needle, haystack, expected) in fixtures {
    let ascii = needle.utf8.allSatisfy { $0 < 128 } && haystack.utf8.allSatisfy { $0 < 128 }
    precondition(Matcher.isSubsequence(needle, of: haystack, ascii: ascii) == expected)
    precondition(Matcher.isSubsequence(needle, of: haystack, ascii: ascii)
                 == Matcher.isSubsequence(needle, of: haystack, ascii: false))
}
var words = [""]
var level = [""]
for _ in 0..<4 {
    level = level.flatMap { prefix in ["a", "b", "\r", "\n"].map { prefix + $0 } }
    words += level
}
for needle in words.prefix(85) {
    for haystack in words {
        precondition(Matcher.isSubsequence(needle, of: haystack, ascii: true)
                     == Matcher.isSubsequence(needle, of: haystack, ascii: false))
    }
}
print("Passed 14 Unicode/ASCII fixtures and \(85 * words.count) exhaustive ASCII/CRLF comparisons")

let paths = (0..<20_000).map { "sources/feature-\($0)/file.swift" }
for query in ["fswt", "nonexistent"] {
    for ascii in [false, true] {
        var checksum = 0
        let start = ContinuousClock.now
        for _ in 0..<10 {
            for path in paths {
                if Matcher.isSubsequence(query, of: path, ascii: ascii) { checksum += 1 }
            }
        }
        let elapsed = ContinuousClock.now - start
        precondition(checksum == (query == "fswt" ? 200_000 : 0))
        print("\(query), ASCII=\(ascii), 20000 paths × 10: \(elapsed)")
    }
}
SWIFT
xcrun swiftc -O "$MATCHER_TMP/Matcher.swift" -o "$MATCHER_TMP/matcher"
"$MATCHER_TMP/matcher"
