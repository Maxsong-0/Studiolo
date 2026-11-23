// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import AppKit
import SwiftUI

/// Streamed text arrives the way zeron draws it (`crates/ui/src/markdown/veil.rs`, itself a
/// port of mugen-markdown's `FadePainter`): every flush is committed to layout at once, and a
/// purely cosmetic veil over the characters that just arrived dissolves over the next beat.
/// Nothing moves and nothing resizes; a token that fades in exactly where it will stay is
/// what reads as text being written, where a token that popped in whole read as a glitch.
///
/// - Each append registers the new tail as a chunk with its arrival time. A fast stream
///   keeps several chunks fading at once (a rolling veil); a chunk fades exactly once, and
///   text already faded never animates again.
/// - The fade's length follows the stream's cadence: an EMA of the gaps between flushes,
///   `duration = clamp(ema × 3, 120ms, 400ms)`, so a fast stream dissolves quickly and a
///   slow one lingers. Three or more chunks in flight speed each other up by 30% a chunk.
/// - The dissolve eases as `(1 − p)^1.6`, so the text shows through at `1 − veil`.
/// - Only opacity changes, and only the paint: every consumer keeps its layout byte for byte.
/// - What is already on screen when a veil attaches (a row scrolled back into view, a thread
///   switched back to mid-reply) is adopted as the baseline and never fades; only later
///   appends do. A rewrite that is not an append (the parser re-deriving a block once
///   `**bold` closes) keeps the common prefix's fades and re-veils only the changed tail.
///
/// Offsets are in characters of the plain text the consumer hands over, so a SwiftUI
/// `AttributedString` and an AppKit text storage each map them into their own ranges.
@MainActor
final class StreamVeil {
    struct Span: Equatable {
        let range: Range<Int>
        /// The text's opacity over `range`, 0 to 1.
        let alpha: Double
    }

    private struct Chunk {
        var range: Range<Int>
        let started: Date
        /// Fixed at arrival from the cadence in force then.
        let duration: TimeInterval
    }

    static let shortestFade: TimeInterval = 0.12
    static let longestFade: TimeInterval = 0.40
    private static let curve = 1.6
    private static let cadenceSeed: TimeInterval = 0.16
    private static let gapCeiling: TimeInterval = 1.0

    /// The text as last handed over, nil until a veil attaches.
    private var previous: [Character]?
    private var chunks: [Chunk] = []
    private var cadence = cadenceSeed
    private var lastAppend: Date?

    /// Registers whatever `text` appends to the last text seen and drops chunks that have
    /// settled. The first call adopts the text as the baseline without fading it. Idempotent
    /// for unchanged text, so a body may call it on every evaluation.
    func advance(_ text: String, at now: Date) {
        let characters = Array(text)
        guard let previous else {
            self.previous = characters
            return
        }
        if characters != previous {
            let prefix = Self.commonPrefix(previous, characters)
            // A rewrite shorter than a chunk's start (a delegation block leaving the reply,
            // say) drops that chunk whole: a range built with its upper bound below its
            // lower one traps.
            chunks = chunks.compactMap { chunk in
                let upper = min(chunk.range.upperBound, prefix)
                guard upper > chunk.range.lowerBound else { return nil }
                var kept = chunk
                kept.range = chunk.range.lowerBound..<upper
                return kept
            }
            if characters.count > prefix {
                if let lastAppend {
                    let gap = min(now.timeIntervalSince(lastAppend), Self.gapCeiling)
                    cadence = cadence * 0.7 + gap * 0.3
                }
                lastAppend = now
                let duration = min(max(cadence * 3, Self.shortestFade), Self.longestFade)
                chunks.append(Chunk(range: prefix..<characters.count, started: now, duration: duration))
            }
            self.previous = characters
        }
        prune(at: now)
    }

    /// The chunks still dissolving at `now`, with the text's opacity over each.
    func spans(at now: Date) -> [Span] {
        prune(at: now)
        let boost = Self.boost(chunks.count)
        return chunks.map { chunk in
            let progress = min(1, max(0, now.timeIntervalSince(chunk.started) * boost / chunk.duration))
            return Span(range: chunk.range, alpha: 1 - pow(1 - progress, Self.curve))
        }
    }

    func isFading(at now: Date) -> Bool {
        prune(at: now)
        return !chunks.isEmpty
    }

    /// One frame of a consumer's update: feeds `text` while `active`, and says whether
    /// anything is still dissolving afterwards.
    func step(_ text: @autoclosure () -> String, active: Bool, at now: Date) -> Bool {
        if active { advance(text(), at: now) }
        return isFading(at: now)
    }

    /// When the last chunk in flight settles, so a frame clock knows when to stop.
    var settlesAt: Date? {
        let boost = Self.boost(chunks.count)
        return chunks.map { $0.started.addingTimeInterval($0.duration / boost) }.max()
    }

    /// Forgets the text and every chunk, so a settled paragraph holds nothing; the next
    /// `advance` attaches afresh.
    func reset() {
        previous = nil
        chunks = []
        lastAppend = nil
        cadence = Self.cadenceSeed
    }

    private func prune(at now: Date) {
        let boost = Self.boost(chunks.count)
        chunks.removeAll { now.timeIntervalSince($0.started) * boost >= $0.duration }
    }

    /// A backed-up stream (three or more chunks fading at once) dissolves faster.
    private static func boost(_ active: Int) -> Double {
        1 + 0.3 * Double(max(0, active - 2))
    }

    private static func commonPrefix(_ a: [Character], _ b: [Character]) -> Int {
        let limit = min(a.count, b.count)
        var index = 0
        while index < limit, a[index] == b[index] { index += 1 }
        return index
    }
}

/// Whether the text under this view belongs to a reply that is still arriving, so its
/// paragraphs veil what streams in. Set once per message, on every block of it, so a
/// paragraph that stops being the last block keeps dissolving instead of snapping.
private struct MarkdownVeiledKey: EnvironmentKey {
    static let defaultValue = false
}

extension EnvironmentValues {
    var markdownVeiled: Bool {
        get { self[MarkdownVeiledKey.self] }
        set { self[MarkdownVeiledKey.self] = newValue }
    }
}

/// One display-rate clock for every veil on screen. A veiled `Text` reads `now` only
/// while it dissolves, so each tick re-renders exactly the paragraphs still fading and
/// nothing else; the link is torn down the moment the last of them has settled. The
/// `Text` itself stays a bare `Text` in its stack (no `TimelineView` around it), so a list
/// marker keeps its first-baseline alignment with the paragraph mid-stream.
@MainActor
@Observable
final class VeilClock {
    static let shared = VeilClock()

    private(set) var now = Date.now
    @ObservationIgnored private var link: CADisplayLink?
    @ObservationIgnored private var deadline: Date?

    /// Keeps the clock ticking until `date`, or later if it already runs past it.
    func run(until date: Date) {
        deadline = max(deadline ?? date, date)
        guard link == nil, let screen = NSScreen.main else { return }
        let link = screen.displayLink(target: self, selector: #selector(tick))
        // A veil is a short opacity ramp, not motion: half the display's rate is
        // indistinguishable here and halves the per-frame `Text` rebuilds for every
        // paragraph of a reply that is still dissolving.
        link.preferredFrameRateRange = CAFrameRateRange(minimum: 24, maximum: 60, preferred: 30)
        link.add(to: .main, forMode: .common)
        self.link = link
    }

    @objc private func tick() {
        // The clock holds its last frame through a scroll, so the veiled paragraphs are
        // not rebuilt for frames the reader never sees; the ramp resumes when the scroll
        // ends.
        guard !ScrollActivity.shared.isScrolling else { return }
        now = .now
        guard let deadline, now >= deadline else { return }
        link?.invalidate()
        link = nil
        self.deadline = nil
    }
}

/// A `Text` whose newly streamed characters fade in through a `StreamVeil`. While the
/// reply streams (and until the last chunk has settled after it) the veil clock repaints
/// the text with the veil's opacities multiplied into the foreground colour of the
/// arriving runs; a finished paragraph is one plain `Text` again and holds no veil at all.
struct VeiledText: View {
    /// Prose comes styled; code comes as the string it is, and is only made attributed
    /// for the frames that veil it, so a finished dump costs a hover nothing new.
    private enum Source {
        case attributed(AttributedString)
        case plain(String)

        var characters: String {
            switch self {
            case .attributed(let attributed): String(attributed.characters)
            case .plain(let string): string
            }
        }

        var styled: AttributedString {
            switch self {
            case .attributed(let attributed): attributed
            case .plain(let string): AttributedString(string)
            }
        }

        var text: Text {
            switch self {
            case .attributed(let attributed): Text(attributed)
            case .plain(let string): Text(verbatim: string)
            }
        }
    }

    private let source: Source
    enum Segment {
        case prose(AttributedString)
        case fixed(Text)
    }
    /// Fixed runs (hand-name glyph + coloured name) interleaved with the veiled prose,
    /// in order: every hand named anywhere in a paragraph renders decorated, inline in
    /// the same flowing `Text`, and only the prose runs veil.
    private let segments: [Segment]?
    /// Where each prose run ends, in characters of the JOINED string's own clustering:
    /// a grapheme that spans a join (a combining mark, a ZWJ emoji) is one character of
    /// the join but two of the runs' own counts, so slicing by those traps past endIndex.
    let proseEnds: [Int]

    @Environment(\.markdownVeiled) private var veiled
    @Environment(\.markdownDimmed) private var dimmed
    @State private var veil = StreamVeil()

    init(_ attributed: AttributedString) {
        source = .attributed(attributed)
        segments = nil
        proseEnds = []
    }

    init(segments: [Segment]) {
        var joined = AttributedString()
        var ends: [Int] = []
        for segment in segments {
            if case .prose(let a) = segment {
                joined.append(a)
                ends.append(joined.characters.count)
            }
        }
        source = .attributed(joined)
        self.segments = segments
        proseEnds = ends
    }

    init(verbatim string: String) {
        source = .plain(string)
        segments = nil
        proseEnds = []
    }

    var body: some View {
        if veil.step(source.characters, active: veiled, at: .now) {
            fadingText()
        } else {
            settledText()
        }
    }

    /// This frame of the dissolve, and the clock kept running to the last chunk's settle.
    /// Reading the clock is what re-renders this text on the next tick; the frame itself
    /// is painted at the real time, so the first frame after a flush is never a stale one.
    private func fadingText() -> Text {
        let clock = VeilClock.shared
        if let settlesAt = veil.settlesAt { clock.run(until: settlesAt) }
        _ = clock.now
        return text(at: .now)
    }

    /// The plain text. Once the reply has finished the veil is emptied too, so a settled
    /// paragraph keeps no copy of itself; while it still streams the baseline stays, or a
    /// quiet beat between two flushes would make the next one arrive unveiled.
    private func settledText() -> Text {
        if !veiled { veil.reset() }
        if let segments {
            var out = Text(verbatim: "")
            for segment in segments {
                switch segment {
                case .fixed(let t): out = out + t
                case .prose(let a): out = out + Text(a)
                }
            }
            return out
        }
        return source.text
    }

    private func text(at now: Date) -> Text {
        var styled = source.styled
        let base = dimmed ? Color.secondary : Color.primary
        let characters = styled.characters
        let count = characters.count
        for span in veil.spans(at: now) where span.alpha < 1 {
            let range = span.range.clamped(to: 0..<count)
            guard !range.isEmpty else { continue }
            let lower = characters.index(characters.startIndex, offsetBy: range.lowerBound)
            let upper = characters.index(lower, offsetBy: range.count)
            styled[lower..<upper].foregroundColor = base.opacity(span.alpha)
        }
        if let segments {
            var out = Text(verbatim: "")
            var cursor = styled.startIndex
            var prose = 0
            for segment in segments {
                switch segment {
                case .fixed(let t): out = out + t
                case .prose:
                    let end = styled.characters.index(styled.startIndex, offsetBy: proseEnds[prose])
                    out = out + Text(AttributedString(styled[cursor..<end]))
                    cursor = end
                    prose += 1
                }
            }
            return out
        }
        return Text(styled)
    }
}

/// The AppKit side of the veil, for `LinkTextView`: the same chunks, painted as temporary
/// foreground colours on the layout manager, which change no glyph and no line. A display
/// link repaints while anything is dissolving and is torn down the moment nothing is.
@MainActor
final class LinkTextVeil {
    private let veil = StreamVeil()
    private var displayLink: CADisplayLink?
    /// The widest character range temporary colours were laid over, cleared before each
    /// repaint and once the veil settles.
    private var painted = NSRange(location: 0, length: 0)

    /// Feeds the text view's current text. `active` while the reply streams; afterwards
    /// the chunks in flight finish and the veil forgets the text.
    func update(_ view: NSTextView, active: Bool) {
        let now = Date.now
        if veil.step(view.string, active: active, at: now) {
            paint(view, at: now)
            if displayLink == nil {
                let link = view.displayLink(target: self, selector: #selector(tick))
                link.add(to: .main, forMode: .common)
                displayLink = link
            }
            tickTarget = view
        } else {
            settle(view, keepText: active)
        }
    }

    /// Stops the clock; the view is leaving its window.
    func detach(_ view: NSTextView) {
        settle(view, keepText: false)
    }

    private weak var tickTarget: NSTextView?

    @objc private func tick() {
        // The clock holds its last frame through a scroll, so the layout-manager repaint
        // does not run for frames the reader never sees; the ramp resumes when the scroll
        // ends.
        guard !ScrollActivity.shared.isScrolling else { return }
        guard let view = tickTarget else {
            displayLink?.invalidate()
            displayLink = nil
            return
        }
        let now = Date.now
        if veil.isFading(at: now) {
            paint(view, at: now)
        } else {
            settle(view, keepText: true)
        }
    }

    private func settle(_ view: NSTextView, keepText: Bool) {
        displayLink?.invalidate()
        displayLink = nil
        clear(view)
        if !keepText { veil.reset() }
    }

    private func clear(_ view: NSTextView) {
        guard painted.length > 0, let layout = view.layoutManager else { return }
        let length = view.textStorage?.length ?? 0
        let range = NSIntersectionRange(painted, NSRange(location: 0, length: length))
        if range.length > 0 { layout.removeTemporaryAttribute(.foregroundColor, forCharacterRange: range) }
        painted = NSRange(location: 0, length: 0)
    }

    private func paint(_ view: NSTextView, at now: Date) {
        guard let layout = view.layoutManager, let storage = view.textStorage else { return }
        clear(view)
        let string = view.string
        let accent = Chrome.accentNSColor
        for span in veil.spans(at: now) where span.alpha < 1 {
            let range = span.range.clamped(to: 0..<string.count)
            guard !range.isEmpty else { continue }
            let lower = string.index(string.startIndex, offsetBy: range.lowerBound)
            let upper = string.index(lower, offsetBy: range.count)
            let nsRange = NSRange(lower..<upper, in: string)
            // Each run keeps its own colour, only thinner: prose its label colour, a link
            // the accent it is drawn in.
            storage.enumerateAttributes(in: nsRange) { attributes, runRange, _ in
                let color: NSColor = attributes[.link] != nil
                    ? accent
                    : (attributes[.foregroundColor] as? NSColor ?? .labelColor)
                layout.addTemporaryAttribute(.foregroundColor, value: color.withAlphaComponent(span.alpha), forCharacterRange: runRange)
            }
            painted = painted.length == 0 ? nsRange : NSUnionRange(painted, nsRange)
        }
    }
}
