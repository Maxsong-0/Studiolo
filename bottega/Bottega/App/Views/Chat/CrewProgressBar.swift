// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import AppKit
import SwiftUI

/// How far a hand has got, as a course run along a compass tape: a stave for each thing it
/// did, in order, the taller the more it changed. A read, a search or a lookup is short and
/// faint, a command taller, an edit taller still, and a reply stands full height in the
/// hand's own colour. Ahead of them the tape is ruled in fine ticks, a longer one every
/// fifth, like the degree marks of a compass; at the frontier a small bow points on along
/// the course, in the hand's colour while it runs, red where it failed, grey where it was
/// stopped. Under it, what that amounts to; how long the hand has been at it sits in the
/// strip's pill (see `CrewElapsedTime`). The steps themselves are the sidebar's to show;
/// here the shape of the work is enough, and a stave under the pointer names the step it
/// stands for on a card above the bar, as the timeline rail previews the message under
/// the pointer.
///
/// The bar is one `Canvas`, redrawn on the display's clock while the hand works: a new
/// stave rises over 0.45 s from the moment its entry arrived, the bow gliding one slot on
/// with it, and a warm highlight sweeps the run stretch every few seconds. Once the hand
/// is done and its last stave is up, the clock stops and the bar is drawn once. A stave is
/// always one step: past what the width holds, the oldest steps slide off the left and
/// the bar shows the latest ones, so it always fits and keeps moving.
struct CrewProgressBar: View {
    let runtime: ThreadRuntime
    let startedAt: Date
    let finishedAt: Date?
    let status: CrewHandInfo.Status
    /// The hand's persona colour.
    let tint: Color
    /// The entries to draw, the runtime's whole list when nil.
    var entries: [TimelineEntry]? = nil

    @State private var width: CGFloat = 0
    /// The hand is done and its last stave has risen: nothing left to animate.
    @State private var isSettled = false
    /// A stave arrived within the last `rise` seconds and is still growing, the one time
    /// a bar without the sweep needs the display's clock.
    @State private var isRising = false
    /// The stave under the pointer, whose step the card names.
    @State private var hoveredIndex: Int?
    /// The last events, staves and counts, so an evaluation where nothing changed
    /// redraws without walking every entry again.
    @State private var cache = StaveCache()
    @Environment(\.isOnGlassPanel) private var isOnGlassPanel
    @Environment(\.colorScheme) private var colorScheme

    private static let height: CGFloat = 28
    private static let staveWidth: CGFloat = 3
    private static let gap: CGFloat = 3
    private static let pitch = staveWidth + gap
    /// The slots the bow keeps at the frontier, clear of staves and ticks alike.
    private static let bowSlots = 2
    /// The staves stand this far above the bar's bottom edge, on the tape's hairline.
    private static let baselineInset: CGFloat = 3
    /// The highlight that sweeps the run stretch: pale gold, warm on brass where white
    /// read as cold.
    private static let glint = Color(red: 1, green: 0.93, blue: 0.78)
    /// How long a new stave takes to rise to its height.
    private static let rise: TimeInterval = 0.45
    /// One pass of the highlight across the filled staves.
    private static let sweep: TimeInterval = 2.6
    /// The rest between two passes, so the bar mostly stands still.
    private static let sweepRest: TimeInterval = 3.4
    private static let cardWidth: CGFloat = 260
    /// How many bars sweep at once. Every sweeping bar is a canvas redrawn on the clock
    /// and a render pass on the GPU; with twenty hands out at once those passes alone
    /// saturated the main thread and the GPU queue and froze the app. The first few
    /// running hands keep the sweep, the rest stand still between staves, and a bar
    /// takes a slot as soon as one frees up.
    static let sweepSlots = 3

    /// The last events, staves and counts, keyed by entry identity plus each tool
    /// row's status, since a status flip rewrites the step's verb. A hit skips the
    /// walk over every entry, so the TimelineView ticks redraw without recomputing.
    @MainActor
    private final class StaveCache {
        private struct Key: Hashable {
            let id: ObjectIdentifier
            let status: ToolCall.Status?
            /// The title streams in for a row that has already arrived, and the step's
            /// label reads it.
            let title: String?
        }

        private var key: [Key] = []
        private var capacity = 0
        private var events: [Stave] = []
        private var staves: [Stave] = []
        private var counts = Counts(of: [])

        func resolve(entries: [TimelineEntry], capacity: Int) -> (events: [Stave], staves: [Stave], counts: Counts) {
            // Compared against the stored keys in place, not against an array built first:
            // the pointer crossing the staves re-evaluates this body once per slot, and
            // each of those passes would otherwise allocate one key per row of the hand's
            // whole timeline only to throw it away again.
            if capacity == self.capacity, matches(entries) {
                return (events, staves, counts)
            }
            self.key = entries.map(Self.key(of:))
            self.capacity = capacity
            let events = CrewProgressBar.events(in: entries)
            let staves = CrewProgressBar.staves(for: events, capacity: capacity)
            let counts = Counts(of: events)
            self.events = events
            self.staves = staves
            self.counts = counts
            return (events, staves, counts)
        }

        /// Whether `entries` is the list the keys were taken from, walked against them
        /// without allocating and out at the first row that differs: the same answer
        /// `key == self.key` gave, for a fraction of the work on a hit.
        private func matches(_ entries: [TimelineEntry]) -> Bool {
            guard entries.count == key.count else { return false }
            for (index, entry) in entries.enumerated() {
                let cached = key[index]
                guard ObjectIdentifier(entry) == cached.id else { return false }
                if entry.kind == .tool, case .tool(let call) = entry.item.content, call.status != cached.status || call.title != cached.title {
                    return false
                }
            }
            return true
        }

        private static func key(of entry: TimelineEntry) -> Key {
            guard entry.kind == .tool, case .tool(let call) = entry.item.content else {
                return Key(id: ObjectIdentifier(entry), status: nil, title: nil)
            }
            return Key(id: ObjectIdentifier(entry), status: call.status, title: call.title)
        }
    }

    var body: some View {
        // The bow keeps its slots at the frontier, so the staves stop short of them.
        let capacity = max(1, Int(((width + Self.gap) / Self.pitch).rounded(.down)) - Self.bowSlots)
        let resolved = cache.resolve(entries: entries ?? runtime.entries, capacity: capacity)
        let events = resolved.events
        let staves = resolved.staves
        let counts = resolved.counts
        let reduceMotion = NSWorkspace.shared.accessibilityDisplayShouldReduceMotion
        let isRunning = status == .running
        let elapsed = RelativeTime.duration((isRunning ? Date.now : finishedAt ?? .now).timeIntervalSince(startedAt))
        let sweeps = isRunning && !reduceMotion && SweepBudget.shared.sweeps(ObjectIdentifier(runtime))
        // The clock runs only while something moves: a stave rising, or the sweep on the
        // bars that hold one of its slots. A bar standing still is drawn once per change.
        let needsClock = !reduceMotion && !isSettled && (sweeps || isRising)
        VStack(alignment: .leading, spacing: 6) {
            // The display's clock drives the rise and the sweep at twenty-four frames a
            // second, enough for a sweep this soft; it keeps going while the reader
            // scrolls, so a hand at work never looks stalled. With reduced motion, or once
            // the hand is done and settled, the canvas is drawn only when the steps change.
            // Held still while the reader scrolls anywhere (see `ScrollActivity`): a fold
            // gliding the timeline gets its frames, the staves catch up when it lands.
            TimelineView(.animation(minimumInterval: 1.0 / 24.0, paused: !needsClock || ScrollActivity.shared.isScrolling)) { context in
                Canvas { graphics, size in
                    Self.draw(
                        staves,
                        in: &graphics,
                        size: size,
                        capacity: capacity,
                        at: context.date,
                        rising: !reduceMotion,
                        sweeping: sweeps,
                        status: status,
                        tint: tint,
                        hovered: hoveredIndex
                    )
                }
            }
            .frame(height: Self.height)
            .onGeometryChange(for: CGFloat.self) { $0.size.width } action: { width = $0 }
            // The pointer picks the stave whose slot it is over; the slots past the
            // filled ones name nothing.
            .onContinuousHover(coordinateSpace: .local) { phase in
                let index: Int? = switch phase {
                case .active(let point):
                    point.x >= 0 && point.y >= 0 && point.y <= Self.height
                        ? Int((point.x / Self.pitch).rounded(.down)) : nil
                case .ended: nil
                }
                let picked = index.map { $0 < staves.count ? $0 : nil } ?? nil
                if picked != hoveredIndex { hoveredIndex = picked }
            }
            .overlay(alignment: .topLeading) {
                if let hoveredIndex, hoveredIndex < staves.count {
                    stepsCard(for: staves[hoveredIndex], at: hoveredIndex)
                }
            }
            .animation(.smooth(duration: 0.15), value: hoveredIndex)
            if !counts.line.isEmpty {
                Text(verbatim: counts.line)
                    .zoomedFont(size: 11)
                    .foregroundStyle(Chrome.secondaryText)
                    .frame(maxWidth: .infinity)
            }
        }
        // A finished hand's last stave still needs its rise before the clock can stop.
        .task(id: isRunning ? -1 : events.count) {
            isSettled = false
            guard !isRunning else { return }
            guard (try? await Task.sleep(for: .seconds(Self.rise + 0.1))) != nil else { return }
            isSettled = true
        }
        // A new stave keeps the clock for its rise, then lets it go.
        .task(id: events.count) {
            guard !events.isEmpty, !reduceMotion else { return }
            isRising = true
            guard (try? await Task.sleep(for: .seconds(Self.rise + 0.1))) != nil else { return }
            isRising = false
        }
        // A running bar asks for a sweep slot; it gives the slot back when it stops or
        // leaves the screen, and the next bar in line takes it.
        .task(id: isRunning) {
            let id = ObjectIdentifier(runtime)
            guard isRunning else { SweepBudget.shared.leave(id); return }
            SweepBudget.shared.join(id)
        }
        .onDisappear { SweepBudget.shared.leave(ObjectIdentifier(runtime)) }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(Text(verbatim: "\(counts.sentence), \(elapsed)\(isRunning ? " so far" : "")"))
    }

    /// The step a stave stands for, on the rail's card. Sits just above the bar, centred on
    /// the stave as far as the bar's width allows, and never takes the pointer.
    private func stepsCard(for stave: Stave, at index: Int) -> some View {
        let centre = CGFloat(index) * Self.pitch + Self.staveWidth / 2
        let x = min(max(0, centre - Self.cardWidth / 2), max(0, width - Self.cardWidth))
        return HStack(spacing: 8) {
            if let entry = stave.mcp {
                MCPGlyph(entry: entry, size: 14)
            } else if let name = stave.symbol {
                Image(systemName: name)
                    .zoomedFont(size: 12, weight: .semibold)
                    .foregroundStyle(Chrome.secondaryText)
                    .frame(width: 14)
            }
            Text(verbatim: stave.step)
                .zoomedFont(size: 13, weight: .semibold)
                .foregroundStyle(Chrome.primaryText)
                .lineLimit(1)
                .truncationMode(.tail)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 10)
        .frame(width: Self.cardWidth, alignment: .leading)
        .modifier(ProgressCardSurface(isOnGlassPanel: isOnGlassPanel, isDark: colorScheme == .dark))
        // Hung from a zero-height frame at the bar's top edge, so the card's bottom sits 8
        // points above the bar whatever its height: an alignment guide on the overlay
        // was not honoured and left the card over the staves.
        .padding(.bottom, 8)
        .frame(height: 0, alignment: .bottom)
        .offset(x: x)
        .allowsHitTesting(false)
        .transition(.opacity)
    }

    // MARK: - Steps

    /// One thing the hand did, as the bar shows it.
    private struct Stave {
        enum Kind: Hashable {
            /// A read, a search, the web, an MCP tool, a helper: looking, not changing.
            case lookup
            case command
            case edit
            case reply
        }

        let kind: Kind
        let arrivedAt: Date
        /// What the stave stands for, for the card under the pointer.
        let step: String

        /// The step's own mark, where it has one: an MCP server's brand icon, or the
        /// kind's SF Symbol. Nil never happens; a reply wears a bubble.
        let mcp: MCPCatalogEntry?
        let symbol: String?

        /// The stave's share of the bar's height.
        var height: CGFloat {
            switch kind {
            case .lookup: 0.45
            case .command: 0.65
            case .edit: 0.85
            case .reply: 1
            }
        }
    }

    /// What the bar counts, for the line under it and the accessibility label.
    private struct Counts {
        var edits = 0
        var commands = 0
        var reads = 0

        init(of events: [Stave]) {
            for event in events {
                switch event.kind {
                case .edit: edits += 1
                case .command: commands += 1
                case .lookup: reads += 1
                case .reply: break
                }
            }
        }

        private var parts: [String] {
            var parts: [String] = []
            if edits > 0 { parts.append("\(edits) \(edits == 1 ? "edit" : "edits")") }
            if commands > 0 { parts.append("\(commands) \(commands == 1 ? "command" : "commands")") }
            if reads > 0 { parts.append("\(reads) \(reads == 1 ? "read" : "reads")") }
            return parts
        }

        /// "3 edits · 2 commands · 5 reads", the zero parts left out.
        var line: String { parts.joined(separator: " · ") }

        var sentence: String {
            parts.isEmpty ? "Nothing done yet" : parts.joined(separator: ", ")
        }
    }

    /// One stave per tool call and per reply, in the order they happened.
    private static func events(in entries: [TimelineEntry]) -> [Stave] {
        entries.compactMap { entry in
            switch entry.kind {
            case .tool:
                guard case .tool(let call) = entry.item.content else { return nil }
                // A command that wrote files (a heredoc, a script over the source) is an
                // edit, the way its row already counts it, not one more command.
                let kind: Stave.Kind = switch call.kind {
                case .edit: .edit
                case .command: call.edits.isEmpty ? .command : .edit
                default: call.edits.isEmpty ? .lookup : .edit
                }
                let mcp = ToolPresentation.mcpParts(call)?.entry
                let symbol = mcp == nil ? ToolPresentation.symbol(for: call.kind) : nil
                return Stave(kind: kind, arrivedAt: entry.item.date, step: ToolPresentation.label(for: call), mcp: mcp, symbol: symbol)
            case .assistant:
                guard case .assistant(let message) = entry.item.content else { return nil }
                // The first line of the reply is all the stave shows, so only the head of the
                // text is split, not the whole reply on every streamed flush.
                let words = TextCleanup.singleLine(String(message.text.prefix(600)), limit: 60)
                return Stave(kind: .reply, arrivedAt: entry.item.date, step: words.isEmpty ? "Replied" : "Replied: \(words)", mcp: nil, symbol: "bubble.left")
            default:
                return nil
            }
        }
    }

    /// The staves the bar draws, one per event: all of them while they fit, else the
    /// latest `capacity` of them, the oldest having slid off the left.
    private static func staves(for events: [Stave], capacity: Int) -> [Stave] {
        events.count > capacity ? Array(events.suffix(capacity)) : events
    }

    // MARK: - Drawing

    private static func color(for kind: Stave.Kind, tint: Color) -> Color {
        switch kind {
        case .edit, .reply: tint
        case .command: tint.opacity(0.7)
        case .lookup: tint.opacity(0.4)
        }
    }

    /// One pass over the staves: each filled from the baseline up, the newest still rising,
    /// faint slots where the bar has yet to fill, and the highlight over what is filled.
    private static func draw(
        _ staves: [Stave],
        in context: inout GraphicsContext,
        size: CGSize,
        capacity: Int,
        at date: Date,
        rising: Bool,
        sweeping: Bool,
        status: CrewHandInfo.Status,
        tint: Color,
        hovered: Int? = nil
    ) {
        let bottom = size.height
        let baseline = bottom - baselineInset
        // The tape's edge: a hairline under the whole bar; the stretch already run gets
        // one in the hand's colour further down, once the frontier is known.
        context.fill(
            Path(CGRect(x: 0, y: bottom - 1, width: size.width, height: 1)),
            with: .color(Chrome.overlay(0.1))
        )
        var filled = Path()
        // One fill per colour, not per stave: every fill resolves its colour through
        // AppKit's dynamic colour system, and a hundred staves at thirty frames a second
        // on twenty bars was a large share of the main thread. The staves of a kind go
        // into one path and are filled together; only the one under the pointer is
        // filled on its own.
        var byKind: [Stave.Kind: Path] = [:]
        var hoveredPath: Path?
        // How far the newest stave has risen; the bow glides on with it.
        var newest: CGFloat = 1
        for (index, stave) in staves.enumerated() {
            var factor: CGFloat = 1
            if rising {
                let age = date.timeIntervalSince(stave.arrivedAt)
                if age < rise {
                    // Eased out, like the smooth curve the rest of the panel moves on.
                    let t = max(0, age / rise)
                    factor = 1 - pow(1 - t, 3)
                }
            }
            if index == staves.count - 1 { newest = factor }
            let height = baseline * stave.height * factor
            guard height > 0 else { continue }
            let rect = CGRect(x: CGFloat(index) * pitch, y: baseline - height, width: staveWidth, height: height)
            let path = Path(roundedRect: rect, cornerRadius: staveWidth / 2)
            byKind[stave.kind, default: Path()].addPath(path)
            // The stave under the pointer lifts towards white, so the card reads as its.
            if index == hovered { hoveredPath = path }
            filled.addPath(path)
        }
        for kind in [Stave.Kind.lookup, .command, .edit, .reply] {
            guard let path = byKind[kind] else { continue }
            context.fill(path, with: .color(color(for: kind, tint: tint)))
        }
        if let hoveredPath { context.fill(hoveredPath, with: .color(.white.opacity(0.4))) }

        // The bow at the frontier, pointing on along the course: the hand's colour while
        // it runs, red where it failed, grey where it was stopped, dimmed once it is home.
        // It sits a slot back while the newest stave rises, and glides up with it.
        let frontier = (CGFloat(staves.count) - (1 - newest)) * pitch
        let bowWidth = CGFloat(bowSlots) * pitch - gap
        let bowHeight: CGFloat = 10
        let bowMid = baseline / 2
        var bow = Path()
        bow.move(to: CGPoint(x: frontier, y: bowMid - bowHeight / 2))
        bow.addLine(to: CGPoint(x: frontier + bowWidth, y: bowMid))
        bow.addLine(to: CGPoint(x: frontier, y: bowMid + bowHeight / 2))
        bow.closeSubpath()
        let bowColor: Color = switch status {
        case .running: tint
        case .completed: tint.opacity(0.6)
        case .failed: Chrome.danger
        case .stopped: Chrome.secondaryText
        }
        context.fill(bow, with: .color(bowColor))
        filled.addPath(bow)
        if frontier > 0 {
            context.fill(
                Path(CGRect(x: 0, y: bottom - 1, width: frontier, height: 1)),
                with: .color(tint.opacity(0.45))
            )
        }

        // The tape ahead, ruled in the slots past the bow: a fine tick in each, a longer
        // one every fifth, like the degree marks of a compass.
        var minor = Path()
        var major = Path()
        let corner = CGSize(width: staveWidth / 2, height: staveWidth / 2)
        for slot in (staves.count + bowSlots)..<(capacity + bowSlots) {
            let tall = slot % 5 == 0
            let height = baseline * (tall ? 0.5 : 0.28)
            let rect = CGRect(x: CGFloat(slot) * pitch, y: baseline - height, width: staveWidth, height: height)
            if tall {
                major.addRoundedRect(in: rect, cornerSize: corner)
            } else {
                minor.addRoundedRect(in: rect, cornerSize: corner)
            }
        }
        context.fill(minor, with: .color(Chrome.overlay(0.06)))
        context.fill(major, with: .color(Chrome.overlay(0.12)))

        guard sweeping else { return }
        // A warm band crossing the run stretch, bow and all, left to right, once every
        // sweep; clipped to what is filled, so the tape and the gaps stay as they are.
        let filledWidth = frontier + bowWidth
        let band = max(24, filledWidth / 3)
        // One pass, then a rest: the band crosses in `sweep` seconds and the bar holds
        // still for `sweepRest` before the next.
        let within = date.timeIntervalSinceReferenceDate.truncatingRemainder(dividingBy: sweep + sweepRest)
        guard within < sweep else { return }
        let phase = CGFloat(within / sweep)
        let x = -band + phase * (filledWidth + band)
        var highlight = context
        highlight.clip(to: filled)
        highlight.fill(
            Path(CGRect(x: x, y: 0, width: band, height: bottom)),
            with: .linearGradient(
                Gradient(colors: [glint.opacity(0), glint.opacity(0.55), glint.opacity(0)]),
                startPoint: CGPoint(x: x, y: 0),
                endPoint: CGPoint(x: x + band, y: 0)
            )
        )
    }
}

/// A hand's panel with "Show what hands are doing" off: what it was sent to do, centred on
/// one row, and the progress bar under it, centred where the transcript would be. How long
/// the hand has been at it sits in the strip's pill beside its name.
struct CrewHandProgress: View {
    let hand: ChatThread
    let runtime: ThreadRuntime

    var body: some View {
        let info = hand.crew
        let task = info?.task.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let status = info?.status ?? .running
        let startedAt = info?.startedAt ?? hand.createdAt
        VStack(spacing: 0) {
            Spacer(minLength: 0)
            CrewWorkingTitle(text: task.isEmpty ? hand.title : task, isRunning: status == .running)
                .frame(maxWidth: .infinity)
            CrewProgressBar(
                runtime: runtime,
                startedAt: startedAt,
                finishedAt: info?.finishedAt,
                status: status,
                tint: (info?.persona ?? CrewRoster.persona(at: 0)).color
            )
            .padding(.top, 10)
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 20)
        // The strip overlays the top of the panel and the chat box is an inset at the
        // bottom: the room under the strip is what the task and the bar centre in.
        .padding(.top, FloatingPanelMetrics.stripHeight)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

/// The hand's task. While the hand works the text breathes, and a slightly darker band
/// drifts across it from left to right, so a bar with nothing on it yet still reads as a
/// hand at work. Done, or with reduced motion, it is plain text.
struct CrewWorkingTitle: View {
    let text: String
    let isRunning: Bool
    var alignment: TextAlignment = .center
    /// The window's text size, for the title's font: it is built once as a `Text` below.
    @Environment(\.chatZoom) private var zoom

    /// One pass of the band across the text.
    private static let sweep: TimeInterval = 3.2
    /// The rest between two passes of the band.
    private static let sweepRest: TimeInterval = 3.0
    /// One breath, in and out.
    private static let breath: TimeInterval = 3.2
    /// Half the band's width, as a share of the text's width.
    private static let reach = 0.3

    var body: some View {
        let animates = isRunning && !NSWorkspace.shared.accessibilityDisplayShouldReduceMotion
        // The text itself never changes per frame, only the band over it, so it is
        // built once and the closure below just restyles it.
        let title = Text(verbatim: text).font(.zoomed(size: 13, weight: .medium, zoom: zoom))
        // Twenty frames a second is plenty for a band this wide; it rests while the reader
        // scrolls anywhere (see `ScrollActivity`), so a fold's glide has the frames.
        TimelineView(.animation(minimumInterval: 1.0 / 20.0, paused: !animates || ScrollActivity.shared.isScrolling)) { context in
            let now = context.date.timeIntervalSinceReferenceDate
            // The band's centre travels from just left of the text to just right of it, so
            // it enters and leaves rather than snapping; then it rests off the right edge,
            // where the text is one colour.
            let phase = min(now.truncatingRemainder(dividingBy: Self.sweep + Self.sweepRest) / Self.sweep, 1)
            let centre = animates ? -Self.reach + phase * (1 + 2 * Self.reach) : 0.5
            let dip = animates ? 0.6 : 1.0
            let breath = animates ? 0.5 + 0.5 * sin(now / Self.breath * 2 * .pi) : 1.0
            title
                .foregroundStyle(
                    LinearGradient(
                        stops: [
                            .init(color: Chrome.primaryText, location: 0),
                            .init(color: Chrome.primaryText, location: Self.clamped(centre - Self.reach)),
                            .init(color: Chrome.primaryText.opacity(dip), location: Self.clamped(centre)),
                            .init(color: Chrome.primaryText, location: Self.clamped(centre + Self.reach)),
                            .init(color: Chrome.primaryText, location: 1),
                        ],
                        startPoint: .leading,
                        endPoint: .trailing
                    )
                )
                .opacity(0.86 + 0.14 * breath)
        }
        .lineLimit(2)
        .multilineTextAlignment(alignment)
    }

    private static func clamped(_ location: Double) -> CGFloat {
        CGFloat(min(1, max(0, location)))
    }
}

/// How long the hand has been at it, ticking every second while it works. Only this text
/// follows the clock: whatever holds it is drawn once.
struct CrewElapsedTime: View {
    let startedAt: Date
    let finishedAt: Date?
    let isRunning: Bool

    var body: some View {
        Group {
            if isRunning {
                TimelineView(.periodic(from: startedAt, by: 1)) { context in
                    Text(verbatim: RelativeTime.duration(context.date.timeIntervalSince(startedAt)))
                }
            } else {
                Text(verbatim: RelativeTime.duration((finishedAt ?? .now).timeIntervalSince(startedAt)))
            }
        }
        .zoomedFont(size: 11)
        .monospacedDigit()
        .foregroundStyle(Chrome.secondaryText)
    }
}

/// The card's surface: glass, unless it floats inside a glass panel already, where a
/// second glass over the first would sample the same pixels twice; there it is a flat
/// fill of the panel's control colour.
private struct ProgressCardSurface: ViewModifier {
    let isOnGlassPanel: Bool
    let isDark: Bool

    func body(content: Content) -> some View {
        let shape = RoundedRectangle(cornerRadius: Chrome.cardCornerRadius, style: .continuous)
        if isOnGlassPanel {
            content.background(shape.fill(Chrome.panelControlFill(isDark: isDark)))
        } else {
            content.glassEffect(.regular, in: shape)
        }
    }
}

/// Which bars sweep: the first `CrewProgressBar.sweepSlots` running bars to ask, in the
/// order they asked. Observed, so a bar next in line redraws with the sweep the moment a
/// slot frees up, and a bar losing its slot stops its clock on the same change.
@MainActor
@Observable
final class SweepBudget {
    static let shared = SweepBudget()
    private(set) var sweepers: [ObjectIdentifier] = []

    func join(_ id: ObjectIdentifier) {
        guard !sweepers.contains(id) else { return }
        sweepers.append(id)
    }

    func leave(_ id: ObjectIdentifier) {
        guard let index = sweepers.firstIndex(of: id) else { return }
        sweepers.remove(at: index)
    }

    func sweeps(_ id: ObjectIdentifier) -> Bool {
        sweepers.prefix(CrewProgressBar.sweepSlots).contains(id)
    }
}
