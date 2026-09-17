import AppKit
import SwiftUI

/// How far a head has got, as one bar filling in from the left: a stave for each thing it
/// did, in order, the taller the more it changed. A read, a search or a lookup is short and
/// faint, a command taller, an edit taller still, and a reply stands full height in the
/// head's own colour. Under it, what that amounts to; how long the head has been at it
/// sits in the strip's pill (see `HydraElapsedTime`). The steps themselves are the
/// sidebar's to show; here the shape of the work is enough, and a stave under the pointer
/// names the steps it stands for on a card above the bar, as the timeline rail previews
/// the message under the pointer.
///
/// The staves are one `Canvas`, redrawn on the display's clock while the head works: a new
/// stave rises over 0.45 s from the moment its entry arrived, and a soft highlight sweeps
/// the filled staves every 1.8 s. Once the head is done and its last stave is up, the clock
/// stops and the bar is drawn once. Past what the width holds, consecutive steps share a
/// stave, the tallest of them setting its height, so the bar always fits and keeps filling.
struct HydraProgressBar: View {
    let runtime: ThreadRuntime
    let startedAt: Date
    let finishedAt: Date?
    let status: HydraHeadInfo.Status
    /// The head's persona colour.
    let tint: Color
    /// The entries to draw, the runtime's whole list when nil.
    var entries: [TimelineEntry]? = nil

    @State private var width: CGFloat = 0
    /// The head is done and its last stave has risen: nothing left to animate.
    @State private var isSettled = false
    /// The stave under the pointer, whose steps the card names.
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
    /// How long a new stave takes to rise to its height.
    private static let rise: TimeInterval = 0.45
    /// One pass of the highlight across the filled staves.
    private static let sweep: TimeInterval = 2.6
    /// The rest between two passes, so the bar mostly stands still.
    private static let sweepRest: TimeInterval = 3.4
    private static let cardWidth: CGFloat = 260
    /// How many of a stave's steps the card lists before "and N more": few enough that
    /// the card clears the strip of a compact panel, where the bar sits under the task.
    private static let cardLines = 4

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
            let key = entries.map { entry -> Key in
                guard entry.kind == .tool, case .tool(let call) = entry.item.content else {
                    return Key(id: ObjectIdentifier(entry), status: nil, title: nil)
                }
                return Key(id: ObjectIdentifier(entry), status: call.status, title: call.title)
            }
            if key == self.key, capacity == self.capacity {
                return (events, staves, counts)
            }
            let events = HydraProgressBar.events(in: entries)
            let staves = HydraProgressBar.staves(for: events, capacity: capacity)
            let counts = Counts(of: events)
            self.key = key
            self.capacity = capacity
            self.events = events
            self.staves = staves
            self.counts = counts
            return (events, staves, counts)
        }
    }

    var body: some View {
        let capacity = max(1, Int(((width + Self.gap) / Self.pitch).rounded(.down)))
        let resolved = cache.resolve(entries: entries ?? runtime.entries, capacity: capacity)
        let events = resolved.events
        let staves = resolved.staves
        let counts = resolved.counts
        let reduceMotion = NSWorkspace.shared.accessibilityDisplayShouldReduceMotion
        let isRunning = status == .running
        let elapsed = RelativeTime.duration((isRunning ? Date.now : finishedAt ?? .now).timeIntervalSince(startedAt))
        VStack(alignment: .leading, spacing: 6) {
            // The display's clock drives the rise and the sweep at thirty frames a second,
            // enough for a sweep this soft; it keeps going while the reader scrolls, so a
            // head at work never looks stalled. With reduced motion, or once the head is
            // done and settled, the canvas is drawn only when the steps change.
            // Held still while the reader scrolls anywhere (see `ScrollActivity`): a fold
            // gliding the timeline gets its frames, the staves catch up when it lands.
            TimelineView(.animation(minimumInterval: 1.0 / 30.0, paused: reduceMotion || isSettled || ScrollActivity.shared.isScrolling)) { context in
                Canvas { graphics, size in
                    Self.draw(
                        staves,
                        in: &graphics,
                        size: size,
                        capacity: capacity,
                        at: context.date,
                        rising: !reduceMotion,
                        sweeping: isRunning && !reduceMotion,
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
                    .font(.system(size: 11))
                    .foregroundStyle(Chrome.secondaryText)
                    .frame(maxWidth: .infinity)
            }
        }
        // A finished head's last stave still needs its rise before the clock can stop.
        .task(id: isRunning ? -1 : events.count) {
            isSettled = false
            guard !isRunning else { return }
            guard (try? await Task.sleep(for: .seconds(Self.rise + 0.1))) != nil else { return }
            isSettled = true
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(Text(verbatim: "\(counts.sentence), \(elapsed)\(isRunning ? " so far" : "")"))
    }

    /// The steps a stave stands for, on the rail's card: one step is the title on its own,
    /// several are counted with the first few listed. Sits just above the bar, centred on
    /// the stave as far as the bar's width allows, and never takes the pointer.
    private func stepsCard(for stave: Stave, at index: Int) -> some View {
        let steps = stave.steps
        let listed = steps.count > 1 ? Array(steps.prefix(Self.cardLines)) : []
        let more = steps.count - listed.count
        let centre = CGFloat(index) * Self.pitch + Self.staveWidth / 2
        let x = min(max(0, centre - Self.cardWidth / 2), max(0, width - Self.cardWidth))
        return VStack(alignment: .leading, spacing: 3) {
            Text(verbatim: steps.count == 1 ? steps[0] : "\(steps.count) steps")
                .font(.system(size: 13, weight: .semibold))
                .foregroundStyle(Chrome.primaryText)
                .lineLimit(1)
                .truncationMode(.tail)
            ForEach(Array(listed.enumerated()), id: \.offset) { _, step in
                Text(verbatim: step)
                    .font(.system(size: 12))
                    .foregroundStyle(Chrome.secondaryText)
                    .lineLimit(1)
                    .truncationMode(.tail)
            }
            if listed.count < steps.count, more > 0 {
                Text(verbatim: "and \(more) more")
                    .font(.system(size: 12))
                    .foregroundStyle(Chrome.secondaryText)
            }
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

    /// One thing the head did, as the bar shows it.
    private struct Stave {
        enum Kind {
            /// A read, a search, the web, an MCP tool, a helper: looking, not changing.
            case lookup
            case command
            case edit
            case reply
        }

        let kind: Kind
        let arrivedAt: Date
        /// What the stave stands for, one line per step, for the card under the pointer.
        var steps: [String]

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
                return Stave(kind: kind, arrivedAt: entry.item.date, steps: [ToolPresentation.label(for: call)])
            case .assistant:
                guard case .assistant(let message) = entry.item.content else { return nil }
                // The first line of the reply is all the stave shows, so only the head of the
                // text is split, not the whole reply on every streamed flush.
                let words = TextCleanup.singleLine(String(message.text.prefix(600)), limit: 60)
                return Stave(kind: .reply, arrivedAt: entry.item.date, steps: [words.isEmpty ? "Replied" : "Replied: \(words)"])
            default:
                return nil
            }
        }
    }

    /// The staves the bar draws: the events themselves while they fit, else consecutive
    /// runs of them, each run's stave as tall as its tallest member. A run is as old as
    /// its first member, so its stave rises once, when the run opens, and only grows after.
    private static func staves(for events: [Stave], capacity: Int) -> [Stave] {
        guard events.count > capacity else { return events }
        let run = Int((Double(events.count) / Double(capacity)).rounded(.up))
        return stride(from: 0, to: events.count, by: run).map { start in
            let bucket = events[start..<min(start + run, events.count)]
            let tallest = bucket.max { $0.height < $1.height } ?? events[start]
            return Stave(kind: tallest.kind, arrivedAt: events[start].arrivedAt, steps: bucket.flatMap(\.steps))
        }
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
        status: HydraHeadInfo.Status,
        tint: Color,
        hovered: Int? = nil
    ) {
        let bottom = size.height
        var filled = Path()
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
            let height = bottom * stave.height * factor
            guard height > 0 else { continue }
            let rect = CGRect(x: CGFloat(index) * pitch, y: bottom - height, width: staveWidth, height: height)
            let path = Path(roundedRect: rect, cornerRadius: staveWidth / 2)
            // The last stave is where a head that failed or was stopped ended.
            let isLast = index == staves.count - 1
            let color: Color = if isLast, status == .failed {
                Chrome.danger
            } else if isLast, status == .stopped {
                Chrome.secondaryText
            } else {
                color(for: stave.kind, tint: tint)
            }
            context.fill(path, with: .color(color))
            // The stave under the pointer lifts towards white, so the card reads as its.
            if index == hovered { context.fill(path, with: .color(.white.opacity(0.4))) }
            filled.addPath(path)
        }

        if staves.count < capacity {
            let height = bottom * 0.45
            var slots = Path()
            for slot in staves.count..<capacity {
                slots.addRoundedRect(
                    in: CGRect(x: CGFloat(slot) * pitch, y: bottom - height, width: staveWidth, height: height),
                    cornerSize: CGSize(width: staveWidth / 2, height: staveWidth / 2)
                )
            }
            context.fill(slots, with: .color(Chrome.overlay(0.06)))
        }

        guard sweeping, !staves.isEmpty else { return }
        // A soft band crossing the filled staves, left to right, once every sweep; clipped
        // to the staves, so the slots and the gaps stay as they are.
        let filledWidth = CGFloat(staves.count) * pitch - gap
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
                Gradient(colors: [.white.opacity(0), .white.opacity(0.55), .white.opacity(0)]),
                startPoint: CGPoint(x: x, y: 0),
                endPoint: CGPoint(x: x + band, y: 0)
            )
        )
    }
}

/// A head's panel with "Show what heads are doing" off: what it was sent to do, centred on
/// one row, and the progress bar under it, centred where the transcript would be. How long
/// the head has been at it sits in the strip's pill beside its name.
struct HydraHeadProgress: View {
    let head: ChatThread
    let runtime: ThreadRuntime

    var body: some View {
        let info = head.hydra
        let task = info?.task.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let status = info?.status ?? .running
        let startedAt = info?.startedAt ?? head.createdAt
        VStack(spacing: 0) {
            Spacer(minLength: 0)
            HydraWorkingTitle(text: task.isEmpty ? head.title : task, isRunning: status == .running)
                .frame(maxWidth: .infinity)
            HydraProgressBar(
                runtime: runtime,
                startedAt: startedAt,
                finishedAt: info?.finishedAt,
                status: status,
                tint: (info?.persona ?? HydraRoster.persona(at: 0)).color
            )
            .padding(.top, 10)
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 20)
        // The strip overlays the top of the panel and the chat box is an inset at the
        // bottom: the room under the strip is what the task and the bar centre in.
        .padding(.top, HydraPanel.stripHeight)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

/// The head's task. While the head works the text breathes, and a slightly darker band
/// drifts across it from left to right, so a bar with nothing on it yet still reads as a
/// head at work. Done, or with reduced motion, it is plain text.
struct HydraWorkingTitle: View {
    let text: String
    let isRunning: Bool
    var alignment: TextAlignment = .center

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
        let title = Text(verbatim: text).font(.system(size: 13, weight: .medium))
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

/// How long the head has been at it, ticking every second while it works. Only this text
/// follows the clock: whatever holds it is drawn once.
struct HydraElapsedTime: View {
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
        .font(.system(size: 11))
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
