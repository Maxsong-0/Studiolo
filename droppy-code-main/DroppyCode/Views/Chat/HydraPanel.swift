import AppKit
import SwiftUI

/// The lead's team as a floating glass panel, in the same spot and size as the helper
/// panel: the head on stage in the middle, its conversation live as it works, and a strip
/// along the top that is the handle. The strip's left end counts the heads and opens
/// them in a popover, where any head can take the stage or be stopped; its right end
/// pops the head on stage out into a second panel of its own, and dismisses the panel. A
/// Droppy-run head keeps its own chat box at the bottom, so it can be steered or answered;
/// a native head shows what it is up to instead.
///
/// Popped out, the panel holds one head: no count at the left, and its right end puts the
/// head back in the team panel.
struct HydraPanel: View {
    @Environment(AppModel.self) private var model
    @Environment(WindowLiveResize.self) private var liveResize
    @Environment(\.colorScheme) private var colorScheme
    let runtime: ThreadRuntime
    let heads: [ChatThread]
    let size: CGSize
    let workingDirectory: String?
    let projectName: String?
    /// A second panel holding one head popped out of the team panel.
    var isPoppedOut = false
    /// The pointer's travel since the handle was grabbed.
    let onDrag: (CGSize) -> Void
    let onDragEnd: () -> Void
    /// Pops the head on stage out into its own panel; nil where it cannot go anywhere.
    var popOut: ((UUID) -> Void)? = nil
    /// Dismisses the panel; for a popped-out one, puts its head back in the team panel.
    let dismiss: () -> Void

    static let cornerRadius: CGFloat = 22
    /// The strip along the top, an overlay over the content: what sits under it in
    /// progress mode leaves it room (see `HydraHeadProgress`). The helper panel's handle
    /// is the same height.
    static let stripHeight: CGFloat = Chrome.chromeTopPadding + Chrome.capsuleHeight + 6
    /// How long a head that just finished keeps the stage: its wave crosses the panel
    /// and settles before the next head takes over or the panel goes. The model holds a
    /// finished head in the panel this long before clearing it (see `finishHydraHead`).
    static let finishHold: TimeInterval = 2.0

    /// A head that finished while on stage holds it for `finishHold`.
    @State private var hold: StageHold?

    var body: some View {
        // The head on stage: the one picked, else the newest still working, else the newest.
        // A popped-out panel holds one head, and that one is on stage. A head that has just
        // finished on stage keeps it while its wave plays, whatever else is at work.
        let wanted = isPoppedOut
            ? heads.first
            : heads.first { $0.id == runtime.hydraSelectedHeadID }
                ?? heads.last { $0.hydra?.status == .running }
                ?? heads.last
        let held = hold.flatMap { hold in heads.first { $0.id == hold.id } }
        let selected = held ?? wanted
        let shape = RoundedRectangle(cornerRadius: Self.cornerRadius, style: .continuous)
        ZStack {
            if let selected {
                HydraHeadTranscript(
                    head: selected,
                    height: size.height,
                    workingDirectory: workingDirectory,
                    projectName: projectName
                )
                .id(selected.id)
                // The next head fades in over the last rather than popping into place.
                .transition(.opacity)
            }
        }
        .animation(liveResize.isActive ? nil : Chrome.panelSlide, value: selected?.id)
        .overlay {
            // Done: one wave of the head's colour flows through the whole panel.
            if let hold, let head = heads.first(where: { $0.id == hold.id }) {
                DotFieldSweep(tint: NSColor((head.hydra?.persona ?? HydraRoster.persona(at: 0)).color))
                    .clipShape(shape)
                    .transition(.opacity)
                    .id(hold)
            }
        }
        .onChange(of: StageMark(id: selected?.id, status: selected?.hydra?.status)) { old, new in
            // The head on stage just finished: it holds the stage for its wave.
            guard let id = new.id, old.id == id, old.status == .running, new.status != .running else { return }
            // Faded in as it is faded out below, so the wave never pops onto the stage.
            withAnimation(Chrome.panelSlide) { hold = StageHold(id: id, startedAt: .now) }
        }
        .task(id: hold) {
            guard hold != nil else { return }
            guard (try? await Task.sleep(for: .seconds(Self.finishHold))) != nil else { return }
            withAnimation(Chrome.panelSlide) { hold = nil }
        }
        .overlay(alignment: .top) {
            strip(selected: selected)
        }
        .frame(width: size.width, height: size.height)
        // The frame follows the pane every frame of a live resize: the strip's
        // swaps settle after, never during.
        .animation(nil, value: liveResize.isActive)
        .background {
            // The helper panel's recipe: one glass surface, a scrim for the text over
            // whatever the panel floats above, and the theme's tint.
            let isDark = colorScheme == .dark
            shape
                .fill(.clear)
                .glassEffect(.regular, in: shape)
                .overlay {
                    shape.fill(Chrome.glassTint.opacity(isDark ? 0.22 : 0.16))
                }
        }
        // Glass supplies the edge (as in `WindowBackdrop`): a hairline on top of it
        // read as a second ring around the panel whenever its window was key.
        .clipShape(shape)
        // The scrim sits under the glass and carries the panel's shadow: a plain filled
        // shape, so its shadow is drawn once and kept. A shadow on the whole panel was
        // blurred again with every token the transcript streamed under it.
        .background {
            let isDark = colorScheme == .dark
            shape
                .fill((isDark ? Color.black : Color.white).opacity(isDark ? 0.3 : 0.34))
                .shadow(color: .black.opacity(isDark ? 1 : 0.65), radius: 28, y: 10)
        }
    }

    /// The handle across the top: the count and the head on stage at the left, the pop-out
    /// and close buttons at the right, and the room between them drags the panel.
    private func strip(selected: ChatThread?) -> some View {
        HStack(alignment: .top, spacing: 8) {
            HStack(spacing: 8) {
                if !isPoppedOut {
                    HydraHeadsButton(runtime: runtime, heads: heads, dismiss: dismiss)
                }
                // The head's stored record, not its live one: the strip sits over a
                // streaming chat and shows only the name and the state, neither of which
                // the runtime's progress changes. Reading that progress here would redraw
                // the strip on every token the head spends.
                if let selected, let info = selected.hydra {
                    HStack(spacing: 6) {
                        HydraGlyph(persona: info.persona, size: 16, isRunning: info.status == .running, status: info.status)
                        Text(verbatim: info.persona.name)
                            .font(.system(size: 12.5, weight: .medium))
                            .foregroundStyle(Chrome.primaryText.opacity(0.92))
                            .lineLimit(1)
                        // The provider's mark: the strip says whose model is at work on
                        // stage, whether or not it is the lead's own.
                        ProviderIcon(provider: selected.provider, size: 12)
                            .foregroundStyle(Chrome.secondaryText)
                            .help(model.providers.model(selected.model, for: selected.provider)?.shortName ?? selected.provider.displayName)
                        Text(verbatim: HydraStatusText.short(info))
                            .font(.system(size: 12))
                            .foregroundStyle(Chrome.secondaryText)
                            .lineLimit(1)
                        // How long the head has been at it, or took: its own clock, so the
                        // strip itself is still drawn once.
                        HydraElapsedTime(startedAt: info.startedAt, finishedAt: info.finishedAt, isRunning: info.status == .running)
                    }
                    .padding(.horizontal, Chrome.capsuleHorizontalPadding)
                    .frame(height: Chrome.capsuleContentHeight)
                    .padding(.vertical, Chrome.capsuleVerticalPadding)
                    .fixedSize()
                    .chromeGlassCapsule()
                    // The name is a label, not a control, so it drags the panel too:
                    // it is where the hand goes when there is little room beside it.
                    .overlay {
                        PanelDragHandle(onDrag: onDrag, onDragEnd: onDragEnd)
                    }
                    .transition(.softAppear)
                }
            }
            .padding(.top, Chrome.chromeTopPadding)
            .padding(.leading, Chrome.chromeHorizontalPadding)
            // The handle. Invisible, like a window's title bar with no title; the cursor
            // says what it does.
            PanelDragHandle(onDrag: onDrag, onDragEnd: onDragEnd)
                .frame(maxWidth: .infinity)
                .frame(height: Self.stripHeight)
                .help("Drag to move")
                .accessibilityLabel(Text("Drag to move"))
            HStack(spacing: 8) {
                if isPoppedOut {
                    ChromeCircleButton(symbol: "arrow.down.left", help: "Put \(selected?.hydra?.persona.name ?? "the head") back in the team panel") {
                        dismiss()
                    }
                } else {
                    // The head on stage pops out into a second panel, as long as another
                    // head stays behind to keep this one.
                    if let selected, let popOut, heads.count > 1 {
                        ChromeCircleButton(symbol: "arrow.up.right", help: "Pop \(selected.hydra?.persona.name ?? "this head") out into its own panel") {
                            popOut(selected.id)
                        }
                        .transition(.softAppear)
                    }
                    ChromeCircleButton(symbol: "xmark", help: "Dismiss the panel; heads keep working") {
                        dismiss()
                    }
                }
            }
            .padding(.top, Chrome.chromeTopPadding)
            .padding(.trailing, Chrome.chromeHorizontalPadding)
        }
        .animation(liveResize.isActive ? nil : Chrome.panelSlide, value: selected?.id)
        .animation(liveResize.isActive ? nil : Chrome.panelSlide, value: heads.count > 1)
    }
}

/// The head holding the stage past its finish, and since when: a new hold for the same
/// head plays its wave again.
private struct StageHold: Hashable {
    let id: UUID
    let startedAt: Date
}

/// What the stage shows, as the key that says a head finished on it.
private struct StageMark: Equatable {
    let id: UUID?
    let status: HydraHeadInfo.Status?
}

/// The count at the strip's left end: it opens the team in a popover. Not a Button: a
/// plain-style Button keeps the click to itself and the glass never sees it, so the capsule
/// sat still under the pointer while the name capsule beside it, bare interactive glass,
/// pressed like a native control. The tap rides along with the glass's own tracking instead,
/// so the whole capsule presses the native way.
private struct HydraHeadsButton: View {
    @Environment(AppModel.self) private var model
    let runtime: ThreadRuntime
    let heads: [ChatThread]
    let dismiss: () -> Void

    @State private var isPresented = false
    @State private var isHovering = false

    var body: some View {
        let running = heads.count { $0.hydra?.status == .running }
        HStack(spacing: 6) {
            HydraSpokesMark()
            Text(verbatim: "\(heads.count)")
                .font(.system(size: 12.5, weight: .semibold).monospacedDigit())
            if running > 0 {
                MiniSpinner()
            }
            Image(systemName: "chevron.down")
                .font(Chrome.chevronFont)
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
        .help(heads.count == 1 ? "1 head" : "\(heads.count) heads")
        .accessibilityAddTraits(.isButton)
        .accessibilityLabel(Text(heads.count == 1 ? "1 head" : "\(heads.count) heads"))
        .accessibilityAction { isPresented.toggle() }
        .popover(isPresented: $isPresented, arrowEdge: .bottom) {
            PopoverMenu {
                PopoverSectionHeader(running == 0 ? "Heads" : (running == 1 ? "1 head working" : "\(running) heads working"))
                ForEach(heads) { head in
                    HydraHeadRow(head: head, isOnStage: head.id == runtime.hydraSelectedHeadID) {
                        runtime.hydraSelectedHeadID = head.id
                        isPresented = false
                    }
                }
                if running > 1 || heads.contains(where: { $0.hydra?.isFinished == true }) {
                    PopoverDivider()
                }
                if running > 1 {
                    PopoverItem("Stop all heads", symbol: "stop.circle") {
                        model.stopAllHydraHeads(of: runtime.threadID)
                        isPresented = false
                    }
                }
                if heads.contains(where: { $0.hydra?.isFinished == true }) {
                    PopoverItem("Clear finished heads", symbol: "checkmark.circle") {
                        withAnimation(Chrome.panelSlide) {
                            model.clearFinishedHydraHeads(of: runtime.threadID)
                        }
                    }
                }
            }
        }
    }
}

/// The dragon-head mark at capsule size.
private struct HydraSpokesMark: View {
    var body: some View {
        HydraMarkImage()
            .foregroundStyle(Chrome.primaryText.opacity(0.9))
            .frame(width: 15, height: 15)
    }
}

/// One head in the popover: its glyph (wearing its outcome once it is done), its name and
/// task, what it is doing, and a stop button while it works. The head on stage sits on a
/// tinted row.
private struct HydraHeadRow: View {
    @Environment(AppModel.self) private var model
    let head: ChatThread
    let isOnStage: Bool
    let select: () -> Void

    @State private var isHovering = false

    var body: some View {
        if let stored = head.hydra {
            // A running native head's note and counts live on its runtime, off the thread
            // record; this row is the one view that follows them.
            let info = HydraLiveInfo.shown(stored, model.existingRuntime(for: head.id))
            HStack(spacing: 10) {
                Button(action: select) {
                    HStack(spacing: 10) {
                        HydraGlyph(persona: info.persona, size: 22, isRunning: info.status == .running, status: info.status)
                        VStack(alignment: .leading, spacing: 2) {
                            HStack(spacing: 6) {
                                Text(verbatim: info.persona.name)
                                    .font(.system(size: 13, weight: .medium))
                                    .foregroundStyle(Chrome.primaryText)
                                // The provider's mark, so the list says whose model each
                                // head runs on.
                                ProviderIcon(provider: head.provider, size: 11)
                                    .foregroundStyle(Chrome.secondaryText)
                                    .help(model.providers.model(head.model, for: head.provider)?.shortName ?? head.provider.displayName)
                                Text(verbatim: HydraStatusText.long(info))
                                    .font(.system(size: 11))
                                    .foregroundStyle(info.status == .failed ? Chrome.danger : Chrome.secondaryText)
                                    .lineLimit(1)
                            }
                            Text(verbatim: info.task.isEmpty ? "Waiting for a task" : info.task)
                                .font(.system(size: 12))
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
                    HydraStopButton(name: info.persona.name) {
                        model.stopHydraHead(head.id)
                    }
                }
            }
            .padding(.horizontal, 8)
            .padding(.vertical, 5)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background {
                RoundedRectangle(cornerRadius: 6, style: .continuous)
                    .fill(isOnStage ? Chrome.overlay(0.14) : (isHovering ? Chrome.overlay(0.08) : Color.clear))
            }
            .onHover { hovering in
                withAnimation(Chrome.hover) { isHovering = hovering }
            }
            .accessibilityElement(children: .combine)
            .accessibilityLabel(Text("\(info.persona.name), \(HydraStatusText.long(info)), \(info.task)\(isOnStage ? ", on stage" : "")"))
        }
    }
}

/// The stop button beside a working head: the same tinted circle with the white stop
/// mark the composer shows while a turn runs.
private struct HydraStopButton: View {
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

/// A head's conversation, live as it works, with what it can be told below.
private struct HydraHeadTranscript: View {
    @Environment(AppModel.self) private var model
    let head: ChatThread
    let height: CGFloat
    let workingDirectory: String?
    let projectName: String?

    @State private var scrollChrome = ChromeScrollModel()
    @State private var scrollState = TimelineScrollState()

    var body: some View {
        let runtime = model.runtime(for: head.id)
        Group {
            if model.settings.hydraShowsHeadDetails {
                ThreadTimeline(
                    runtime: runtime,
                    scrollChrome: scrollChrome,
                    scrollState: scrollState,
                    projectName: projectName,
                    workingDirectory: workingDirectory,
                    supportsRewind: false,
                    columnHeight: height,
                    // A panel's transcript needs no rail, and the rail's hover tracking
                    // and geometry reader are per-frame work under the pointer.
                    showsMinimap: false
                )
                .equatable()
            } else {
                // The task and the progress bar stand in for the steps, which the
                // sidebar shows; the chat box below still steers the head.
                HydraHeadProgress(head: head, runtime: runtime)
            }
        }
        .safeAreaInset(edge: .bottom, spacing: 0) {
            if head.hydra?.kind == .droppy {
                // A head of Droppy Code's own can be steered and answered like any chat.
                ComposerArea(runtime: runtime, workingDirectory: workingDirectory, compactModelChip: true, takesFocusOnAppear: false, showsChanges: false)
                    .overlay(alignment: .top) {
                        JumpToLatestButton(scrollState: scrollState)
                    }
            } else if let stored = head.hydra {
                HydraHeadFooter(info: HydraLiveInfo.shown(stored, runtime))
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

/// What a native head is up to, in place of a chat box: its progress note or last tool,
/// its tool count and its time.
private struct HydraHeadFooter: View {
    @Environment(\.colorScheme) private var colorScheme
    let info: HydraHeadInfo

    var body: some View {
        HStack(spacing: 8) {
            if info.status == .running {
                MiniSpinner()
            } else {
                Image(systemName: info.status == .completed ? "checkmark.circle.fill" : (info.status == .failed ? "xmark.circle.fill" : "stop.circle.fill"))
                    .font(.system(size: 12))
                    .foregroundStyle(info.status == .completed ? Chrome.success : (info.status == .failed ? Chrome.danger : Chrome.secondaryText))
            }
            Text(verbatim: HydraStatusText.footer(info))
                .font(.system(size: 12))
                .foregroundStyle(Chrome.secondaryText)
                .lineLimit(1)
                .truncationMode(.tail)
            Spacer(minLength: 8)
            if info.toolCalls > 0 {
                Text(verbatim: info.toolCalls == 1 ? "1 step" : "\(info.toolCalls) steps")
                    .font(.system(size: 11).monospacedDigit())
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

/// A running head's record with the live progress the panel actually prints on top of it.
/// Deliberately not the runtime's own `hydraLiveInfo`, which also reads the live token
/// total: no line in the panel shows tokens, and a provider reports them many times a
/// second, so following them here would redraw the row and the footer on every token the
/// head spends. The note and the step count are shown, so those are read.
@MainActor
enum HydraLiveInfo {
    static func shown(_ stored: HydraHeadInfo, _ runtime: ThreadRuntime?) -> HydraHeadInfo {
        guard stored.status == .running, let runtime else { return stored }
        var info = stored
        if let activity = runtime.hydraActivity { info.activity = activity }
        info.toolCalls = max(info.toolCalls, runtime.hydraToolCalls)
        return info
    }
}

/// The words for a head's state.
enum HydraStatusText {
    static func short(_ info: HydraHeadInfo) -> String {
        switch info.status {
        case .running: "working"
        case .completed: "done"
        case .failed: "failed"
        case .stopped: "stopped"
        }
    }

    static func long(_ info: HydraHeadInfo) -> String {
        switch info.status {
        case .running:
            return info.activity.map { "· " + TextCleanup.singleLine($0, limit: 40) } ?? "· working"
        case .completed:
            return "· done" + elapsed(info) + steps(info) + (landed(info).map { " · " + $0 } ?? "")
        case .failed:
            return "· failed"
        case .stopped:
            return "· stopped"
        }
    }

    static func footer(_ info: HydraHeadInfo) -> String {
        switch info.status {
        case .running:
            return info.activity.map { TextCleanup.singleLine($0, limit: 80) } ?? "Working"
        case .completed:
            return "Done" + elapsed(info) + steps(info) + (landed(info).map { " · " + $0 } ?? "") + (info.summary.map { " · " + TextCleanup.singleLine($0, limit: 80) } ?? "")
        case .failed:
            return "Failed" + (info.summary.map { " · " + TextCleanup.singleLine($0, limit: 80) } ?? "")
        case .stopped:
            return "Stopped"
        }
    }

    /// Where a Droppy-run head's work went, in a few words; nil for a head with no copy
    /// of its own.
    static func landed(_ info: HydraHeadInfo) -> String? {
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

    private static func steps(_ info: HydraHeadInfo) -> String {
        guard info.toolCalls > 0 else { return "" }
        return info.toolCalls == 1 ? " · 1 step" : " · \(info.toolCalls) steps"
    }

    private static func elapsed(_ info: HydraHeadInfo) -> String {
        guard let finished = info.finishedAt else { return "" }
        let seconds = Int(finished.timeIntervalSince(info.startedAt))
        guard seconds > 0 else { return "" }
        return seconds < 60 ? " in \(seconds)s" : " in \(seconds / 60)m \(seconds % 60)s"
    }
}
