// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import AppKit
import SwiftUI

/// The deck: the lead and every hand of the team at once, one card each, down one
/// panel. A card is the hand's glyph, name, state and clock over its progress bar, then a
/// line or two of what it is doing right now: the tail of its thinking and its last
/// steps. A card opens in place to the hand's whole transcript, live; several can be
/// open at once, and the column scrolls. The lead's card comes first and shows its
/// running turn the same way, so the panel is the whole engine room at a glance.
///
/// "Every step" in Settings opens every card as it arrives; a card closed by hand stays
/// closed. Picking a hand in the strip's popover scrolls to its card and opens it (see
/// `ThreadRuntime.crewDeckReveal`). With `showsMetrics` the run's recorded usage and each
/// hand's phases head the column (see `CrewRunMetricsView`).
struct CrewDeck: View {
    @Environment(AppModel.self) private var model
    let runtime: ThreadRuntime
    let hands: [ChatThread]
    let workingDirectory: String?
    let projectName: String?
    var showsMetrics = false

    /// The cards open to their transcripts.
    @State private var open: Set<UUID> = []
    /// The cards that have been seeded from Settings, so a card closed by hand is not
    /// opened again on the next arrival.
    @State private var seeded: Set<UUID> = []

    static let cardSpacing: CGFloat = 8
    static let inset: CGFloat = 12
    /// An open transcript's height: enough rows to follow, never the whole column.
    static let transcriptHeight: CGFloat = 320

    var body: some View {
        ScrollViewReader { proxy in
            ScrollView(.vertical) {
                LazyVStack(spacing: Self.cardSpacing) {
                    if showsMetrics {
                        CrewRunMetricsView(runtime: runtime, hands: hands)
                            .transition(.opacity)
                    }
                    CrewLeadCard(runtime: runtime, workingDirectory: workingDirectory, isOpen: binding(for: runtime.threadID))
                        .id(runtime.threadID)
                    // Hands at work first, live; the finished ones after them, under a header
                    // that folds them away without taking them off the page.
                    ForEach(hands.filter { $0.crew?.isFinished != true }) { hand in
                        card(for: hand)
                    }
                    let finished = hands.filter { $0.crew?.isFinished == true }
                    if !finished.isEmpty {
                        CrewFinishedHeader(count: finished.count, isFolded: runtime.crewFinishedFolded) {
                            withAnimation(Chrome.panelSlide) { runtime.crewFinishedFolded.toggle() }
                        }
                        if !runtime.crewFinishedFolded {
                            ForEach(finished) { hand in
                                card(for: hand)
                            }
                        }
                    }
                }
                .padding(.horizontal, Self.inset)
                .padding(.bottom, 14)
            }
            // Only an explicit ask scrolls and opens a card: the model picks hands as they
            // arrive too, and following that would open every card and scroll the lead's
            // out of view.
            .onChange(of: runtime.crewDeckReveal) { _, id in
                guard let id else { return }
                runtime.crewDeckReveal = nil
                guard let hand = hands.first(where: { $0.id == id }) else { return }
                withAnimation(Chrome.panelSlide) {
                    // A finished hand picked while its group is folded: the group opens first.
                    if hand.crew?.isFinished == true { runtime.crewFinishedFolded = false }
                    open.insert(id)
                    proxy.scrollTo(id, anchor: .top)
                }
            }
        }
        .onChange(of: hands.map(\.id) + [runtime.threadID], initial: true) { _, ids in
            for id in ids where !seeded.contains(id) {
                seeded.insert(id)
                if model.settings.crewShowsHandDetails { open.insert(id) }
            }
        }
    }

    private func card(for hand: ChatThread) -> some View {
        CrewDeckCard(
            hand: hand,
            workingDirectory: workingDirectory,
            projectName: projectName,
            isOpen: binding(for: hand.id)
        )
        .id(hand.id)
    }

    private func binding(for id: UUID) -> Binding<Bool> {
        Binding(
            get: { open.contains(id) },
            set: { isOpen in
                withAnimation(Chrome.panelSlide) {
                    if isOpen { open.insert(id) } else { open.remove(id) }
                }
            }
        )
    }
}

// MARK: - The finished hands' header

/// The header over the deck's finished hands: how many, and a chevron that folds them
/// away or brings them back.
private struct CrewFinishedHeader: View {
    let count: Int
    let isFolded: Bool
    let toggle: () -> Void

    var body: some View {
        Button(action: toggle) {
            HStack(spacing: 6) {
                Image(systemName: "chevron.right")
                    .chromeChevronFont()
                    .rotationEffect(.degrees(isFolded ? 0 : 90))
                Text(verbatim: "Finished")
                    .zoomedFont(size: 11.5, weight: .semibold)
                Text(verbatim: "\(count)")
                    .zoomedFont(size: 11.5, monospacedDigit: true)
                Spacer(minLength: 0)
            }
            .foregroundStyle(Chrome.secondaryText)
            .padding(.top, 6)
            .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .focusable(false)
        .help(isFolded ? "Show finished hands" : "Fold finished hands")
        .accessibilityLabel(Text(verbatim: count == 1 ? "1 finished hand" : "\(count) finished hands"))
    }
}

// MARK: - A hand's card

/// One hand: its glyph, name, provider and clock, its task, its progress bar, and while
/// it works the tail of its thinking and its last steps; done, its outcome. Open, the
/// card shows the hand's transcript live, with the chat box or the footer under it.
private struct CrewDeckCard: View {
    @Environment(AppModel.self) private var model
    let hand: ChatThread
    let workingDirectory: String?
    let projectName: String?
    @Binding var isOpen: Bool

    var body: some View {
        if let stored = hand.crew {
            let runtime = model.runtime(for: hand.id)
            let info = CrewLiveInfo.shown(stored, runtime)
            let tint = info.persona.color
            CrewCardSurface(isOpen: isOpen) {
                CrewCardHeader(isOpen: $isOpen) {
                    CrewGlyph(persona: info.persona, size: 18, isRunning: info.status == .running, status: info.status)
                    CrewNameText(persona: info.persona, size: 13, weight: .semibold, color: Chrome.primaryText)
                        .lineLimit(1)
                    ProviderIcon(provider: hand.provider, size: 11)
                        .foregroundStyle(Chrome.secondaryText)
                        .help(model.providers.model(hand.model, for: hand.provider)?.shortName ?? hand.provider.displayName)
                    Text(verbatim: CrewStatusText.short(info) == "working" ? "Under way" : CrewStatusText.short(info).capitalized)
                        .zoomedFont(size: 11)
                        .foregroundStyle(info.status == .failed ? Chrome.danger : Chrome.secondaryText)
                        .lineLimit(1)
                } trailing: {
                    CrewTokenCount(runtime: runtime, recorded: info.tokens, ticks: info.status == .running && info.kind == .native)
                    CrewElapsedTime(startedAt: info.startedAt, finishedAt: info.finishedAt, isRunning: info.status == .running)
                        .foregroundStyle(Chrome.secondaryText)
                    if info.status == .running, info.canStop {
                        CrewCardButton(symbol: "stop.fill", help: "Stop \(info.displayName)") {
                            model.stopCrewHand(hand.id)
                        }
                    }
                }
                let task = info.task.trimmingCharacters(in: .whitespacesAndNewlines)
                Text(verbatim: task.isEmpty ? hand.title : task)
                    .zoomedFont(size: 12)
                    .foregroundStyle(Chrome.secondaryText)
                    .lineLimit(isOpen ? 3 : 1)
                    .truncationMode(.tail)
                CrewProgressBar(runtime: runtime, startedAt: info.startedAt, finishedAt: info.finishedAt, status: info.status, tint: tint)
                if isOpen {
                    CrewHandTranscript(hand: hand, height: CrewDeck.transcriptHeight, workingDirectory: workingDirectory, projectName: projectName)
                        .frame(height: CrewDeck.transcriptHeight)
                        .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
                        .transition(.opacity)
                } else if info.status == .running {
                    CrewCardTicker(runtime: runtime)
                } else {
                    Text(verbatim: CrewStatusText.footer(info))
                        .zoomedFont(size: 11)
                        .foregroundStyle(Chrome.secondaryText)
                        .lineLimit(2)
                }
            }
            .accessibilityElement(children: .contain)
            .accessibilityLabel(Text("\(info.displayName), \(CrewStatusText.long(info)), \(info.task)"))
        }
    }
}

// MARK: - The lead's card

/// The lead itself, first on the deck: the anchor, the helm's word while a turn runs,
/// the turn's clock and progress bar, and the tail of its thinking and steps. Open, the
/// running turn's thinking and steps in full, live. Between turns the card says so.
private struct CrewLeadCard: View {
    @Environment(AppModel.self) private var model
    let runtime: ThreadRuntime
    let workingDirectory: String?
    @Binding var isOpen: Bool

    var body: some View {
        let isRunning = runtime.isRunning
        let startedAt = runtime.turnStartedAt
        let entries = CrewTurn.entries(of: runtime)
        let provider = model.thread(runtime.threadID)?.provider
        CrewCardSurface(isOpen: isOpen) {
            CrewCardHeader(isOpen: $isOpen) {
                CrewMarkImage()
                    .foregroundStyle(Chrome.accent)
                    .frame(width: 18, height: 18)
                Text("Lead")
                    .zoomedFont(size: 13, weight: .semibold)
                    .foregroundStyle(Chrome.primaryText)
                if let provider {
                    ProviderIcon(provider: provider, size: 11)
                        .foregroundStyle(Chrome.secondaryText)
                }
                if isRunning, let startedAt {
                    // The helm's word, on the working line's own clock and seed.
                    TimelineView(.periodic(from: startedAt, by: TimeInterval(WorkingWords.rotateSeconds))) { context in
                        Text(verbatim: WorkingWords.word(seed: WorkingWords.seed(runtime.threadID.uuidString), elapsedSeconds: Int64(max(0, context.date.timeIntervalSince(startedAt)))) + "…")
                    }
                    .zoomedFont(size: 11)
                    .foregroundStyle(Chrome.secondaryText)
                    .lineLimit(1)
                } else {
                    Text("Moored")
                        .zoomedFont(size: 11)
                        .foregroundStyle(Chrome.secondaryText)
                }
            } trailing: {
                if let startedAt {
                    CrewElapsedTime(startedAt: startedAt, finishedAt: isRunning ? nil : entries.last?.item.date, isRunning: isRunning)
                        .foregroundStyle(Chrome.secondaryText)
                }
            }
            if let startedAt {
                CrewProgressBar(
                    runtime: runtime,
                    startedAt: startedAt,
                    finishedAt: isRunning ? nil : entries.last?.item.date,
                    status: isRunning ? .running : .completed,
                    tint: Chrome.accent,
                    entries: entries
                )
            }
            if isOpen {
                CrewTurnSteps(runtime: runtime, entries: entries, workingDirectory: workingDirectory)
                    .transition(.opacity)
            } else if isRunning {
                CrewCardTicker(runtime: runtime, turnOnly: true)
            } else {
                Text(startedAt == nil ? "Waiting for a prompt" : "Between turns")
                    .zoomedFont(size: 11)
                    .foregroundStyle(Chrome.secondaryText)
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel(Text(isRunning ? "Lead, at the helm" : "Lead, moored"))
    }
}

/// The running turn's rows: everything since the turn's prompt, walked back from the end
/// and stopped at the previous turn, so a streaming thread is never rescanned.
enum CrewTurn {
    @MainActor
    static func entries(of runtime: ThreadRuntime) -> [TimelineEntry] {
        guard let turnID = runtime.entries.last.flatMap(\.turnID) else { return [] }
        var rows: [TimelineEntry] = []
        for entry in runtime.entries.reversed() {
            guard let entryTurn = entry.turnID else { continue }
            guard entryTurn == turnID else { break }
            rows.append(entry)
        }
        return rows.reversed()
    }
}

/// The running turn opened up: its thinking, dimmed, its tool rows, and what it said
/// between them, in order and live, in a box of the transcript's height.
private struct CrewTurnSteps: View {
    let runtime: ThreadRuntime
    let entries: [TimelineEntry]
    let workingDirectory: String?

    var body: some View {
        ScrollView(.vertical) {
            VStack(alignment: .leading, spacing: 6) {
                ForEach(entries) { entry in
                    CrewTurnRow(entry: entry, runtime: runtime, workingDirectory: workingDirectory)
                }
                if entries.isEmpty {
                    Text("Nothing yet")
                        .zoomedFont(size: 11)
                        .foregroundStyle(Chrome.secondaryText.opacity(0.7))
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.vertical, 4)
        }
        .frame(height: CrewDeck.transcriptHeight)
        .defaultScrollAnchor(.bottom)
    }
}

/// One row of the lead's opened turn. Kind is fixed for the row's life, so the switch
/// never observes streaming content beyond the row's own.
private struct CrewTurnRow: View {
    let entry: TimelineEntry
    let runtime: ThreadRuntime
    let workingDirectory: String?

    var body: some View {
        switch entry.kind {
        case .reasoning:
            if case .reasoning(let block) = entry.item.content, !block.text.isEmpty {
                MarkdownView(text: block.text, isStreaming: block.isStreaming).equatable()
                    .foregroundStyle(.secondary)
                    .environment(\.markdownPointSize, 11)
                    .environment(\.markdownDimmed, true)
            }
        case .assistant:
            if case .assistant(let message) = entry.item.content, !message.text.isEmpty {
                MarkdownView(text: message.text, isStreaming: message.isStreaming).equatable()
                    .environment(\.markdownPointSize, 12)
            }
        case .tool:
            ToolRow(entry: entry, runtime: runtime, workingDirectory: workingDirectory)
        default:
            EmptyView()
        }
    }
}

// MARK: - The ticker

/// What a hand is doing right now, in a few lines: the tail of its latest thinking and
/// its last two steps. Reads the runtime's rows, so it follows the stream; the rest of
/// the card is drawn once per change of state.
private struct CrewCardTicker: View {
    let runtime: ThreadRuntime
    /// Only the running turn's rows, for the lead's card.
    var turnOnly = false
    /// The window's text size: the symbols' column grows with the type beside it.
    @Environment(\.chatZoom) private var zoom

    var body: some View {
        let tail = Self.tail(of: runtime, turnOnly: turnOnly)
        VStack(alignment: .leading, spacing: 3) {
            if let thought = tail.thought {
                HStack(alignment: .firstTextBaseline, spacing: 6) {
                    Image(systemName: "brain")
                        .zoomedFont(size: 9, weight: .semibold)
                        .frame(width: ChatZoom.scaled(12, by: zoom))
                    Text(verbatim: thought)
                        .italic()
                        .lineLimit(2)
                }
                .zoomedFont(size: 11)
                .foregroundStyle(Chrome.secondaryText.opacity(0.85))
            }
            ForEach(tail.steps, id: \.self) { step in
                HStack(spacing: 6) {
                    Image(systemName: step.symbol)
                        .zoomedFont(size: 9, weight: .semibold)
                        .frame(width: ChatZoom.scaled(12, by: zoom))
                    Text(verbatim: step.label)
                        .lineLimit(1)
                        .truncationMode(.tail)
                }
                .zoomedFont(size: 11)
                .foregroundStyle(Chrome.secondaryText)
            }
            if tail.thought == nil, tail.steps.isEmpty {
                Text("Nothing yet")
                    .zoomedFont(size: 11)
                    .foregroundStyle(Chrome.secondaryText.opacity(0.7))
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    struct Step: Hashable {
        let symbol: String
        let label: String
    }

    struct Tail {
        var thought: String?
        var steps: [Step] = []
    }

    /// The latest thinking's last line and the last two tool rows, newest last.
    @MainActor
    static func tail(of runtime: ThreadRuntime, turnOnly: Bool) -> Tail {
        var tail = Tail()
        let turnID = turnOnly ? runtime.entries.last.flatMap(\.turnID) : nil
        for entry in runtime.entries.reversed() {
            if let turnID {
                guard let entryTurn = entry.turnID else { continue }
                guard entryTurn == turnID else { break }
            }
            switch entry.kind {
            case .reasoning where tail.thought == nil:
                if case .reasoning(let block) = entry.item.content, !block.text.isEmpty {
                    tail.thought = lastLine(of: block.text)
                }
            case .tool where tail.steps.count < 2:
                if case .tool(let call) = entry.item.content {
                    let symbol = ToolPresentation.mcpParts(call) == nil ? ToolPresentation.symbol(for: call.kind) : "puzzlepiece.extension"
                    tail.steps.append(Step(symbol: symbol, label: ToolPresentation.label(for: call)))
                }
            default:
                break
            }
            if tail.thought != nil, tail.steps.count == 2 { break }
        }
        tail.steps.reverse()
        return tail
    }

    /// The last line of a thought, as plain words: a provider's thinking often comes as
    /// markdown headings and bold lead-ins, and the marks are noise at this size.
    private static func lastLine(of text: String) -> String {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        var line = trimmed.split(whereSeparator: \.isNewline).last.map(String.init) ?? trimmed
        while line.hasPrefix("#") || line.hasPrefix(">") { line.removeFirst() }
        for mark in ["**", "__", "`", "*", "_"] { line = line.replacingOccurrences(of: mark, with: "") }
        return TextCleanup.singleLine(line, limit: 140)
    }
}

// MARK: - The spend

/// What a hand has spent, in tokens, beside its clock: the running total its provider
/// reports, or what its own turns recorded, whichever is more (see
/// `ThreadRuntime.crewSpentTokens`). The turns are observed, so a hosted hand's count
/// moves as its turns record their spend. A native hand's provider reports a running
/// total many times a second, and that total is not observed (following every report
/// would redraw the whole card each time), so while such a hand works its count is read
/// on a tick of its own. A finished hand shows what landed on its record. Nothing until
/// there is something to count.
private struct CrewTokenCount: View {
    let runtime: ThreadRuntime
    /// The spend the hand's record holds: its total once it has finished.
    let recorded: Int
    /// A native hand at work, whose running total is read on the tick.
    let ticks: Bool

    /// How often a working native hand's count is read again.
    private static let tick: TimeInterval = 2

    var body: some View {
        if ticks {
            TimelineView(.periodic(from: .now, by: Self.tick)) { _ in
                label(max(recorded, runtime.crewSpentTokens))
            }
        } else {
            label(max(recorded, runtime.crewSpentTokens))
        }
    }

    @ViewBuilder
    private func label(_ tokens: Int) -> some View {
        if tokens > 0 {
            Text(verbatim: UsagePanel.tokens(tokens))
                .zoomedFont(size: 11, monospacedDigit: true)
                .foregroundStyle(Chrome.secondaryText)
                .help("\(tokens.formatted()) tokens spent")
                .accessibilityLabel(Text("\(tokens.formatted()) tokens spent"))
        }
    }
}

// MARK: - Card chrome

/// The card's surface: a flat control fill on the panel's glass (a second glass here
/// sampled the same pixels twice), rounder and a touch brighter while it is open.
private struct CrewCardSurface<Content: View>: View {
    @Environment(\.colorScheme) private var colorScheme
    let isOpen: Bool
    @ViewBuilder var content: Content

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            content
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background {
            RoundedRectangle(cornerRadius: 14, style: .continuous)
                .fill(Chrome.panelControlFill(isDark: colorScheme == .dark, hovered: isOpen))
        }
    }
}

/// The card's first row: the identity at the left, which opens and closes the card, and
/// the clock and buttons at the right, which never do.
private struct CrewCardHeader<Leading: View, Trailing: View>: View {
    @Binding var isOpen: Bool
    @ViewBuilder var leading: Leading
    @ViewBuilder var trailing: Trailing

    var body: some View {
        HStack(spacing: 8) {
            Button {
                isOpen.toggle()
            } label: {
                HStack(spacing: 6) {
                    leading
                    Spacer(minLength: 4)
                }
                .contentShape(.rect)
            }
            .buttonStyle(.plain)
            .accessibilityAddTraits(.isButton)
            trailing
            Image(systemName: "chevron.right")
                .chromeChevronFont()
                .foregroundStyle(Chrome.secondaryText)
                .rotationEffect(.degrees(isOpen ? 90 : 0))
        }
    }
}

/// A small round button on a card: stop, or pop out.
private struct CrewCardButton: View {
    let symbol: String
    let help: String
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Image(systemName: symbol)
                .font(.system(size: 9, weight: .bold))
                .foregroundStyle(Chrome.primaryText.opacity(0.9))
                .frame(width: 20, height: 20)
                .background(Chrome.overlay(0.12), in: .circle)
        }
        .buttonStyle(.plain)
        .help(help)
        .accessibilityLabel(Text(help))
    }
}

// MARK: - The hand's transcript, as the team panel showed it before the deck

/// A hand's conversation, live as it works, with what it can be told below.
struct CrewHandTranscript: View {
    @Environment(AppModel.self) private var model
    let hand: ChatThread
    let height: CGFloat
    let workingDirectory: String?
    let projectName: String?

    @State private var scrollChrome = ChromeScrollModel()
    @State private var scrollState = TimelineScrollState()

    var body: some View {
        let runtime = model.runtime(for: hand.id)
        // Always the whole transcript: the card above already carries the task and the
        // bar, and "Every step" in Settings only says whether cards start open.
        ThreadTimeline(
            runtime: runtime,
            scrollChrome: scrollChrome,
            scrollState: scrollState,
            projectName: projectName,
            workingDirectory: workingDirectory,
            allowsUndo: false,
            columnHeight: height,
            // A panel's transcript needs no rail, and the rail's hover tracking
            // and geometry reader are per-frame work under the pointer.
            showsMinimap: false
        )
        .equatable()
        .safeAreaInset(edge: .bottom, spacing: 0) {
            if hand.crew?.kind == .hosted {
                // A child Bottega runs itself can be steered and answered like any chat.
                ComposerArea(runtime: runtime, workingDirectory: workingDirectory, compactModelChip: true, takesFocusOnAppear: false)
                    .overlay(alignment: .top) {
                        JumpToLatestButton(scrollState: scrollState)
                    }
            } else if let stored = hand.crew {
                CrewHandFooter(info: CrewLiveInfo.shown(stored, runtime))
                    .overlay(alignment: .top) {
                        JumpToLatestButton(scrollState: scrollState)
                    }
            }
        }
        .overlay(alignment: .top) {
            PaneTopVeil(model: scrollChrome)
        }
    }
}

/// What a native hand is up to, in place of a chat box: its progress note or last tool,
/// its tool count and its time.
struct CrewHandFooter: View {
    @Environment(\.colorScheme) private var colorScheme
    let info: CrewHandInfo

    var body: some View {
        HStack(spacing: 8) {
            if info.status == .running {
                MiniSpinner()
            } else {
                Image(systemName: info.status == .completed ? "checkmark.circle.fill" : (info.status == .failed ? "xmark.circle.fill" : "stop.circle.fill"))
                    .zoomedFont(size: 12)
                    .foregroundStyle(info.status == .completed ? Chrome.success : (info.status == .failed ? Chrome.danger : Chrome.secondaryText))
            }
            Text(verbatim: CrewStatusText.footer(info))
                .zoomedFont(size: 12)
                .foregroundStyle(Chrome.secondaryText)
                .lineLimit(1)
                .truncationMode(.tail)
            Spacer(minLength: 8)
            if info.toolCalls > 0 {
                Text(verbatim: info.toolCalls == 1 ? "1 step" : "\(info.toolCalls) steps")
                    .zoomedFont(size: 11, monospacedDigit: true)
                    .foregroundStyle(Chrome.secondaryText)
            }
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 10)
        // Flat on the panel's glass: a second glass here sampled the same pixels twice.
        .background(RoundedRectangle(cornerRadius: 16, style: .continuous).fill(Chrome.panelControlFill(isDark: colorScheme == .dark)))
        .padding(.horizontal, 20)
        .padding(.bottom, 14)
    }
}

