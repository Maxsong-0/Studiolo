import Foundation
import Testing
@testable import DroppyCode

@Test @MainActor func displayBlocksAcceptLeadingEntriesWithoutTurn() {
    let entries = [
        TimelineEntry(TimelineItem(id: "notice", turnID: nil, content: .notice(Notice(level: .info, message: "Ready")))),
        TimelineEntry(TimelineItem(id: "reply", turnID: nil, content: .assistant(AssistantMessage(text: "Hello"))))
    ]
    let blocks = DisplayBlock.build(entries, meta: TimelineMeta.build(entries), showReasoning: true, isRunning: false, hydraMergeID: nil)
    #expect(blocks.map(\.id) == ["notice", "reply"])
    #expect(DisplayBlock.build([], meta: TimelineMeta(), showReasoning: true, isRunning: false, hydraMergeID: nil).isEmpty)
}

@Test @MainActor func toolGroupsKeepReplyCollapseAndTurnBoundaries() {
    let firstTurn = UUID()
    let secondTurn = UUID()
    let entries = [
        TimelineEntry(TimelineItem(id: "a", turnID: firstTurn, content: .tool(ToolCall(kind: .read, title: "Read")))),
        TimelineEntry(TimelineItem(id: "thinking", turnID: firstTurn, content: .reasoning(ReasoningBlock(text: "Thinking")))),
        TimelineEntry(TimelineItem(id: "b", turnID: firstTurn, content: .tool(ToolCall(kind: .read, title: "Read")))),
        TimelineEntry(TimelineItem(id: "reply", turnID: firstTurn, content: .assistant(AssistantMessage(text: "Done")))),
        TimelineEntry(TimelineItem(id: "c", turnID: firstTurn, content: .tool(ToolCall(kind: .read, title: "Read")))),
        TimelineEntry(TimelineItem(id: "d", turnID: secondTurn, content: .tool(ToolCall(kind: .read, title: "Read"))))
    ]
    let groups = TimelineGroup.build(entries, showReasoning: true)
    #expect(groups.count == 4)
    guard case .work(_, let earlier, let collapsed) = groups[0],
          case .work(_, let later, let expanded) = groups[2],
          case .work(_, let next, let nextExpanded) = groups[3] else {
        Issue.record("Expected three tool groups separated by a reply and a turn boundary")
        return
    }
    #expect(earlier.map(\.id) == ["a", "b"])
    #expect(collapsed)
    #expect(later.map(\.id) == ["c"] && !expanded)
    #expect(next.map(\.id) == ["d"] && !nextExpanded)
}

@Test @MainActor func fileIndexesShareColdPreparation() async throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent("droppy-index-\(UUID())")
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: directory) }
    try Data().write(to: directory.appendingPathComponent("Needle.swift"))
    let first = FileIndex()
    let second = FileIndex()
    let firstLoad = Task { await first.prepare(directory.path) }
    let secondLoad = Task { await second.prepare(directory.path) }
    await firstLoad.value
    await secondLoad.value
    let deadline = ContinuousClock.now + .seconds(5)
    while await first.search("Needle").isEmpty, ContinuousClock.now < deadline {
        try await Task.sleep(for: .milliseconds(10))
    }
    let found = await first.search("Needle")
    #expect(found.map { ($0 as NSString).lastPathComponent } == ["Needle.swift"])
    #expect(await second.search("Needle") == found)
}

@Test @MainActor func fileRankingMatchesStableReferenceIncludingUnicode() async throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent("droppy-rank-\(UUID())").resolvingSymlinksInPath()
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: directory) }
    var paths = ["A/needle.swift", "B/needle.swift", "needle", "needleLong.swift", "xneedle.swift", "needle-dir/other.txt",
                 "İstanbul.swift", "Straße.swift", "σς.swift", "中文.swift", "👩‍💻.swift", "Café.swift", "Other/Cafe\u{301}.swift", "n_e_e_d_l_e.txt"]
    paths += (0..<80).map { "Sources/part\($0)/needle-\($0).swift" }
    paths += [String(repeating: "long", count: 30) + "/needle.swift"]
    for path in paths {
        let file = directory.appendingPathComponent(path)
        try FileManager.default.createDirectory(at: file.deletingLastPathComponent(), withIntermediateDirectories: true)
        try Data().write(to: file)
    }
    let index = FileIndex()
    await index.prepare(directory.path)
    let ordered = await index.search("", limit: Int.max)
    #expect(ordered.count == paths.count)
    for query in ["", "needle", "NEEDLE", "ndl", "swift", "Sources/", "İ", "ı", "ß", "σ", "中", "👩‍💻", "café", "unmatched"] {
        for limit in [1, 8, 100] {
            #expect(await index.search(query, limit: limit) == referenceRanking(ordered, query: query, limit: limit))
        }
    }
    #expect(await index.search("needle", limit: 0).isEmpty)
}

@Test @MainActor func cancelledIndexConsumerCanRetryWithoutCancellingSharedBuild() async throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent("droppy-cancel-index-\(UUID())").resolvingSymlinksInPath()
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: directory) }
    try Data().write(to: directory.appendingPathComponent("shared.swift"))
    let cancelled = FileIndex()
    let other = FileIndex()
    let first = Task { await cancelled.prepare(directory.path) }
    let second = Task { await other.prepare(directory.path) }
    first.cancel()
    await first.value
    await second.value
    let shared = await other.search("shared")
    #expect(shared.map { ($0 as NSString).lastPathComponent } == ["shared.swift"])
    await cancelled.prepare(directory.path)
    #expect(await cancelled.search("shared") == shared)
}

@Test @MainActor func overtakenFileScanPublishesNothing() async throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent("droppy-scan-\(UUID())").resolvingSymlinksInPath()
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: directory) }
    // Past 4,096 entries the scan looks for cancellation.
    for index in 0..<4_200 { try Data().write(to: directory.appendingPathComponent("f\(index).swift")) }
    let index = FileIndex()
    await index.prepare(directory.path)
    #expect(await index.search("f1").count == 8)
    let overtaken = Task { @MainActor in await index.search("f1") }
    overtaken.cancel()
    #expect(await overtaken.value.isEmpty)
}

@Test @MainActor func trackEffectTraitsMatchTheirSeedsAtEveryWidth() {
    let traits = TrackEffect.Traits.build(width: 300)
    #expect(traits.particles.count == 60)
    #expect(traits.streaks.count == 21)
    #expect(traits.particles[7].speed == 12 + 28 * TrackEffect.random(7, 2))
    #expect(traits.particles[59].shimmerRate == 2 + 3 * TrackEffect.random(59, 6))
    #expect(traits.streaks[4].brightness == 0.35 + 0.55 * TrackEffect.random(104, 6))
    // Narrow fills keep their floors, and an element keeps its traits as the fill grows.
    #expect(TrackEffect.Traits.build(width: 40).particles.count == 10)
    #expect(TrackEffect.Traits.build(width: 40).streaks.count == 5)
    #expect(Array(TrackEffect.Traits.build(width: 600).particles.prefix(60)) == traits.particles)
}

@Test @MainActor func fileSubsequencePreservesGraphemeAndCRLFBoundaries() {
    let cases = [
        ("fswt", "sources/file.swift", true), ("none", "sources/file.swift", false),
        ("e", "e\u{301}", false), ("é", "e\u{301}", true),
        ("i", "i\u{307}", false), ("中", "文中文", true),
        ("👩", "👩‍💻", false), ("👩‍💻", "src/👩‍💻.swift", true),
        ("\n", "a\r\nb", false), ("\r", "a\r\nb", false),
        ("\r\n", "a\r\nb", true), ("ab", "a\r\nb", true),
        ("", "anything", true), ("a", "", false)
    ]
    for (needle, haystack, expected) in cases {
        let ascii = needle.utf8.allSatisfy { $0 < 128 } && haystack.utf8.allSatisfy { $0 < 128 }
        #expect(FileIndex.isSubsequence(needle, of: haystack, ascii: ascii) == expected)
        #expect(FileIndex.isSubsequence(needle, of: haystack, ascii: ascii)
                == FileIndex.isSubsequence(needle, of: haystack, ascii: false))
    }
}

@Test @MainActor func fileIndexEvictsOldDirectoriesAndRejectsStaleInstanceResults() async throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent("droppy-index-eviction-\(UUID())")
    defer { try? FileManager.default.removeItem(at: root) }
    let directories = (0..<6).map { root.appendingPathComponent(String($0)) }
    for directory in directories {
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        try Data().write(to: directory.appendingPathComponent("file-\(directory.lastPathComponent).swift"))
        await FileIndex().prepare(directory.path)
    }
    try Data().write(to: directories[0].appendingPathComponent("new-after-eviction.swift"))
    let index = FileIndex()
    await index.prepare(directories[0].path)
    #expect(await index.search("new-after-eviction").count == 1)
    var started = false
    let oldLoad = Task {
        started = true
        await index.prepare(directories[1].path)
    }
    while !started { await Task.yield() }
    await index.prepare(directories[5].path)
    await oldLoad.value
    #expect(await index.search("file-5").count == 1)
    #expect(await index.search("file-1").isEmpty)
}

@Test @MainActor func fileIndexesCompleteQueuedDirectoriesAndSharedWaiters() async throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent("droppy-index-queued-\(UUID())")
    defer { try? FileManager.default.removeItem(at: root) }
    let directories = (0..<12).map { root.appendingPathComponent(String($0)) }
    for directory in directories {
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        try Data().write(to: directory.appendingPathComponent("file-\(directory.lastPathComponent).swift"))
    }
    let indexes = (0..<24).map { _ in FileIndex() }
    let loads = indexes.enumerated().map { offset, index in
        Task { await index.prepare(directories[offset / 2].path) }
    }
    for load in loads { await load.value }
    for (offset, index) in indexes.enumerated() {
        #expect(await index.search("file-\(offset / 2).swift").count == 1)
    }
}

@Test @MainActor func collapsedDiffPreservesTintMergingAndStopsAtItsBudget() {
    let lines = (0..<10_000).map { DiffLine(id: $0, kind: $0 < 3 ? .context : .addition, text: "line \($0)") }
    let file = DiffFile(path: "test.swift", change: .modified, isBinary: false, hunks: [
        DiffHunk(id: 0, header: "first", lines: Array(lines.prefix(4))),
        DiffHunk(id: 1, header: "continued", lines: Array(lines.dropFirst(4)))
    ])
    let sections = DiffLinesView.makeSections(file: file, showsLineNumbers: true, limit: 200)
    #expect(sections == [.header("first", 0), .plain(Array(lines.prefix(3))), .tinted(Array(lines[3..<200]))])
    let full = DiffLinesView.makeSections(file: file, showsLineNumbers: false, limit: Int.max)
    #expect(full == [.plain(Array(lines.prefix(3))), .tinted(Array(lines.dropFirst(3)))])
    #expect(DiffLinesView.makeSections(file: file, showsLineNumbers: true, limit: 0).isEmpty)
    let split = DiffFile(path: "split", change: .modified, isBinary: false, hunks: [
        DiffHunk(id: 0, header: "empty", lines: []),
        DiffHunk(id: 1, header: "add", lines: [lines[3]]),
        DiffHunk(id: 2, header: "context", lines: [lines[0]])
    ])
    let splitSections = DiffLinesView.makeSections(file: split, showsLineNumbers: true, limit: 2)
    #expect(splitSections == [.header("add", 1), .tinted([lines[3]]), .header("context", 2), .plain([lines[0]])])
    // Header ids stay clear of line ids, whatever numbers the parser hands out.
    #expect(Set(splitSections.map(\.id)).count == splitSections.count)
}

@Test @MainActor func markdownWarmBudgetsUniqueMessagesAndBytes() {
    let prefix = UUID().uuidString
    let messages = (0..<100).map { "\(prefix)-\($0)" }
    #expect(MarkdownView.warmTexts(messages) == Array(messages.prefix(24)))
    #expect(MarkdownView.warmTexts([messages[0], messages[0], messages[1]]) == Array(messages.prefix(2)))
    let large = prefix + String(repeating: "x", count: 262_144)
    #expect(MarkdownView.warmTexts([large, messages[0]]) == [messages[0]])
    let quarterMegabyte = String(repeating: "x", count: 262_100)
    let sized = (0..<8).map { prefix + String($0) + quarterMegabyte }
    #expect(MarkdownView.warmTexts(sized).count == 4)
    #expect(MarkdownView.warmTexts(Array(repeating: messages[0], count: 96) + [messages[1]]) == [messages[0]])
}

@Test @MainActor func markdownWarmPublishesCompletedResultsAndRejectsCancelledResults() async throws {
    let text = "# \(UUID())\n\nHello **world**.\n\n- One\n  - Nested\n\n> Quoted"
    let expected = MarkdownParser.parse(text)
    try await MarkdownView.warm([text, text])
    #expect(MarkdownView.warmTexts([text]).isEmpty)
    #expect(MarkdownView.blocks(for: text) == expected)
    let cancelledText = "Cancelled \(UUID())"
    let cancelled = Task { try await MarkdownView.warm([cancelledText]) }
    cancelled.cancel()
    do {
        try await cancelled.value
        Issue.record("Cancelled warming should throw")
    } catch is CancellationError {}
    #expect(MarkdownView.warmTexts([cancelledText]) == [cancelledText])
}

/// Markdown that exercises every block kind, so a streaming prefix of it passes through every
/// open-block state: fences (both markers, unclosed), headings, rules, quotes, tables with a
/// partial delimiter, nested and ordered lists, task items, tab indents, CRLF, whitespace-only
/// lines and multi-byte text.
private let streamingMarkdownFixtures = [
    "# Title\n\nFirst paragraph with **bold** and a — dash.\nSecond line | not a table\n\n- one\n- two\n  - nested\n\n    - deeper\n\n1. first\n2. second\n\n3) third\n\n```swift\nlet x = 1\n\n```\n\n~~~\nfenced ~~~ too\n~~\n~~~\n\n> quoted\n> more\n\n| a | b |\n|---|:-:|\n| 1 | 2 |\n| 3 |\n\n---\n\n* * *\n\nTail paragraph\nwith ordered 2. inside\n\n- [ ] todo\n- [x] done\n\n\ttabbed\n   \nİstanbul 👩‍💻 中文 e\u{301}\n",
    "para\r\n\r\n- item\r\n\r\n  child\r\n\r\n| h | i |\r\n| - | - |\r\nrow | cells\r\n\r\n## Heading\r\n```\r\ncode\r\n",
    "text\n2. item\n3. more\n\n1. start\n\n\n- a\n\n\n- b\nafter\n>q\n#not heading\n#\n# \n|\n|-\n|-|\n- x\n\n  ",
    "\n\n  \n\t\nlast",
    // A partial last line that reads as a heading, rule, ordered marker or table delimiter
    // closes the paragraph above it until the next byte arrives.
    "abc\n#x\nabc\n##x\nabc\n***bold***\nabc\n---x\nabc\n1.5 million\nabc\nx|y\n-x\nabc\n2.\n3.\n",
]

@Test @MainActor func markdownNestingIsBoundedForHostileInput() {
    let quotes = MarkdownParser.parse(String(repeating: "> ", count: 20_000) + "x")
    #expect(quotes.count == 1)
    let list = (0..<2_000).map { String(repeating: " ", count: $0) + "- item" }.joined(separator: "\n")
    #expect(!MarkdownParser.parse(list).isEmpty)
    #expect(MarkdownParser.parse("- a\n  - b\n    - c") == [
        .list(ordered: false, start: 1, items: [MarkdownListItem(text: "a", checked: nil, children: [
            .list(ordered: false, start: 1, items: [MarkdownListItem(text: "b", checked: nil, children: [
                .list(ordered: false, start: 1, items: [MarkdownListItem(text: "c", checked: nil, children: [])])
            ])])
        ])])
    ])
    #expect(MarkdownParser.parse("- a\n***\n- - -\n-x") == [
        .list(ordered: false, start: 1, items: [
            MarkdownListItem(text: "a\n***", checked: nil, children: []),
            MarkdownListItem(text: "- -\n-x", checked: nil, children: [])
        ])
    ])
    #expect(MarkdownParser.parse("***\n- - -\n_ _\n**") == [.rule, .rule, .paragraph("_ _\n**")])
}

@Test @MainActor func markdownIncrementalParseMatchesFullParseAtEveryPrefix() {
    for document in streamingMarkdownFixtures {
        var previous: (text: String, parse: MarkdownParser.Parse)?
        var prefix = ""
        for scalar in document.unicodeScalars {
            prefix.unicodeScalars.append(scalar)
            let incremental = MarkdownParser.parse(prefix, extending: previous)
            let full = MarkdownParser.parse(prefix, extending: nil)
            #expect(incremental == full, "prefix of \(prefix.utf8.count) bytes: \(prefix.suffix(40).debugDescription)")
            #expect(full.blocks == MarkdownParser.parse(prefix))
            previous = (prefix, incremental)
        }
        // A rewrite that is not an extension falls back to a full parse.
        let rewritten = String(document.dropLast(3)) + "!"
        #expect(MarkdownParser.parse(rewritten, extending: previous) == MarkdownParser.parse(rewritten, extending: nil))
    }
}

@Test @MainActor func markdownStreamingSlotsServeInterleavedMessages() {
    let prefix = UUID().uuidString
    let first = "\(prefix) one\n\n- a\n- b\n\nfinal one"
    let second = "\(prefix) two\n\n```\ncode\n```\n\nfinal two"
    var firstShown = ""
    var secondShown = ""
    for step in 0..<max(first.count, second.count) {
        if step < first.count {
            firstShown = String(first.prefix(step + 1))
            #expect(MarkdownView.blocks(for: firstShown, streaming: true) == MarkdownParser.parse(firstShown))
        }
        if step < second.count {
            secondShown = String(second.prefix(step + 1))
            #expect(MarkdownView.blocks(for: secondShown, streaming: true) == MarkdownParser.parse(secondShown))
        }
    }
    #expect(MarkdownView.blocks(for: first) == MarkdownParser.parse(first))
    #expect(MarkdownView.blocks(for: second) == MarkdownParser.parse(second))
}

@Test(.enabled(if: ProcessInfo.processInfo.environment["DROPPY_RENDER_BENCHMARKS"] == "1"))
@MainActor func optimizedRenderingBenchmarks() async throws {
    let reply = String(repeating: streamingMarkdownFixtures[0], count: 40)
    var flushes: [String] = []
    var cursor = reply.startIndex
    while cursor < reply.endIndex {
        cursor = reply.index(cursor, offsetBy: 64, limitedBy: reply.endIndex) ?? reply.endIndex
        flushes.append(String(reply[..<cursor]))
    }
    for (label, incremental) in [("full re-parse", false), ("incremental", true)] {
        let start = ContinuousClock.now
        var checksum = 0
        for _ in 0..<5 {
            var previous: (text: String, parse: MarkdownParser.Parse)?
            for flush in flushes {
                let parse = MarkdownParser.parse(flush, extending: incremental ? previous : nil)
                checksum += parse.blocks.count
                previous = (flush, parse)
            }
        }
        print("Streaming \(reply.utf8.count) bytes in \(flushes.count) flushes × 5, \(label): \(ContinuousClock.now - start) blocks=\(checksum)")
    }
    for count in [2_000, 10_000, 50_000] {
        let turn = UUID()
        let entries = (0..<count).map { index in
            TimelineEntry(TimelineItem(id: String(index), turnID: turn, content: index.isMultiple(of: 2)
                ? .tool(ToolCall(kind: .read, title: "Read")) : .assistant(AssistantMessage(text: "Done"))))
        }
        var checksum = 0
        let start = ContinuousClock.now
        for _ in 0..<10 { checksum += TimelineGroup.build(entries, showReasoning: true).count }
        let duration = ContinuousClock.now - start
        #expect(checksum == count * 10)
        print("Grouping \(count) entries × 10: \(duration)")
    }
    for count in [200, 20_000, 200_000] {
        let lines = (0..<count).map { DiffLine(id: $0, kind: .addition, text: "line") }
        let file = DiffFile(path: "large", change: .modified, isBinary: false,
                            hunks: [DiffHunk(id: 0, header: "header", lines: lines)])
        var checksum = 0
        let start = ContinuousClock.now
        for _ in 0..<1_000 {
            checksum += DiffLinesView.makeSections(file: file, showsLineNumbers: true, limit: 200).count
        }
        let duration = ContinuousClock.now - start
        #expect(checksum == 2_000)
        print("Collapsed diff \(count) total lines × 1000: \(duration)")
    }
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent("droppy-index-benchmark-\(UUID())")
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: directory) }
    for index in 0..<20_000 {
        try Data().write(to: directory.appendingPathComponent("File-\(index).swift"))
    }
    let index = FileIndex()
    await index.prepare(directory.path)
    #expect(await index.search("", limit: Int.max).count == 20_000)
    for query in ["File", "swift", "fswt", "nonexistent"] {
        var checksum = 0
        let start = ContinuousClock.now
        for _ in 0..<20 { checksum += await index.search(query).count }
        let duration = ContinuousClock.now - start
        #expect(checksum == (query == "nonexistent" ? 0 : 160))
        print("File search 20000 entries, \(query), × 20: \(duration)")
    }
}

private func referenceRanking(_ paths: [String], query: String, limit: Int) -> [String] {
    let needle = query.lowercased()
    guard !needle.isEmpty else { return Array(paths.prefix(limit)) }
    return paths.compactMap { path -> (String, Int)? in
        let lowercased = path.lowercased()
        let name = (lowercased as NSString).lastPathComponent
        let score: Int
        if name == needle { score = 1_000 }
        else if name.hasPrefix(needle) { score = 800 }
        else if name.contains(needle) { score = 600 }
        else if lowercased.contains(needle) { score = 400 }
        else {
            var remaining = needle[...]
            for character in lowercased where character == remaining.first { remaining = remaining.dropFirst() }
            guard remaining.isEmpty else { return nil }
            score = 100
        }
        return (path, score - min(path.count, 99))
    }.sorted { $0.1 > $1.1 }.prefix(limit).map(\.0)
}

@main struct RenderingPerformanceTests {
    static func main() async {
        let result: CInt = await Testing.__swiftPMEntryPoint()
        exit(result)
    }
}
