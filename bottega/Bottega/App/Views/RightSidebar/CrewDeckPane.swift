// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import AppKit
import SwiftUI

/// The Crew page of the right sidebar: a bar naming the team, and under it the deck —
/// the lead and every hand it has sent out, one card each, live (see `CrewDeck`): those
/// at work first, the finished ones under a header that folds. The bar's usage switch
/// puts the run's recorded usage and each hand's phases on a card at the top of the deck
/// (see `CrewRunMetricsView`). With nothing out yet the page says so rather than standing
/// empty.
struct CrewDeckPane: View {
    @Environment(AppModel.self) private var model
    let runtime: ThreadRuntime
    let hands: [ChatThread]
    let workingDirectory: String?
    let projectName: String?

    /// How long a hand that just finished keeps the page's wave running.
    static let finishHold: TimeInterval = FloatingPanelMetrics.finishHold

    /// A hand that has just finished: the page plays its wave.
    @State private var hold: HandFinish?
    /// The usage and timings card is on the deck. Off until asked for: reading it loads
    /// the history of every hand on the deck.
    @State private var showsMetrics = false

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            if !hands.isEmpty {
                HStack(spacing: 8) {
                    CrewHandsButton(runtime: runtime, hands: hands)
                    ChromeCircleButton(
                        symbol: showsMetrics ? "chart.bar.fill" : "chart.bar",
                        help: showsMetrics ? "Hide usage and timings" : "Show usage and timings"
                    ) {
                        withAnimation(Chrome.panelSlide) { showsMetrics.toggle() }
                    }
                }
                .padding(.horizontal, Chrome.listInset)
                .padding(.bottom, 8)
            }
            if hands.isEmpty {
                RightSidebarEmpty(
                    text: "No hands out yet",
                    detail: "Ask for work big enough to split and the lead sends out a team; every hand gets a card here."
                )
            } else {
                CrewDeck(
                    runtime: runtime,
                    hands: hands,
                    workingDirectory: workingDirectory,
                    projectName: projectName,
                    showsMetrics: showsMetrics
                )
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
        .overlay {
            // Done: one wave of the hand's colour flows through the page.
            if let hold, let hand = hands.first(where: { $0.id == hold.id }) {
                DotFieldSweep(tint: NSColor((hand.crew?.persona ?? CrewRoster.persona(at: 0)).color))
                    .allowsHitTesting(false)
                    .transition(.opacity)
                    .id(hold)
            }
        }
        .onChange(of: hands.map { HandMark(id: $0.id, status: $0.crew?.status) }) { old, new in
            // A hand just finished: the page plays its wave.
            let before = Dictionary(uniqueKeysWithValues: old.compactMap { mark in mark.id.map { ($0, mark.status) } })
            guard let id = new.first(where: { mark in
                guard let id = mark.id else { return false }
                return (before[id] ?? nil) == .running && mark.status != .running
            })?.id else { return }
            withAnimation(Chrome.panelSlide) { hold = HandFinish(id: id, startedAt: .now) }
        }
        .task(id: hold) {
            guard hold != nil else { return }
            guard (try? await Task.sleep(for: .seconds(Self.finishHold))) != nil else { return }
            withAnimation(Chrome.panelSlide) { hold = nil }
        }
    }
}

/// The hand whose finish the page is playing, and since when: a new hold for the same
/// hand plays its wave again.
private struct HandFinish: Hashable {
    let id: UUID
    let startedAt: Date
}

/// A hand's state, as the key that says it finished.
private struct HandMark: Equatable {
    let id: UUID?
    let status: CrewHandInfo.Status?
}

// MARK: - The team bar

/// The bar over the deck: how many hands, how many at work, and a popover with every
/// hand — pick one to scroll the deck to its card, stop it, or send the team away.
private struct CrewHandsButton: View {
    @Environment(AppModel.self) private var model
    let runtime: ThreadRuntime
    let hands: [ChatThread]

    @State private var isPresented = false
    @State private var isHovering = false

    var body: some View {
        let running = hands.count { $0.crew?.status == .running }
        HStack(spacing: 8) {
            HStack(spacing: 6) {
                CrewMarkImage()
                    .foregroundStyle(Chrome.primaryText.opacity(0.9))
                    .frame(width: 15, height: 15)
                Text(verbatim: "\(hands.count)")
                    .zoomedFont(size: 12.5, weight: .semibold, monospacedDigit: true)
                if running > 0 {
                    MiniSpinner()
                }
                Image(systemName: "chevron.down")
                    .chromeChevronFont()
                    .foregroundStyle(Chrome.secondaryText)
            }
            .foregroundStyle(Chrome.primaryText.opacity(isHovering || isPresented ? 1 : 0.92))
            .padding(.horizontal, Chrome.capsuleHorizontalPadding)
            .frame(height: Chrome.capsuleContentHeight)
            .padding(.vertical, Chrome.capsuleVerticalPadding)
            .contentShape(Capsule(style: .continuous))
            .fixedSize()
            .chromeGlassCapsule()
            .simultaneousGesture(TapGesture().onEnded { isPresented.toggle() })
            .onHover { hovering in
                withAnimation(Chrome.hover) { isHovering = hovering }
            }
            .help(hands.count == 1 ? "1 hand" : "\(hands.count) hands")
            .accessibilityAddTraits(.isButton)
            .accessibilityLabel(Text(hands.count == 1 ? "1 hand" : "\(hands.count) hands"))
            .accessibilityAction { isPresented.toggle() }
            Text(verbatim: CrewTeamText.line(running: running, waiting: runtime.crewWaitingCount, total: hands.count))
                .zoomedFont(size: 11)
                .foregroundStyle(Chrome.secondaryText)
                .lineLimit(1)
            Spacer(minLength: 0)
        }
        .popover(isPresented: $isPresented, arrowEdge: .bottom) {
            PopoverMenu {
                let waiting = runtime.crewWaitingCount
                let header = running == 0 ? "Hands" : (running == 1 ? "1 hand working" : "\(running) hands working")
                PopoverSectionHeader(waiting > 0 ? header + (waiting == 1 ? ", 1 waiting" : ", \(waiting) waiting") : header)
                ForEach(hands) { hand in
                    CrewHandRow(hand: hand, isPicked: hand.id == runtime.crewSelectedHandID) {
                        runtime.crewSelectedHandID = hand.id
                        runtime.crewDeckReveal = hand.id
                        isPresented = false
                    }
                }
                PopoverDivider()
                if running > 1 {
                    PopoverItem("Stop all hands", symbol: "stop.circle") {
                        model.stopAllCrewHands(of: runtime.threadID)
                        isPresented = false
                    }
                }
                // Offered while there is a finished hand still in the panel, or one still
                // unfolded here: they leave the panel and give their copies back, and on this
                // page they only fold under their header, still there to open.
                if hands.contains(where: { $0.crew?.isFinished == true && ($0.isInPanel || !runtime.crewFinishedFolded) }) {
                    PopoverItem("Clear finished hands", symbol: "checkmark.circle") {
                        withAnimation(Chrome.panelSlide) {
                            model.clearFinishedCrewHands(of: runtime.threadID)
                            runtime.crewFinishedFolded = true
                        }
                    }
                }
                PopoverItem("Send the team away", symbol: "xmark.circle") {
                    isPresented = false
                    withAnimation(Chrome.panelSlide) {
                        model.dismissCrewHands(of: runtime.threadID)
                    }
                }
            }
        }
    }
}

/// The line beside the count: what the team is up to, in a few words.
enum CrewTeamText {
    static func line(running: Int, waiting: Int, total: Int) -> String {
        if waiting > 0 {
            return waiting == 1 ? "1 waiting for go" : "\(waiting) waiting for go"
        }
        if running > 0 {
            return running == 1 ? "1 under way" : "\(running) under way"
        }
        return total == 1 ? "1 home" : "all home"
    }
}

/// One hand in the popover: its glyph (wearing its outcome once it is done), its name and
/// task, what it is doing, and a stop button while it works. The hand last picked sits on
/// a tinted row.
private struct CrewHandRow: View {
    @Environment(AppModel.self) private var model
    let hand: ChatThread
    let isPicked: Bool
    let select: () -> Void

    @State private var isHovering = false

    var body: some View {
        if let stored = hand.crew {
            // A running hand's note and counts live on its runtime, off the thread
            // record; this row is the one view that follows them.
            let info = CrewLiveInfo.shown(stored, model.existingRuntime(for: hand.id))
            HStack(spacing: 10) {
                Button(action: select) {
                    HStack(spacing: 10) {
                        CrewGlyph(persona: info.persona, size: 22, isRunning: info.status == .running, status: info.status)
                        VStack(alignment: .leading, spacing: 2) {
                            HStack(spacing: 6) {
                                CrewNameText(persona: info.persona, size: 13, weight: .medium, color: Chrome.primaryText)
                                // The provider's mark, so the list says whose model each
                                // hand runs on.
                                ProviderIcon(provider: hand.provider, size: 11)
                                    .foregroundStyle(Chrome.secondaryText)
                                    .help(model.providers.model(hand.model, for: hand.provider)?.shortName ?? hand.provider.displayName)
                                Text(verbatim: CrewStatusText.long(info))
                                    .zoomedFont(size: 11)
                                    .foregroundStyle(info.status == .failed ? Chrome.danger : Chrome.secondaryText)
                                    .lineLimit(1)
                            }
                            Text(verbatim: info.task.isEmpty ? "Waiting for a task" : info.task)
                                .zoomedFont(size: 12)
                                .foregroundStyle(Chrome.secondaryText)
                                .lineLimit(1)
                                .truncationMode(.tail)
                        }
                        Spacer(minLength: 8)
                    }
                    .contentShape(.rect)
                }
                .buttonStyle(.plain)
                if info.status == .running, info.canStop {
                    CrewStopButton(name: info.displayName) {
                        model.stopCrewHand(hand.id)
                    }
                }
            }
            .padding(.horizontal, 8)
            .padding(.vertical, 5)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background {
                RoundedRectangle(cornerRadius: 6, style: .continuous)
                    .fill(isPicked ? Chrome.overlay(0.14) : (isHovering ? Chrome.overlay(0.08) : Color.clear))
            }
            .onHover { hovering in
                withAnimation(Chrome.hover) { isHovering = hovering }
            }
            .accessibilityElement(children: .combine)
            .accessibilityLabel(Text("\(info.displayName), \(CrewStatusText.long(info)), \(info.task)"))
        }
    }
}

/// The stop button beside a working hand: the same tinted circle with the white stop
/// mark the composer shows while a turn runs.
struct CrewStopButton: View {
    let name: String
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Image(systemName: "stop.fill")
                .font(Chrome.iconFont)
                .foregroundStyle(.white)
                .frame(width: 28, height: 28)
                .background(.tint, in: .circle)
        }
        .buttonStyle(.plain)
        .help("Stop \(name)")
        .accessibilityLabel(Text("Stop \(name)"))
    }
}

// MARK: - A hand's live record and the words for its state

/// A running hand's record with the live progress the deck actually prints on top of it:
/// the note and the step count. The live token total stays out: a provider reports it
/// many times a second, so following it would redraw the row on every token the hand
/// spends; the card's count reads it on a tick of its own (`CrewTokenCount`), and it lands
/// on the record as the hand finishes.
@MainActor
enum CrewLiveInfo {
    static func shown(_ stored: CrewHandInfo, _ runtime: ThreadRuntime?) -> CrewHandInfo {
        guard stored.status == .running, let runtime else { return stored }
        var info = stored
        if let activity = runtime.crewActivity { info.activity = activity }
        info.toolCalls = max(info.toolCalls, runtime.crewToolCalls)
        return info
    }
}

/// The words for a hand's state.
enum CrewStatusText {
    static func short(_ info: CrewHandInfo) -> String {
        switch info.status {
        case .running: "working"
        case .completed: "done"
        case .failed: "failed"
        case .stopped: "stopped"
        }
    }

    static func long(_ info: CrewHandInfo) -> String {
        switch info.status {
        case .running:
            return info.activity.map { "· " + TextCleanup.singleLine($0, limit: 40) } ?? "· under way"
        case .completed:
            return "· done" + elapsed(info) + steps(info) + spent(info) + (landed(info).map { " · " + $0 } ?? "")
        case .failed:
            return "· failed"
        case .stopped:
            return "· stopped"
        }
    }

    static func footer(_ info: CrewHandInfo) -> String {
        switch info.status {
        case .running:
            return info.activity.map { TextCleanup.singleLine($0, limit: 80) } ?? "Under way"
        case .completed:
            return "Done" + elapsed(info) + steps(info) + (landed(info).map { " · " + $0 } ?? "") + (info.summary.map { " · " + TextCleanup.singleLine($0, limit: 80) } ?? "")
        case .failed:
            return "Failed" + (info.summary.map { " · " + TextCleanup.singleLine($0, limit: 80) } ?? "")
        case .stopped:
            return "Stopped"
        }
    }

    /// Where a hosted hand's work went, in a few words; nil for a hand that recorded no
    /// landing (one in the checkout itself whose tools landed no edit).
    static func landed(_ info: CrewHandInfo) -> String? {
        guard let landing = info.landing else { return nil }
        if landing.isEmpty { return "changed nothing" }
        let files = landing.files.count == 1 ? "1 file" : "\(landing.files.count) files"
        if landing.patchPath != nil { return "kept as a patch" }
        if landing.error != nil { return "did not land" }
        if !landing.conflicts.isEmpty {
            return "landed \(files), " + (landing.conflicts.count == 1 ? "1 conflict" : "\(landing.conflicts.count) conflicts")
        }
        return "landed \(files)"
    }

    private static func steps(_ info: CrewHandInfo) -> String {
        guard info.toolCalls > 0 else { return "" }
        return info.toolCalls == 1 ? " · 1 step" : " · \(info.toolCalls) steps"
    }

    /// What the hand spent, as it landed on its record when it finished; nothing where no
    /// provider reported a spend.
    private static func spent(_ info: CrewHandInfo) -> String {
        guard info.tokens > 0 else { return "" }
        return " · \(UsagePanel.tokens(info.tokens)) tokens"
    }

    private static func elapsed(_ info: CrewHandInfo) -> String {
        guard let finished = info.finishedAt else { return "" }
        let seconds = Int(finished.timeIntervalSince(info.startedAt))
        guard seconds > 0 else { return "" }
        return seconds < 60 ? " in \(seconds)s" : " in \(seconds / 60)m \(seconds % 60)s"
    }
}
