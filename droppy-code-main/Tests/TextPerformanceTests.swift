import Foundation
import Testing
@testable import DroppyCode

@Test func textCleanupPreservesUnicodeAndLineSemantics() {
    for text in ["", "plain ASCII", "Ελληνικά中文👩🏽‍💻", "curly ‘quotes’", "—x―y⸺z⸻", "—\u{301}", " \n\nbody", "\n\nfirst\nsecond"] {
        let expected = String(text.map { ["—", "―", "⸺", "⸻"].contains($0) ? "-" : $0 })
        #expect(TextCleanup.withoutEmDashes(text) == expected)
        for limit in [1, 5, 120] {
            let first = text.split(whereSeparator: \.isNewline).first.map(String.init)?.trimmingCharacters(in: .whitespaces) ?? ""
            let expected = first.count > limit ? String(first.prefix(limit - 1)) + "…" : first
            #expect(TextCleanup.singleLine(text, limit: limit) == expected)
        }
    }
    let manyLines = "summary\n" + String(repeating: "output\n", count: 100_000)
    #expect(TextCleanup.singleLine(manyLines) == "summary")
    // Prose full of other E2-led scalars is returned as the same string, untouched.
    let curly = "“quoted” … → ‘single’ – en dash"
    #expect(TextCleanup.withoutEmDashes(curly) == curly)
    #expect(TextCleanup.withoutEmDashes("a—b") == "a-b")
    #expect(TextCleanup.withoutEmDashes("a\u{2014}\u{301}b") == "a\u{2014}\u{301}b")
}

@Test func reportedEditsPastTheCapKeepCountsAndDropTheHunk() {
    let small = FileEdit(path: "a.txt", old: "one\ntwo\n", new: "one\nthree\n")
    #expect(small.diff?.contains("-two") == true)
    #expect(small.additions == 1 && small.deletions == 1)
    let big = String(repeating: "line\n", count: 60_000)
    let capped = FileEdit(path: "b.txt", old: "", new: big)
    #expect(capped.diff == nil)
    #expect(capped.additions == 60_001 && capped.deletions == 0)
}

@Test @MainActor func patchPathsPreserveFirstOccurrenceAndUnicode() {
    let patch = """
    *** Begin Patch
    *** Update File: src/first.swift
    *** Add File:  src/中文.swift
    *** Delete File: src/first.swift
    *** Update File:\(" ")
    *** Add File: src/👩🏽‍💻.swift
    *** End Patch
    """
    #expect(CopilotSession.patchPaths(patch) == ["src/first.swift", "src/中文.swift", "src/👩🏽‍💻.swift"])
}
