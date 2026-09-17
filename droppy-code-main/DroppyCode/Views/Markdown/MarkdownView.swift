import AppKit
import SwiftUI

struct MarkdownView: View, Equatable {
    let text: String
    static let warmCandidateLimit = 96
    /// True while the text is still arriving. A streaming message parses through a slot of
    /// its own instead of the shared cache: it changes on every flush, and each of those
    /// would otherwise take a cache entry away from a finished message that scrolling back
    /// through the thread will ask for again.
    var isStreaming = false

    /// Equal text renders equally, so finished replies are skipped entirely while a
    /// new reply streams or the timeline rebuilds around them.
    nonisolated static func == (lhs: MarkdownView, rhs: MarkdownView) -> Bool {
        lhs.text == rhs.text && lhs.isStreaming == rhs.isStreaming
    }

    var body: some View {
        let blocks = Self.blocks(for: text, streaming: isStreaming)
        let last = blocks.count - 1
        VStack(alignment: .leading, spacing: 12) {
            ForEach(Array(blocks.enumerated()), id: \.offset) { index, block in
                MarkdownBlockView(block: block)
                    .equatable()
                    // Only the block still being written is streaming; the ones above it are
                    // settled and cache like any finished text.
                    .environment(\.markdownStreaming, isStreaming && index == last)
                    .transition(.softAppear)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    /// Parsed blocks by source. Rows are rebuilt as they scroll into view, but a finished
    /// message parses to the same blocks, so each distinct text parses once.
    @MainActor private static var blockCache = RecentCache<String, [MarkdownBlock]>(limit: 800)
    /// The latest parse of each message still streaming, newest last: re-renders between
    /// flushes never parse, and a flush parses only the block the new text extends. A few
    /// slots, so a lead and the heads in its panel streaming together keep theirs.
    @MainActor private static var streamingParses: [(text: String, parse: MarkdownParser.Parse)] = []
    private static let streamingSlots = 4

    @MainActor
    static func blocks(for text: String, streaming: Bool = false) -> [MarkdownBlock] {
        if streaming {
            if let parse = streamingParses.last(where: { $0.text == text }) { return parse.parse.blocks }
            let slot = streamingParses.lastIndex { text.utf8.starts(with: $0.text.utf8) }
            let parsed = MarkdownParser.parse(text, extending: slot.map { streamingParses[$0] })
            if let slot { streamingParses.remove(at: slot) } else if streamingParses.count == streamingSlots { streamingParses.removeFirst() }
            streamingParses.append((text, parsed))
            return parsed.blocks
        }
        if let cached = blockCache.value(for: text) { return cached }
        // A message that just finished streaming is already parsed; keep that parse.
        let parsed: [MarkdownBlock]
        if let slot = streamingParses.lastIndex(where: { $0.text == text }) {
            parsed = streamingParses.remove(at: slot).parse.blocks
        } else {
            parsed = MarkdownParser.parse(text)
        }
        blockCache.insert(parsed, for: text)
        return parsed
    }

    /// Mirrors the parser's fence test for the open-fence walk.
    private static func fenceMarker(in trimmed: String) -> String? {
        for marker in ["```", "~~~"] where trimmed.hasPrefix(marker) {
            guard let first = marker.first else { continue }
            return String(trimmed.prefix(while: { $0 == first }))
        }
        return nil
    }

    /// First non-blank line at or after `index`, for the merge check across the boundary.
    private static func firstContentLine(after index: String.Index, in text: String) -> String? {
        var start = index
        while start < text.endIndex {
            let end = text[start...].firstIndex(of: "\n") ?? text.endIndex
            let line = String(text[start..<end])
            if !line.trimmingCharacters(in: .whitespaces).isEmpty { return line }
            if end == text.endIndex { break }
            start = text.index(after: end)
        }
        return nil
    }

    /// Last non-blank line of the head, without copying it.
    private static func lastContentLine(of head: Substring) -> String? {
        var rest = head
        while rest.hasSuffix("\n") { rest = rest.dropLast() }
        if let newline = rest.lastIndex(of: "\n") { rest = rest[rest.index(after: newline)...] }
        let line = String(rest)
        return line.trimmingCharacters(in: .whitespaces).isEmpty ? nil : line
    }

    /// Approximate source length of the trailing blocks, so the boundary lands before
    /// all of them and the tail re-parse covers each one whole.
    private static func approximateSourceLength(_ blocks: ArraySlice<MarkdownBlock>) -> Int {
        blocks.reduce(0) { $0 + approximateSourceLength($1) + 4 }
    }

    private static func approximateSourceLength(_ block: MarkdownBlock) -> Int {
        switch block {
        case .paragraph(let text), .heading(_, let text):
            return text.count + 2
        case .code(_, let code):
            return code.count + 8
        case .quote(let inner):
            return approximateSourceLength(inner[...]) + 2
        case .table(let header, let rows):
            return header.joined(separator: "|").count + rows.reduce(0) { $0 + $1.joined(separator: "|").count + 2 } + 8
        case .list(_, _, let items):
            return items.reduce(0) { $0 + $1.text.count + approximateSourceLength($1.children[...]) + 4 }
        case .rule:
            return 4
        }
    }

    /// Parses finished messages before their rows are built, off the main thread, so a row
    /// scrolling into view finds its blocks and its paragraphs' runs already made instead of
    /// parsing them on the frame. Texts already cached cost nothing.
    @MainActor
    static func warm(_ texts: some Sequence<String>) async throws {
        try Task.checkCancellation()
        let missing = warmTexts(texts)
        guard !missing.isEmpty else { return }
        let parsed = try await parseWarm(missing)
        try Task.checkCancellation()
        for (text, blocks, inline) in parsed {
            try Task.checkCancellation()
            if !blockCache.contains(text) { blockCache.insert(blocks, for: text) }
            for (source, pretty) in inline {
                try Task.checkCancellation()
                RichLink.warm(source, with: pretty)
            }
        }
    }

    @MainActor
    static func warmTexts(_ texts: some Sequence<String>) -> [String] {
        var missing: [String] = []
        var seen: Set<String> = []
        var remainingBytes = 1_048_576
        for text in texts.prefix(warmCandidateLimit) {
            let bytes = text.utf8.prefix(262_145).count
            guard bytes <= 262_144, bytes <= remainingBytes,
                  !blockCache.contains(text), seen.insert(text).inserted else { continue }
            missing.append(text)
            remainingBytes -= bytes
            if missing.count == 24 { break }
        }
        return missing
    }

    @concurrent
    private nonisolated static func parseWarm(_ texts: [String]) async throws -> [(String, [MarkdownBlock], [(String, AttributedString)])] {
        var parsed: [(String, [MarkdownBlock], [(String, AttributedString)])] = []
        var seenInline: Set<String> = []
        for text in texts {
            try Task.checkCancellation()
            let blocks = MarkdownParser.parse(text)
            var sources: Set<String> = []
            try inlineSources(of: blocks, into: &sources)
            var inline: [(String, AttributedString)] = []
            for source in sources where seenInline.insert(source).inserted {
                try Task.checkCancellation()
                inline.append((source, RichLink.build(source)))
            }
            parsed.append((text, blocks, inline))
        }
        try Task.checkCancellation()
        return parsed
    }

    nonisolated private static func inlineSources(of blocks: [MarkdownBlock], into sources: inout Set<String>) throws {
        for block in blocks {
            try Task.checkCancellation()
            switch block {
            case .paragraph(let text), .heading(_, let text):
                sources.insert(text)
            case .list(_, _, let items):
                for item in items {
                    try Task.checkCancellation()
                    if !item.text.isEmpty { sources.insert(item.text) }
                    try inlineSources(of: item.children, into: &sources)
                }
            case .quote(let inner):
                try inlineSources(of: inner, into: &sources)
            case .code, .table, .rule:
                break
            }
        }
    }
}

struct MarkdownBlockView: View, Equatable {
    let block: MarkdownBlock
    @Environment(\.markdownDimmed) private var dimmed
    @Environment(\.markdownListDepth) private var listDepth
    @Environment(\.chatZoom) private var zoom
    @Environment(\.markdownStreaming) private var streaming
    @Environment(\.hydraMentionPersonas) private var hydraMentionPersonas

    /// Blocks compare by content, so a finished block is skipped while the reply keeps streaming.
    nonisolated static func == (lhs: MarkdownBlockView, rhs: MarkdownBlockView) -> Bool {
        lhs.block == rhs.block
    }

    var body: some View {
        switch block {
        case .heading(let level, let text):
            InlineText(text)
                .font(headingFont(level))
                .padding(.top, level <= 2 ? 6 : 2)
        case .paragraph(let text):
            if listDepth == 0, let mention = leadingMention(in: text) {
                let rest = String(text.dropFirst(mention.consumed))
                Group {
                    if RichLink.containsLinks(in: rest, streaming: streaming) {
                        // Links need the AppKit paragraph, which cannot join the name's
                        // Text; the trimmed rest keeps the gap to the name even.
                        HStack(alignment: .firstTextBaseline, spacing: 6) {
                            HydraGlyph(persona: mention.persona, size: 16 * zoom)
                            Text(verbatim: mention.name)
                                .fontWeight(.bold)
                                .foregroundStyle(mention.persona.color)
                            InlineText(String(rest.drop(while: { $0 == " " })), hasLinks: true)
                        }
                    } else {
                        // One flowing Text, so wrapped lines start under the glyph.
                        let size = 16 * zoom
                        Text(Self.mentionGlyph(mention.persona, size: size))
                            .foregroundColor(mention.persona.color)
                            .baselineOffset(Self.mentionGlyphOffset(size: size))
                            + Text(verbatim: " ")
                            + Text(verbatim: mention.name)
                            .fontWeight(.bold)
                            .foregroundColor(mention.persona.color)
                            + Text(RichLink.prettyAttributed(rest, streaming: streaming))
                    }
                }
                .modifier(MarkdownBlockSelection())
                .fixedSize(horizontal: false, vertical: true)
                .frame(maxWidth: .infinity, alignment: .leading)
            } else {
                InlineText(text, hasLinks: RichLink.containsLinks(in: text, streaming: streaming))
            }
        case .code(let language, let code):
            // The lead's delegation block (info string `hydra`) is a brief for the team, not
            // code: it reads as a card while it streams and whenever it stays in the reply.
            if language?.lowercased() == "hydra" {
                HydraDelegationBlock(json: code)
            } else {
                CodeBlock(language: language, code: code)
            }
        case .list(let ordered, let start, let items):
            VStack(alignment: .leading, spacing: 6) {
                ForEach(Array(items.enumerated()), id: \.offset) { index, item in
                    HStack(alignment: .firstTextBaseline, spacing: 8) {
                        marker(ordered: ordered, number: start + index, checked: item.checked)
                        VStack(alignment: .leading, spacing: 6) {
                            if !item.text.isEmpty { InlineText(item.text, hasLinks: RichLink.containsLinks(in: item.text, streaming: streaming)) }
                            ForEach(Array(item.children.enumerated()), id: \.offset) { _, child in
                                MarkdownBlockView(block: child)
                                    .equatable()
                            }
                        }
                        // Nested lists read one level deeper, so their dots turn into rings.
                        .environment(\.markdownListDepth, listDepth + 1)
                    }
                }
            }
        case .quote(let blocks):
            HStack(alignment: .top, spacing: 10) {
                Capsule().fill(.quaternary).frame(width: 3)
                VStack(alignment: .leading, spacing: 8) {
                    ForEach(Array(blocks.enumerated()), id: \.offset) { _, child in
                        MarkdownBlockView(block: child)
                            .equatable()
                    }
                }
            }
            .foregroundStyle(.secondary)
            .environment(\.markdownDimmed, true)
        case .table(let header, let rows):
            TableBlock(header: header, rows: rows)
        case .rule:
            Rectangle()
                .fill(.quaternary)
                .frame(height: 1)
                .padding(.vertical, 4)
        }
    }

    private func headingFont(_ level: Int) -> Font {
        switch level {
        case 1: .chat(.title2, weight: .semibold, zoom: zoom)
        case 2: .chat(.title3, weight: .semibold, zoom: zoom)
        case 3: .chat(.headline, zoom: zoom)
        default: .chat(.subheadline, weight: .semibold, zoom: zoom)
        }
    }

    /// Lifts the 16pt glyph off the baseline onto the surrounding text's x-height.
    private static func mentionGlyphOffset(size: CGFloat) -> CGFloat { -(size * 0.2) }

    /// The head's dragon as a template image small enough to sit in a Text run.
    /// Redrawn once per (asset, size), then kept; tinted like the name.
    @MainActor
    static func mentionGlyph(_ persona: HydraPersona, size: CGFloat) -> Image {
        let key = persona.asset + "@\(size)"
        if let cached = glyphCache[key] { return cached }
        // Drawn on demand rather than into fixed pixels, so it stays crisp on any display.
        let source = NSImage(named: persona.asset)
        let out = NSImage(size: NSSize(width: size, height: size), flipped: false) { rect in
            source?.draw(in: rect, from: .zero, operation: .sourceOver, fraction: 1)
            return true
        }
        out.isTemplate = true
        let image = Image(nsImage: out).renderingMode(.template)
        glyphCache[key] = image
        return image
    }

    @MainActor private static var glyphCache: [String: Image] = [:]

    /// The head a paragraph opens with, if it opens with one of this thread's heads: the
    /// name itself, or the name wrapped in one inline emphasis marker (`**Otto**`), and
    /// then the end of the text or a word boundary, never another letter (`Bo` is not
    /// `Bob`). `consumed` is how much of the source the glyph and the coloured name
    /// replace, marker included. A name that only opens a list of heads ("Vera, Mira and
    /// Odin are still at work") is not that head's part, and stays plain text.
    private func leadingMention(in text: String) -> (persona: HydraPersona, name: String, consumed: Int)? {
        let boundaries = " :,.;'’-–—("
        for persona in hydraMentionPersonas {
            for marker in ["", "**", "__", "*", "_", "`"] {
                let opener = marker + persona.name + marker
                guard text.hasPrefix(opener) else { continue }
                let rest = text.dropFirst(opener.count)
                guard rest.first == nil || boundaries.contains(rest.first!) else { continue }
                guard !opensList(rest) else { return nil }
                return (persona, persona.name, opener.count)
            }
        }
        return nil
    }

    /// Whether what follows a head's name goes straight on to another head's name, as a
    /// list does: ", Mira" or " and Odin", with or without an emphasis marker.
    private func opensList(_ rest: Substring) -> Bool {
        for joiner in [", ", " and ", " & "] where rest.hasPrefix(joiner) {
            var next = rest.dropFirst(joiner.count)
            for marker in ["**", "__", "*", "_", "`"] where next.hasPrefix(marker) {
                next = next.dropFirst(marker.count)
            }
            return hydraMentionPersonas.contains { next.hasPrefix($0.name) }
        }
        return false
    }

    @ViewBuilder
    private func marker(ordered: Bool, number: Int, checked: Bool?) -> some View {
        if let checked {
            Image(systemName: checked ? "checkmark.circle.fill" : "circle")
                .foregroundStyle(checked ? AnyShapeStyle(.tint) : AnyShapeStyle(.tertiary))
                .imageScale(.small)
        } else if ordered {
            Text("\(number).")
                .monospacedDigit()
                .fontWeight(.medium)
                .foregroundStyle(markerColor)
        } else {
            // A hidden bullet glyph keeps the text baseline; the dot drawn over it is the
            // marker, in the theme accent. Nested levels get a ring instead of a fill.
            Text("•")
                .hidden()
                .overlay {
                    if listDepth == 0 {
                        Circle()
                            .fill(markerColor)
                            .frame(width: 5, height: 5)
                    } else {
                        Circle()
                            .strokeBorder(markerColor, lineWidth: 1.2)
                            .frame(width: 5.5, height: 5.5)
                    }
                }
        }
    }

    private var markerColor: Color {
        Chrome.accent.opacity(dimmed ? 0.55 : 0.9)
    }
}

/// How many lists the block sits inside; the marker style follows it.
private struct MarkdownListDepthKey: EnvironmentKey {
    static let defaultValue = 0
}

extension EnvironmentValues {
    @Entry var hydraMentionPersonas: [HydraPersona] = []

    var markdownListDepth: Int {
        get { self[MarkdownListDepthKey.self] }
        set { self[MarkdownListDepthKey.self] = newValue }
    }
}

/// Pretty linktitels + favicons voor chat-markdown.
///
/// Kale URL's (`https://…`) kwamen volledig in beeld. Afspraak: toon de titel
/// (expliciete `[titel](url)` of host+pad zonder scheme) met de favicon ervoor.
enum RichLink {
    @MainActor private static var prettyCache = RecentCache<String, AttributedString>(limit: 1200)
    /// The paragraphs still being streamed, newest last, kept apart so their every flush
    /// leaves the cache alone. As many slots as `MarkdownView` keeps parses, so a lead and
    /// the heads streaming beside it never evict each other between flushes.
    @MainActor private static var streamingPretty: [(source: String, value: AttributedString)] = []
    private static let streamingSlots = 4

    @MainActor
    static func prettyAttributed(_ source: String, streaming: Bool = false) -> AttributedString {
        if streaming {
            if let pretty = streamingPretty.last(where: { $0.source == source }) { return pretty.value }
            let value = build(source)
            if streamingPretty.count == streamingSlots { streamingPretty.removeFirst() }
            streamingPretty.append((source, value))
            return value
        }
        if let hit = prettyCache.value(for: source) { return hit }
        let value: AttributedString
        if let slot = streamingPretty.lastIndex(where: { $0.source == source }) {
            value = streamingPretty.remove(at: slot).value
        } else {
            value = build(source)
        }
        prettyCache.insert(value, for: source)
        return value
    }

    /// Stores a run built off the main thread (see `MarkdownView.warm`).
    @MainActor
    static func warm(_ source: String, with pretty: AttributedString) {
        if !prettyCache.contains(source) { prettyCache.insert(pretty, for: source) }
    }

    nonisolated static func build(_ source: String) -> AttributedString {
        let base = baseAttributed(source)
        var result = AttributedString()
        for run in base.runs {
            let slice = AttributedString(base[run.range])
            guard let url = run.link else {
                result.append(slice)
                continue
            }
            let display = String(slice.characters)
            var linkSlice: AttributedString
            if isBareDisplay(display, url: url) {
                linkSlice = AttributedString(prettyTitle(for: url))
                linkSlice.link = url
            } else {
                linkSlice = slice
            }
            // Every link renders bold, whether the author titled it or the
            // URL was bare. Other intent bits (italic, code) are preserved.
            var intent = run.inlinePresentationIntent ?? []
            intent.insert(.stronglyEmphasized)
            linkSlice.inlinePresentationIntent = intent
            result.append(linkSlice)
        }
        return result
    }

    @MainActor
    static func linkHosts(for source: String, streaming: Bool = false) -> [String] {
        let pretty = prettyAttributed(source, streaming: streaming)
        var hosts: [String] = []
        var seen = Set<String>()
        for run in pretty.runs {
            if let host = run.link?.host?.lowercased(), !host.isEmpty, seen.insert(host).inserted {
                hosts.append(host)
            }
        }
        return hosts
    }

    /// Short display title for a bare URL. Forge links collapse to their
    /// native reference (`!3190`, `#123`); everything else keeps the host
    /// plus the last path segment, so a deep path never spills into chat.
    static func prettyTitle(for url: URL) -> String {
        if let reference = forgeReference(for: url) { return reference }
        guard var host = url.host?.lowercased(), !host.isEmpty else {
            return compactFallback(url.absoluteString)
        }
        if host.hasPrefix("www.") { host.removeFirst(4) }
        let segments = (url.path.removingPercentEncoding ?? url.path)
            .split(separator: "/")
            .map(String.init)
            .filter { !$0.isEmpty }
        guard let last = segments.last else { return host }
        let tail = last.count > 24 ? String(last.prefix(21)) + "…" : last
        let title = segments.count == 1 ? host + "/" + tail : host + "/…/" + tail
        if title.count > 40 {
            return String(title.prefix(19)) + "…" + String(title.suffix(18))
        }
        return title
    }

    /// `!3190` for merge requests, `#123` for issues and pull requests.
    /// Matches GitLab (`/-/merge_requests/`, `/-/issues/`) and GitHub-style
    /// (`/pull/`, `/issues/`) paths on any host.
    static func forgeReference(for url: URL) -> String? {
        let segments = url.path.split(separator: "/").map(String.init)
        // GitLab nests the kind behind /-/: /group/project/-/merge_requests/3190
        if let dash = segments.firstIndex(of: "-"),
           dash + 2 < segments.count,
           let id = Int(segments[dash + 2]) {
            switch segments[dash + 1] {
            case "merge_requests": return "!\(id)"
            case "issues": return "#\(id)"
            default: break
            }
        }
        // GitHub style: /owner/repo/pull/123 or /owner/repo/issues/123
        if segments.count >= 2, let id = Int(segments.last ?? "") {
            let kind = segments[segments.count - 2]
            if kind == "pull" || kind == "issues" { return "#\(id)" }
        }
        return nil
    }

    /// Hostless URLs (mailto:, custom schemes) have no host to lean on, so
    /// they only get the length cap.
    static func compactFallback(_ absolute: String) -> String {
        guard absolute.count > 40 else { return absolute }
        return String(absolute.prefix(19)) + "…" + String(absolute.suffix(18))
    }

    @MainActor
    static func containsLinks(in source: String, streaming: Bool = false) -> Bool {
        prettyAttributed(source, streaming: streaming).runs.contains { $0.link != nil }
    }

    static func isBareDisplay(_ display: String, url: URL) -> Bool {
        var d = display.trimmingCharacters(in: .whitespacesAndNewlines)
        if d.hasPrefix("<"), d.hasSuffix(">"), d.count >= 2 {
            d = String(d.dropFirst().dropLast())
        }
        while let last = d.last, ".,;:!?)]}>".contains(last) { d.removeLast() }
        if d == url.absoluteString { return true }
        if (d.hasPrefix("http://") || d.hasPrefix("https://")) && !d.contains(" ") && !d.contains("\n") {
            return true
        }
        return false
    }

    private static func baseAttributed(_ source: String) -> AttributedString {
        let options = AttributedString.MarkdownParsingOptions(
            interpretedSyntax: .inlineOnlyPreservingWhitespace,
            failurePolicy: .returnPartiallyParsedIfPossible
        )
        return (try? AttributedString(markdown: source, options: options)) ?? AttributedString(source)
    }
}

/// Base point size for link paragraphs. SwiftUI's `.font` environment is
/// opaque, so the two non-default contexts (thinking text, headings) set
/// this explicitly; everything else reads the 13 pt chat default.
private struct MarkdownPointSizeKey: EnvironmentKey {
    static let defaultValue: CGFloat = 13
}

/// Whether the paragraph renders in secondary styling (quotes, thinking).
/// Threaded explicitly for the same reason as the point size above.
private struct MarkdownDimmedKey: EnvironmentKey {
    static let defaultValue = false
}

/// Whether the paragraph's text is still arriving, so its parses bypass the shared caches.
private struct MarkdownStreamingKey: EnvironmentKey {
    static let defaultValue = false
}

/// Whether each block selects its own text. On everywhere, replies included: a
/// finished reply's blocks are copyable at all times, one paragraph at a time. Whole
/// replies go to the clipboard through the row menu's Copy message (`MessageText`).
private struct MarkdownBlockSelectionKey: EnvironmentKey {
    static let defaultValue = true
}

extension EnvironmentValues {
    var markdownPointSize: CGFloat {
        get { self[MarkdownPointSizeKey.self] }
        set { self[MarkdownPointSizeKey.self] = newValue }
    }
    var markdownDimmed: Bool {
        get { self[MarkdownDimmedKey.self] }
        set { self[MarkdownDimmedKey.self] = newValue }
    }
    var markdownStreaming: Bool {
        get { self[MarkdownStreamingKey.self] }
        set { self[MarkdownStreamingKey.self] = newValue }
    }
    var markdownBlockSelection: Bool {
        get { self[MarkdownBlockSelectionKey.self] }
        set { self[MarkdownBlockSelectionKey.self] = newValue }
    }
}

/// `.textSelection(.enabled)` only while the environment allows blocks their own selection.
struct MarkdownBlockSelection: ViewModifier {
    @Environment(\.markdownBlockSelection) private var enabled
    func body(content: Content) -> some View {
        if enabled {
            content.textSelection(.enabled)
        } else {
            content.textSelection(.disabled)
        }
    }
}

@MainActor
enum FaviconCache {
    private static var memory: [String: NSImage] = [:]

    static func cached(host: String) -> NSImage? {
        memory[host.lowercased()]
    }

    /// Hosts with no icon are remembered too, and concurrent asks share one request: a streaming
    /// reply restarts its row's task on every token, and scrolling re-runs it on every appear.
    private static var failed: Set<String> = []
    private static var inFlight: [String: Task<NSImage?, Never>] = [:]

    static func image(for host: String) async -> NSImage? {
        let key = host.lowercased()
        if let hit = memory[key] { return hit }
        guard !failed.contains(key) else { return nil }
        // Icons come from a service on the web, so the host is told to it. Tool output is
        // full of names that belong to this machine and this network; those never leave.
        guard isPublic(host: key) else {
            failed.insert(key)
            return nil
        }
        if let task = inFlight[key] { return await task.value }
        guard let url = URL(string: "https://www.google.com/s2/favicons?domain=\(key)&sz=64") else { return nil }
        let task = Task { () -> NSImage? in
            guard let (data, _) = try? await URLSession.shared.data(from: url),
                  let raw = NSImage(data: data) else { return nil }
            return resizedIcon(raw)
        }
        inFlight[key] = task
        let image = await task.value
        inFlight[key] = nil
        if let image {
            if memory.count >= 300 { memory.removeAll(keepingCapacity: true) }
            memory[key] = image
        } else {
            failed.insert(key)
        }
        return image
    }

    /// Suffixes that only ever name something inside a network, never a site on the web.
    private static let privateSuffixes = [
        ".local", ".localhost", ".localdomain", ".internal", ".intranet",
        ".lan", ".home", ".home.arpa", ".test", ".invalid", ".corp",
    ]

    /// Whether a host is one a public favicon service could possibly know: not the loopback,
    /// not a bare machine name, not one of the private suffixes, and not an address in the
    /// ranges that never leave a network. A host that fails any of these is never asked about.
    static func isPublic(host: String) -> Bool {
        guard !host.isEmpty, host != "localhost" else { return false }
        // An IPv6 literal (no brackets by the time it comes off a URL) has no favicon worth
        // the request, and half of them are link-local or unique-local addresses.
        guard !host.contains(":") else { return false }
        for suffix in privateSuffixes where host.hasSuffix(suffix) { return false }
        // A single label with no dot in it is a machine on this network, not a site.
        guard host.contains(".") else { return false }
        let labels = host.split(separator: ".", omittingEmptySubsequences: false)
        var octets: [Int] = []
        for label in labels {
            guard let value = Int(label), (0...255).contains(value) else { break }
            octets.append(value)
        }
        guard octets.count == 4, octets.count == labels.count else { return true }
        switch (octets[0], octets[1]) {
        case (0, _), (10, _), (127, _): return false
        case (169, 254), (192, 168): return false
        case (172, 16...31): return false
        // Carrier-grade NAT, which tunnels and VPNs hand out.
        case (100, 64...127): return false
        default: return true
        }
    }

    private static func resizedIcon(_ image: NSImage) -> NSImage {
        let size = NSSize(width: 14, height: 14)
        let out = NSImage(size: size)
        out.lockFocus()
        NSGraphicsContext.current?.imageInterpolation = .high
        image.draw(in: NSRect(origin: .zero, size: size),
                   from: NSRect(origin: .zero, size: image.size),
                   operation: .copy, fraction: 1)
        out.unlockFocus()
        return out
    }
}

struct InlineText: View {
    let source: String
    /// Set when the caller already branched on containsLinks, so the check runs once.
    let hasLinks: Bool?
    @Environment(\.markdownPointSize) private var pointSize
    @Environment(\.chatZoom) private var zoom
    @Environment(\.markdownDimmed) private var dimmed
    @Environment(\.markdownStreaming) private var streaming
    @State private var faviconRevision = 0
    /// The link paragraph's text view, for the hover that sets the cursor.
    @State private var linkView = WeakView()
    /// Whether the pointing hand is up over a link, so hover only sets the cursor on change.
    @State private var showsHand = false

    init(_ source: String, hasLinks: Bool? = nil) {
        self.source = source
        self.hasLinks = hasLinks
    }

    var body: some View {
        // faviconRevision read so loaded favicons rebuild the text.
        let _ = faviconRevision
        let scaled = (pointSize * zoom * 2).rounded() / 2
        // Hosts key the favicon work, so a streamed token with no new link costs nothing.
        let hosts = RichLink.linkHosts(for: source, streaming: streaming)
        let hasLinkText = hasLinks ?? !hosts.isEmpty
        if hasLinkText {
            // An AppKit view has no text baseline of its own, so a list's marker sat on the
            // paragraph's top edge and the text started a line below it. The first line's
            // baseline is the base font's ascender from the top; the last is one line up
            // from the bottom. Measured once per size and kept: the alignment guides run
            // off the main actor, so they take the two numbers rather than the font.
            let metrics = Self.metrics(pointSize: scaled)
            let ascender = metrics.ascender
            let lineHeight = metrics.lineHeight
            // Stable hosts key the favicon work, so a streamed token with no new link
            // leaves the task alone.
            let hostKey = hosts.joined(separator: ",")
            // The hover and the host callback keep only the text view's weak box and the
            // hand binding: the hover responder and the representable must not keep this
            // paragraph (and its styled text) alive after it scrolls away.
            let linkBox = linkView
            let hand = $showsHand
            LinkParagraphView(source: source, pointSize: scaled, dimmed: dimmed, streaming: streaming, revision: faviconRevision, onHost: { [weak linkBox] view in linkBox?.value = view })
                .frame(maxWidth: .infinity, alignment: .leading)
                .alignmentGuide(.firstTextBaseline) { _ in ascender }
                .alignmentGuide(.lastTextBaseline) { $0.height - lineHeight + ascender }
                // The pointing hand over links, from here rather than the text view's own
                // tracking (which AppKit would rebuild every scrolled frame). Hover is off
                // while the timeline scrolls, so this costs nothing then.
                .onContinuousHover(coordinateSpace: .local) { [weak linkBox, hand] phase in
                    let overLink: Bool
                    switch phase {
                    case .active(let point): overLink = (linkBox?.value as? LinkTextView)?.hasLink(at: point) ?? false
                    case .ended: overLink = false
                    }
                    guard overLink != hand.wrappedValue else { return }
                    hand.wrappedValue = overLink
                    if overLink { NSCursor.pointingHand.push() } else { NSCursor.pop() }
                }
                .onDisappear { [hand] in
                    if hand.wrappedValue { NSCursor.pop(); hand.wrappedValue = false }
                }
                // Stable hosts key the favicon work, so a streamed token with no new link
                // leaves the task alone.
                .task(id: hostKey) { [source, streaming, revision = $faviconRevision] in
                    await Self.fetchFavicons(source: source, streaming: streaming, revision: revision)
                }
        } else {
            // Link-free, so the attributed string renders as one Text; bold, italic and
            // code come through as inline presentation intents.
            Text(RichLink.prettyAttributed(source, streaming: streaming))
                .modifier(MarkdownBlockSelection())
                .fixedSize(horizontal: false, vertical: true)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    /// The base font's baseline numbers, measured once per point size. Asking AppKit for a
    /// font and its metrics on every render of every link paragraph is work with one answer.
    @MainActor private static var metricsBySize: [CGFloat: (ascender: CGFloat, lineHeight: CGFloat)] = [:]

    @MainActor
    private static func metrics(pointSize: CGFloat) -> (ascender: CGFloat, lineHeight: CGFloat) {
        if let known = metricsBySize[pointSize] { return known }
        let font = NSFont.systemFont(ofSize: pointSize)
        let measured = (ceil(font.ascender), ceil(font.ascender - font.descender + font.leading))
        metricsBySize[pointSize] = measured
        return measured
    }

    /// Static so the `.task` above keeps only the paragraph's source and the refresh
    /// binding, never the whole view value (and its styled text) after it goes away.
    private static func fetchFavicons(source: String, streaming: Bool, revision: Binding<Int>) async {
        let hosts = await MainActor.run { RichLink.linkHosts(for: source, streaming: streaming) }
        var changed = false
        for host in hosts where FaviconCache.cached(host: host) == nil {
            if await FaviconCache.image(for: host) != nil { changed = true }
        }
        if changed { await MainActor.run { revision.wrappedValue += 1 } }
    }

    @MainActor
    static func attributed(_ source: String, streaming: Bool = false) -> AttributedString {
        RichLink.prettyAttributed(source, streaming: streaming)
    }
}

/// A hover flag kept in an object instead of view state. A code block's body holds its whole
/// text, counts its lines and cuts its head off; running all of that again because the
/// pointer crossed the block is what made the copy control feel sticky over a big dump.
/// Only the control reads this, so only the control re-renders.
@MainActor
@Observable
private final class BlockHover {
    var isHovering = false
}

/// The copy control on a code block's header, in a view of its own so the hover that
/// reveals it never reaches the block's text.
private struct CodeBlockCopyButton: View {
    let text: String
    let hover: BlockHover

    var body: some View {
        CopyButton(text: text)
            .opacity(hover.isHovering ? 1 : 0)
    }
}

struct CodeBlock: View {
    let language: String?
    let code: String

    @Environment(\.chatZoom) private var zoom
    @State private var hover = BlockHover()
    @State private var showsAll = false

    /// Long dumps render collapsed: materializing thousands of lines at once is what
    /// makes expanding feel laggy. The full text is one instant tap away.
    private static let collapsedLineLimit = 120

    var body: some View {
        // A count of newlines, not a split: hovering re-runs this body, and a big dump split
        // into an array of lines on every hover is what made the copy button feel sticky.
        let lineCount = 1 + code.utf8.count { $0 == 0x0A }
        let truncated = !showsAll && lineCount > Self.collapsedLineLimit
        let visible = truncated ? Self.head(of: code, lines: Self.collapsedLineLimit) : code
        // The box alone: the hover handler must not keep this block (and its whole
        // text) alive after it scrolls away.
        let hoverBox = hover
        VStack(alignment: .leading, spacing: 0) {
            HStack {
                Text(language ?? "code")
                    .font(.chat(.caption, zoom: zoom))
                    .foregroundStyle(.secondary)
                Spacer()
                CodeBlockCopyButton(text: code, hover: hover)
            }
            .padding(.leading, 12)
            .padding(.trailing, 6)
            .padding(.top, 6)
            ScrollView(.horizontal, showsIndicators: false) {
                Text(visible)
                    .font(.chat(.callout, design: .monospaced, zoom: zoom))
                    .modifier(MarkdownBlockSelection())
                    .fixedSize(horizontal: true, vertical: true)
                    .padding(.horizontal, 12)
                    .padding(.bottom, 12)
                    .padding(.top, 4)
            }
            if lineCount > Self.collapsedLineLimit {
                Button(showsAll ? "Show less" : "Show all \(lineCount) lines") {
                    showsAll.toggle()
                }
                .buttonStyle(.link)
                .font(.chat(.caption, zoom: zoom))
                .foregroundStyle(.secondary)
                .padding(.horizontal, 12)
                .padding(.bottom, 10)
            }
        }
        .background(.quaternary.opacity(0.45), in: .rect(cornerRadius: 12))
        .onHover { [weak hoverBox] in hoverBox?.isHovering = $0 }
    }

    /// The first `lines` lines of `code`, without splitting the rest.
    private static func head(of code: String, lines: Int) -> String {
        var remaining = lines
        var end = code.utf8.startIndex
        for index in code.utf8.indices where code.utf8[index] == 0x0A {
            remaining -= 1
            if remaining == 0 {
                end = index
                break
            }
        }
        return remaining == 0 ? String(code[..<end]) : code
    }
}

struct TableBlock: View {
    let header: [String]
    let rows: [[String]]

    @Environment(\.chatZoom) private var zoom
    @State private var showsAll = false

    private static let collapsedRowLimit = 30

    /// Cell runs by source text, so a re-layout joins cached runs instead of re-parsing.
    @MainActor private static var cellCache = RecentCache<String, AttributedString>(limit: 1200)

    @MainActor
    private static func cell(_ source: String) -> AttributedString {
        if let hit = cellCache.value(for: source) { return hit }
        let value = InlineText.attributed(source)
        cellCache.insert(value, for: source)
        return value
    }

    var body: some View {
        let visible = showsAll ? rows : Array(rows.prefix(Self.collapsedRowLimit))
        let head = header.map(Self.cell)
        let cells = visible.map { $0.map(Self.cell) }
        return VStack(alignment: .leading, spacing: 0) {
            ScrollView(.horizontal, showsIndicators: false) {
                // Cells go through the shared cache even while the table streams: only its
                // last row changes between flushes, and the rest hit.
                Grid(alignment: .leading, horizontalSpacing: 18, verticalSpacing: 8) {
                    GridRow {
                        ForEach(head.indices, id: \.self) { column in
                            Text(head[column])
                                .fontWeight(.semibold)
                        }
                    }
                    ForEach(cells.indices, id: \.self) { row in
                        GridRow {
                            ForEach(head.indices, id: \.self) { column in
                                Text(column < cells[row].count ? cells[row][column] : AttributedString(""))
                                    .foregroundStyle(.primary.opacity(0.9))
                            }
                        }
                    }
                }
                .modifier(MarkdownBlockSelection())
                .padding(12)
            }
            if !showsAll, rows.count > Self.collapsedRowLimit {
                Button("Show all \(rows.count) rows") {
                    showsAll.toggle()
                }
                .buttonStyle(.link)
                .font(.chat(.caption, zoom: zoom))
                .foregroundStyle(.secondary)
                .padding(.horizontal, 12)
                .padding(.bottom, 10)
            }
        }
        .background(.quaternary.opacity(0.35), in: .rect(cornerRadius: 12))
    }
}

struct CopyButton: View {
    let text: String
    var label = "Copy"

    @State private var didCopy = false

    var body: some View {
        Button {
            NSPasteboard.general.clearContents()
            NSPasteboard.general.setString(text, forType: .string)
            didCopy = true
            // The binding alone: the delayed reset must not keep this button (and its
            // whole text) alive.
            Task { [copied = $didCopy] in
                try? await Task.sleep(for: .seconds(1.4))
                copied.wrappedValue = false
            }
        } label: {
            Image(systemName: didCopy ? "checkmark" : "doc.on.doc")
                .contentTransition(.symbolEffect(.replace))
                .frame(width: 22, height: 22)
                .contentShape(.rect)
        }
        .buttonStyle(.borderless)
        .foregroundStyle(.secondary)
        .help(label)
    }
}
