// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import AppKit
import SwiftUI

struct ComposerArea: View {
    @Environment(\.chatColumnIsShown) private var isColumnShown
    let runtime: ThreadRuntime
    /// The folder the thread works in, for the `@` file index. Read once by the chat and
    /// handed down, so the composer never observes the project itself.
    let workingDirectory: String?
    /// Floating panels are narrow: the model chip collapses to the provider's icon.
    var compactModelChip: Bool = false
    /// Whether the box takes typing focus as it appears; a floating panel's does not.
    var takesFocusOnAppear = true

    var body: some View {
        GlassEffectContainer(spacing: 8) {
            VStack(spacing: 12) {
                ForEach(runtime.approvals) { request in
                    ApprovalCard(request: request, runtime: runtime)
                }
                VStack(alignment: .center, spacing: -FollowUpQueueTab.overlap) {
                    TabSlot(runtime: runtime)
                    ComposerView(runtime: runtime, workingDirectory: workingDirectory, compactModelChip: compactModelChip, takesFocusOnAppear: takesFocusOnAppear)
                }
            }
        }
        .frame(maxWidth: Chrome.columnMaxWidth)
        .padding(.horizontal, 20)
        .padding(.bottom, 14)
        .frame(maxWidth: .infinity)
        .animation(Chrome.panelSlide, value: runtime.approvals.map(\.id))
    }
}

/// The tab above the pill: the queued follow-ups, while any are queued. The agent's
/// questions are badges in the conversation now, and the thread's changes live on the
/// right sidebar's Changes page. Reading the queue here keeps a prompt queued or sent
/// from re-running the area, whose body watches approvals alone.
private struct TabSlot: View {
    let runtime: ThreadRuntime

    var body: some View {
        // The slot exists only while the queue does, or the stack's overlap would
        // pull an empty slot's box up by that much. The tab is the view the slot
        // inserts and removes, so its own soft arrival and quick fade always run.
        if !runtime.followUps.isEmpty {
            FollowUpQueueTab(runtime: runtime)
                .transition(.softAppear)
        }
    }
}

struct ComposerView: View {
    @Environment(AppModel.self) private var model
    @Environment(\.chatColumnIsShown) private var isColumnShown
    @Environment(\.chatZoom) private var zoom
    /// Plain reference: the pill itself never binds the draft, so a keystroke only
    /// reaches the text column and the send button's draft reader below.
    let runtime: ThreadRuntime
    let workingDirectory: String?
    /// Floating panels are narrow: the model chip collapses to the provider's icon.
    var compactModelChip: Bool = false
    var takesFocusOnAppear = true

    @State private var controller = ComposerController()
    @State private var suggestions = SuggestionState()
    /// Escape closed the list; the next caret move may open it again.
    @State private var dismissedSuggestions = false
    @State private var historyIndex: Int?
    @State private var showingRecents = false
    @State private var fileIndex = FileIndex()
    /// Why an attach did not take everything it was handed. Shown over the text for a few
    /// seconds: files used to be dropped at the cap with nothing said about it.
    @State private var attachmentNotice: String?

    /// How many files one message carries.
    static let maxAttachments = Storage.attachmentLimit

    var body: some View {
        // Layout only: the resolved thread and help strings are plain values handed down,
        // so neither a keystroke nor a thread write re-evaluates the pill itself.
        let thread = model.thread(runtime.threadID)
        // One pill: the text on the left, the controls tucked into its trailing end. The
        // controls sit on the pill's bottom line, so they stay put while the text grows upward.
        HStack(alignment: .bottom, spacing: 8) {
            ComposerTextColumn(
                runtime: runtime,
                controller: controller,
                placeholder: placeholder(for: thread),
                takesFocusOnAppear: takesFocusOnAppear,
                attachmentNotice: attachmentNotice,
                onKey: handleKey,
                onAttach: attach,
                onCursorChange: cursorMoved(to:),
                onBlur: blur
            )
            // A single line of text is as tall as the send button, so an empty pill is 44 high.
            .padding(.vertical, 4)

            if let thread {
                ComposerControls(
                    runtime: runtime,
                    thread: thread,
                    compact: compactModelChip,
                    recentDownloadsPicker: model.settings.recentDownloadsPicker,
                    queueGoesToHand: queueGoesToHand(thread),
                    handsProvider: handsProvider(thread),
                    showingRecents: $showingRecents,
                    onAttach: { attach($0.map(AttachmentSource.file)) },
                    onChooseFiles: chooseFiles,
                    onSend: send
                )
            }
        }
        .padding(.leading, 14)
        .padding(.trailing, 8)
        .padding(.vertical, 8)
        // While a turn runs, accent dots flow along the pill's bottom two-fifths, behind
        // the text. They must live inside the glass content: a glass view renders in its
        // own pass above ordinary siblings, so glass as a separate background layer
        // covered the composer instead of sitting under it.
        .background {
            ComposerWorkingDotsSlot(runtime: runtime)
        }
        .modifier(ComposerSurface())
        // The pill's width glides as the panels take or give room; the text view and the
        // controls move as one with the glass rather than each on their own frame.
        .geometryGroup()
        .onChange(of: suggestions) { _, new in
            if new.isVisible {
                controller.showSuggestions(AnyView(suggestionMenu()), itemCount: new.items.count, zoom: zoom)
            } else {
                controller.hideSuggestions()
            }
        }
        .onDisappear { controller.hideSuggestions() }
        .onChange(of: runtime.threadID) { _, _ in controller.hideSuggestions() }
        // The column shown again takes the keyboard back, as it did when it first appeared;
        // one going behind another drops its list.
        .onChange(of: isColumnShown) { _, shown in
            if shown { if takesFocusOnAppear { controller.focus() } } else { controller.hideSuggestions() }
        }
        // The palette put a command in the box: the caret follows it there.
        .onChange(of: runtime.composerFocusRequest) { _, _ in controller.focus() }
        .task(id: commandKey) { await loadCommands() }
        .onChange(of: model.providers.commands[commandKey]) { _, _ in
            if controller.hasFocus { cursorMoved(to: controller.cursorLocation) }
        }
        .onChange(of: model.providers.commandErrors[commandKey]) { _, _ in
            if controller.hasFocus, suggestions.isVisible {
                controller.showSuggestions(AnyView(suggestionMenu()), itemCount: suggestions.items.count, zoom: zoom)
            }
        }
        .onChange(of: suggestions.isVisible) { _, visible in
            if visible, suggestions.kind == .command { Task { await loadCommands() } }
        }
        .task(id: workingDirectory) {
            if let workingDirectory { await fileIndex.prepare(workingDirectory) }
        }
        // The notice says its piece and goes; a second drop restarts the wait.
        .task(id: attachmentNotice) {
            guard attachmentNotice != nil else { return }
            try? await Task.sleep(for: .seconds(5))
            guard !Task.isCancelled else { return }
            withAnimation(Chrome.panelSlide) { attachmentNotice = nil }
        }
    }

    private func blur() {
        // Clicking a row focuses the popover; only dismiss when
        // focus truly left both the text and the suggestions.
        if !controller.isClickInsideSuggestions() {
            // A search still under way for the last keystroke would put
            // the popover back over a field that no longer has focus.
            controller.suggestionRefresh?.cancel()
            suggestions = SuggestionState()
        }
    }

    /// The slash/@ list hosted in a caret-anchored NSPopover (see ComposerController).
    @ViewBuilder
    private func suggestionMenu() -> some View {
        ScrollViewReader { proxy in
            PopoverMenu {
                PopoverSectionHeader(suggestions.kind == .command ? "Commands · Recently used first" : "Files")
                if suggestions.kind == .command, let error = model.providers.commandErrors[commandKey] {
                    PopoverNote(error)
                    PopoverItem("Retry loading commands", symbol: "arrow.clockwise") { Task { await loadCommands(force: true) } }
                }
                ForEach(suggestions.items.indices, id: \.self) { index in
                    let item = suggestions.items[index]
                    SuggestionPopoverRow(
                        item: item,
                        isSelected: index == suggestions.selected,
                        select: { suggestions.selected = index },
                        pick: { pick(item) }
                    )
                    .id(index)
                }
            }
            .onChange(of: suggestions.selected) { _, selected in proxy.scrollTo(selected) }
        }
        // AppKit hosts the list apart from the window (see `ComposerController`), so the
        // text size is handed over; the panel is sized for the rows at the same size.
        .environment(\.chatZoom, zoom)
    }

    private var commandKey: String {
        guard let thread = model.thread(runtime.threadID), let workingDirectory else { return "" }
        return ProviderRegistry.commandKey(thread.provider, directory: workingDirectory)
    }

    private func loadCommands(force: Bool = false) async {
        guard let thread = model.thread(runtime.threadID), let workingDirectory else { return }
        await model.providers.loadCommands(thread.provider, directory: workingDirectory, force: force)
        if controller.hasFocus { cursorMoved(to: controller.cursorLocation) }
    }

    private func placeholder(for thread: ChatThread?) -> String {
        guard let thread else { return "" }
        if thread.interactionMode == .plan { return "Describe what you want to plan" }
        if model.crewIsOn(thread) {
            return "Ask \(thread.provider.displayName) for anything; big jobs go out to a team of hands"
        }
        return "Ask \(thread.provider.displayName) to build, fix or explain. @ for files, / for commands"
    }

    /// Whether a message queued behind the running turn is handed to a hand of its own
    /// rather than waiting for the lead.
    private func queueGoesToHand(_ thread: ChatThread) -> Bool {
        model.settings.crewQueueHands && model.crewIsOn(thread)
    }

    /// The pair's hands provider when it differs from the lead's, so the usage popover
    /// can show both plans' limits at once. Nil keeps it to the chat's own provider.
    private func handsProvider(_ thread: ChatThread) -> ProviderKind? {
        guard model.crewIsOn(thread), let pair = model.crewPair(for: thread) else { return nil }
        let hands = model.crewHandsProvider(of: pair)
        return hands == thread.provider ? nil : hands
    }

    // MARK: - Keys

    private func handleKey(_ key: ComposerKey) -> Bool {
        if suggestions.isVisible {
            switch key {
            case .up:
                suggestions.selected = max(0, suggestions.selected - 1)
                return true
            case .down:
                suggestions.selected = min(suggestions.items.count - 1, suggestions.selected + 1)
                return true
            // With the list open, Return takes the highlighted row, as Tab and ⌘Return do:
            // the list is what the key is for, never a new line or a send.
            case .tab, .newline, .submit, .queue:
                pick(suggestions.items[suggestions.selected])
                return true
            case .escape:
                dismissedSuggestions = true
                suggestions = SuggestionState()
                return true
            case .deleteAtStart:
                return false
            }
        }
        switch key {
        case .submit:
            submit()
            return true
        case .queue:
            queue()
            return true
        case .newline:
            return false
        case .up:
            return recall(older: true)
        case .down:
            return recall(older: false)
        case .escape:
            // Never the running turn: a stray Escape cost the turn and its words. The stop
            // button and the stop chord stop it, and Stop and take back rewinds it. The panels
            // go the way ⌘J and ⌘D take them, running or not.
            if runtime.isTerminalVisible {
                runtime.isTerminalVisible = false
            } else if runtime.isDiffVisible {
                runtime.toggleDiff()
            } else {
                return false
            }
            return true
        case .tab:
            return false
        case .deleteAtStart:
            if runtime.draft.command != nil {
                withAnimation(Chrome.panelSlide) { runtime.draft.command = nil }
                return true
            }
            if !runtime.draft.quotes.isEmpty {
                withAnimation(Chrome.panelSlide) { _ = runtime.draft.quotes.removeLast() }
                return true
            }
            return false
        }
    }

    private func send() {
        guard !runtime.draft.isEmpty else { return }
        historyIndex = nil
        suggestions = SuggestionState()
        if runtime.isRunning {
            // Command-Return while the turn runs stops it and sends right away. With hands
            // at work it queues instead (a hand takes it, or it waits for the lead): the
            // hands run inside the lead's session, and a stop would kill them.
            runtime.interruptAndSend()
        } else {
            runtime.send()
        }
    }

    /// Command-Return. Words in the box go out (see `send`). With nothing typed and the
    /// agent waiting on a request, the one on top is approved, the card whose button the
    /// chord used to press: from the box that has the keyboard, so a hand's box answers its
    /// own hand and never the chat's card, and the chord can never approve while words wait.
    /// With the keyboard in no box, the window approves the chat's own (`ThreadWindow`).
    private func submit() {
        guard runtime.draft.isEmpty else {
            send()
            return
        }
        runtime.approveTopRequest()
    }

    /// The Queue a chat chord typed in this box: the draft lines up behind the running turn,
    /// or goes out when nothing runs (see `queueDraftAsFollowUp`). The box takes the chord
    /// before the menu does (`ComposerNSTextView`), so a hand's box queues its own draft
    /// rather than the chat's on screen.
    private func queue() {
        guard !runtime.draft.isEmpty else { return }
        historyIndex = nil
        suggestions = SuggestionState()
        runtime.queueDraftAsFollowUp()
    }

    private func recall(older: Bool) -> Bool {
        let prompts = runtime.sentPrompts.filter { !$0.isEmpty }
        guard !prompts.isEmpty, runtime.draft.attachments.isEmpty else { return false }
        let text = runtime.draft.text
        let recalledIndex = historyIndex.flatMap { prompts.indices.contains($0) && prompts[$0] == text ? $0 : nil }
        guard text.isEmpty || recalledIndex != nil else { return false }
        if older {
            let next = (recalledIndex ?? prompts.count) - 1
            guard next >= 0 else { return true }
            historyIndex = next
            runtime.draft.text = prompts[next]
        } else {
            guard let index = recalledIndex else { return false }
            if index + 1 < prompts.count {
                historyIndex = index + 1
                runtime.draft.text = prompts[index + 1]
            } else {
                historyIndex = nil
                runtime.draft.text = ""
            }
        }
        return true
    }

    // MARK: - Suggestions

    /// One refresh per keystroke. The text view reports a keystroke twice, as the change and
    /// as the caret's move, and each report scored the whole file index; the earlier refresh
    /// is dropped and the one that runs reads the caret where it is then.
    private func cursorMoved(to _: Int) {
        dismissedSuggestions = false
        if !runtime.draft.text.isEmpty { runtime.warmSession() }
        controller.suggestionRefresh?.cancel()
        controller.suggestionRefresh = Task { @MainActor in
            guard !dismissedSuggestions else { return }
            await refreshSuggestions(cursor: controller.cursorLocation)
        }
    }

    private func refreshSuggestions(cursor: Int) async {
        let text = runtime.draft.text as NSString
        guard cursor > 0, cursor <= text.length else {
            if suggestions.isVisible { suggestions = SuggestionState() }
            return
        }
        var start = cursor
        while start > 0 {
            let character = text.character(at: start - 1)
            if character == 0x20 || character == 0x0A || character == 0x09 { break }
            start -= 1
        }
        let range = NSRange(location: start, length: cursor - start)
        let word = text.substring(with: range)
        var kind = SuggestionState.Kind.file
        var items: [Suggestion] = []
        if word.hasPrefix("@") {
            let paths = await fileIndex.search(String(word.dropFirst()))
            // The caret moved on while the index was searched: the newer keystroke's refresh
            // is under way, and this answer is for a word that is gone.
            guard !Task.isCancelled else { return }
            items = paths.map { path in
                Suggestion(value: "@\(path) ", title: (path as NSString).lastPathComponent, detail: path, symbol: "doc")
            }
        } else if word.hasPrefix("/"), start == 0 {
            kind = .command
            items = commandSuggestions(matching: String(word.dropFirst()))
        }
        guard !items.isEmpty else {
            if suggestions.isVisible { suggestions = SuggestionState() }
            return
        }
        let selected = suggestions.range.location == range.location ? min(suggestions.selected, items.count - 1) : 0
        suggestions = SuggestionState(kind: kind, range: range, items: items, selected: selected)
    }

    private func commandSuggestions(matching query: String) -> [Suggestion] {
        guard let thread = model.thread(runtime.threadID) else { return [] }
        var commands = [SlashCommand(name: "plan", detail: "Turn plan mode on or off", isBuiltIn: true)]
        if thread.provider == .codex || thread.provider == .claude || thread.provider == .copilot || thread.provider == .deepseek || thread.provider == .meta || thread.provider == .zai || thread.provider == .pi {
            commands.append(SlashCommand(name: "compact", detail: "Summarize the conversation to free up context", isBuiltIn: true))
        }
        // The prompt macros on every provider, ahead of the provider's own list so a command
        // of its by the same name never stands in for one (see `PromptMacro`).
        commands += PromptMacro.allCases.map { SlashCommand(name: $0.rawValue, detail: $0.detail, isBuiltIn: true) }
        commands += model.providers.commands[commandKey] ?? []
        return SlashCommand.suggestions(commands, matching: query, recent: model.providers.recentCommands[thread.provider.rawValue] ?? []).map { command in
            Suggestion(value: "/\(command.name) ", title: "/\(command.name)", detail: command.detail, symbol: command.isBuiltIn ? "command" : "sparkles", command: command)
        }
    }

    private func pick(_ suggestion: Suggestion) {
        let range = suggestions.range
        let wasCommand = suggestions.kind == .command
        suggestions = SuggestionState()
        if wasCommand, let c = suggestion.command {
            controller.replaceCharacters(in: range, with: "")
            withAnimation(Chrome.panelSlide) {
                runtime.draft.command = DraftCommand(name: c.name, detail: c.detail, isBuiltIn: c.isBuiltIn)
            }
            controller.focus()
            return
        }
        controller.replaceCharacters(in: range, with: suggestion.value)
        controller.focus()
    }

    // MARK: - Attachments

    /// Files and images go through `Storage.attach`: the slots are counted here, the
    /// copies and transcodes run off the main actor, and what lands past the cap is said.
    private func attach(_ sources: [AttachmentSource]) {
        let room = max(0, Self.maxAttachments - runtime.draft.attachments.count)
        let dropped = sources.count - min(sources.count, room)
        Storage.attach(sources, to: Bindable(runtime).draft.attachments)
        if dropped > 0 {
            let isImage = sources.count == 1 && { if case .image = sources[0] { return true }; return false }()
            note(isImage
                ? "The image was left out: a message holds \(Self.maxAttachments) attachments"
                : dropped == 1
                    ? "One file was left out: a message holds \(Self.maxAttachments) attachments"
                    : "\(dropped) files were left out: a message holds \(Self.maxAttachments) attachments")
        }
        controller.focus()
    }

    private func note(_ message: String) {
        withAnimation(Chrome.panelSlide) { attachmentNotice = message }
    }

    private func chooseFiles() {
        let panel = NSOpenPanel()
        panel.allowsMultipleSelection = true
        panel.canChooseDirectories = false
        panel.prompt = "Attach"
        guard panel.runModal() == .OK else { return }
        attach(panel.urls.map(AttachmentSource.file))
    }
}

// MARK: - Pill

/// The pill's text side: the attachment strip, the notice and the text view. It owns the
/// draft-text binding and the growth math, so a keystroke never reaches the pill itself.
private struct ComposerTextColumn: View {
    @Environment(WindowLiveResize.self) private var liveResize
    @Environment(\.chatZoom) private var zoom
    @Bindable var runtime: ThreadRuntime
    let controller: ComposerController
    let placeholder: String
    let takesFocusOnAppear: Bool
    let attachmentNotice: String?
    let onKey: (ComposerKey) -> Bool
    let onAttach: ([AttachmentSource]) -> Void
    let onCursorChange: (Int) -> Void
    let onBlur: () -> Void

    @State private var textHeight: CGFloat = 20

    private static let minComposerHeight: CGFloat = 20
    private static let maxComposerLines = 5
    /// Five lines of the box's own font at the window's text size (see
    /// `ComposerTextView.font(zoom:)`), so the cap grows with the type it holds.
    private static func maxComposerHeight(zoom: CGFloat) -> CGFloat {
        let font = ComposerTextView.font(zoom: zoom)
        let line = ceil(font.ascender - font.descender + font.leading)
        // Matches ComposerTextView's textContainerInset.height * 2.
        return line * CGFloat(maxComposerLines) + 4
    }

    private var composerHeight: CGFloat {
        min(max(textHeight, ChatZoom.scaled(Self.minComposerHeight, by: zoom)), Self.maxComposerHeight(zoom: zoom))
    }

    var body: some View {
        // Spacing 0 and the strip carries its own gap: the strip reads the draft
        // itself, so it stays a zero-height subview while there is nothing attached
        // and the composer never has to look at the draft to decide whether to
        // stack it. Reading the draft here would re-render the text view on every
        // keystroke, since text and attachments are one observable property.
        VStack(alignment: .leading, spacing: 0) {
            DraftChips(runtime: runtime)
            DraftAttachments(runtime: runtime)
            if let attachmentNotice {
                Text(verbatim: attachmentNotice)
                    .zoomedFont(size: 11)
                    .foregroundStyle(Chrome.warning)
                    .lineLimit(1)
                    .padding(.bottom, 6)
                    .transition(.softAppear)
            }
            ComposerTextView(
                text: $runtime.draft.text,
                height: $textHeight,
                placeholder: placeholder,
                controller: controller,
                onKey: onKey,
                onAttach: onAttach,
                onCursorChange: onCursorChange,
                onBlur: onBlur,
                takesFocusOnAppear: takesFocusOnAppear
            )
            // The text view itself takes its new height at once, so it lays out
            // every line with nothing to scroll; only the clip around it grows
            // and shrinks. Animating the scroll view's own frame made AppKit
            // scroll the caret into a too-short view and then snap back.
            .frame(height: composerHeight)
            .transaction { $0.animation = nil }
            .frame(height: composerHeight, alignment: .top)
            .clipped()
            // While the window is being dragged the box's height follows the rewrap at once; a spring restarted per frame lagged it behind the window.
            .animation(liveResize.isReshaping ? nil : .smooth(duration: 0.28), value: composerHeight)
        }
    }
}

/// The pill's trailing controls: the attach button, the model chip, the context ring
/// and send. It takes the resolved thread and help values as plain values, so a thread
/// write repaints the controls without reaching the pill or the text.
private struct ComposerControls: View {
    let runtime: ThreadRuntime
    let thread: ChatThread
    let compact: Bool
    let recentDownloadsPicker: Bool
    let queueGoesToHand: Bool
    let handsProvider: ProviderKind?
    @Binding var showingRecents: Bool
    let onAttach: ([URL]) -> Void
    let onChooseFiles: () -> Void
    let onSend: () -> Void

    var body: some View {
        HStack(spacing: 2) {
            Button {
                if recentDownloadsPicker {
                    showingRecents.toggle()
                } else {
                    onChooseFiles()
                }
            } label: {
                Image(systemName: "paperclip")
                    .font(Chrome.iconFont)
            }
            .buttonStyle(.chip(active: showingRecents))
            .focusable(false)
            .help("Attach files")
            .accessibilityLabel(Text("Attach files"))
            .popover(isPresented: $showingRecents, arrowEdge: .bottom) {
                DownloadsPopover(
                    pick: { showingRecents = false; onAttach([$0]) },
                    chooseOther: { showingRecents = false; onChooseFiles() }
                )
            }
            ModelEffortSlot(runtime: runtime, thread: thread, compact: compact)
            // A floating panel's box (compact chip) leaves the ring out: a hand's context
            // is not what the panel is for, and the row is short of room as it is.
            if !compact {
                ContextMeter(runtime: runtime, provider: thread.provider, handsProvider: handsProvider)
            }
            SendButton(runtime: runtime, queueGoesToHand: queueGoesToHand, send: onSend)
        }
        .fixedSize()
    }
}

/// The working dots behind the text. Reading `isRunning` here keeps the pill's body
/// off the run flag, so starting and stopping a turn repaints the dots alone.
private struct ComposerWorkingDotsSlot: View {
    let runtime: ThreadRuntime

    var body: some View {
        if runtime.isRunning {
            GeometryReader { proxy in
                ComposerWorkingDots()
                    .frame(height: proxy.size.height * 0.4)
                    .frame(maxHeight: .infinity, alignment: .bottom)
            }
            .clipShape(.rect(cornerRadius: Chrome.composerCornerRadius, style: .continuous))
            .transition(.opacity.animation(.easeInOut(duration: 0.4)))
        }
    }
}

// MARK: - Controls

/// The model and effort chip. It reads the thread's turns itself rather than taking a flag
/// from the composer: a turn record is rewritten several times per turn (its checkpoints,
/// its anchor, its diff), and the composer would re-render whole for each of them.
private struct ModelEffortSlot: View {
    let runtime: ThreadRuntime
    let thread: ChatThread
    let compact: Bool

    var body: some View {
        ModelEffortButton(thread: thread, hasHistory: !runtime.turns.isEmpty, compact: compact)
    }
}

/// The context ring. Clicking it opens the context window and whatever else the provider
/// reports: its plan's usage limits, or a pay-as-you-go key's remaining credits.
///
/// Usage is read here rather than handed down, so a usage report repaints the ring alone.
private struct ContextMeter: View {
    @Environment(AppModel.self) private var model
    let runtime: ThreadRuntime
    let provider: ProviderKind
    let handsProvider: ProviderKind?

    @State private var isPresented = false

    var body: some View {
        let usage = runtime.usage
        let fraction = usage?.fraction
        // A key with credit behind it has something to show from the first turn, before any
        // usage has been reported, so the ring is not waiting on a context window to exist.
        let hasCredits = CreditsReader.exposesCredits(provider) && model.settings.hasAPIKey(for: provider)
        // Popped out as a floating panel, the usage is already on screen beside the chat:
        // the ring goes until that panel is dismissed, which brings it back here.
        let isPoppedOut = model.settings.showsUsagePanel
        if !isPoppedOut, fraction != nil || PlanLimitsReader.exposesLimits(provider) || hasCredits {
            Button {
                isPresented.toggle()
            } label: {
                ZStack {
                    Circle()
                        .stroke(.quaternary, lineWidth: 2.5)
                    Circle()
                        .trim(from: 0, to: fraction ?? 0)
                        .stroke((fraction ?? 0) > 0.85 ? AnyShapeStyle(Chrome.warning) : AnyShapeStyle(.tint), style: StrokeStyle(lineWidth: 2.5, lineCap: .round))
                        .rotationEffect(.degrees(-90))
                }
                .frame(width: 15, height: 15)
                .padding(.horizontal, 6)
                .padding(.vertical, 5)
                .contentShape(.rect)
            }
            .buttonStyle(.plain)
            .focusable(false)
            .help(helpText(fraction: fraction, hasCredits: hasCredits))
            .accessibilityLabel(Text(label(fraction: fraction, hasCredits: hasCredits)))
            // The ring is the only reading of the context window on screen, so the number
            // behind it is spoken rather than left to the tooltip.
            .accessibilityValue(Text(fraction.map { "\(Int($0 * 100))% of the context window used" } ?? ""))
            .popover(isPresented: $isPresented, arrowEdge: .bottom) {
                VStack(spacing: 0) {
                    // The same limits as a floating panel beside every chat (see
                    // `UsageFloatingPanel`); the panel then stays until dismissed or
                    // switched off in Settings, and this button is where it comes back
                    // from, so it is always here.
                    HStack {
                        Text("Usage")
                            .zoomedFont(size: 12, weight: .semibold)
                            .foregroundStyle(Chrome.secondaryText)
                        Spacer(minLength: 8)
                        ChromeCircleButton(symbol: "arrow.up.right", help: "Open as a floating panel beside the chat") {
                            isPresented = false
                            withAnimation(Chrome.panelSlide) {
                                model.settings.showsUsagePanel = true
                            }
                        }
                    }
                    .padding(.top, 10)
                    .padding(.horizontal, 16)
                    UsagePanel(usage: usage, provider: provider, handsProvider: handsProvider)
                }
                .presentedChrome()
            }
        }
    }

    private func helpText(fraction: Double?, hasCredits: Bool) -> String {
        if let fraction { return "\(Int(fraction * 100))% of the context window used" }
        return hasCredits ? "Remaining credits" : "Usage limits"
    }

    private func label(fraction: Double?, hasCredits: Bool) -> String {
        if fraction != nil { return "Context window" }
        return hasCredits ? "Remaining credits" : "Usage limits"
    }
}

/// The send shortcuts as plain text, cached once per process: the help reads them without
/// touching the shortcut store on every render, so a keystroke never pays for a lookup.
@MainActor
private enum SendShortcutText {
    /// The box's own send, which no command may take: fixed, not a setting.
    static let send = KeyChord.composerSend.description
    static let queue = ShortcutStore.label(for: .queueChat)
    static let stop = ShortcutStore.hint(for: .stopTurn)
}

private struct SendButton: View {
    let runtime: ThreadRuntime
    /// With hands at work, Command-Return never stops the lead: the message goes to a hand of
    /// its own when queued messages do, else it waits for the lead, and the help says which.
    let queueGoesToHand: Bool
    let send: () -> Void

    var body: some View {
        SendRunState(runtime: runtime, queueGoesToHand: queueGoesToHand, send: send)
    }
}

/// The send/run state reader: watches `isRunning` and hands state, so starting and
/// stopping a turn repaint the button without ever looking at the draft text.
private struct SendRunState: View {
    let runtime: ThreadRuntime
    let queueGoesToHand: Bool
    let send: () -> Void

    var body: some View {
        let isRunning = runtime.isRunning
        let handsWorking = isRunning && runtime.hasWorkingHands
        // The queue tail of the idle help depends on hands working, which this reader
        // already watches; the empty state comes from the child below alone.
        SendDraftState(runtime: runtime, isRunning: isRunning, handsWorking: handsWorking, queueGoesToHand: queueGoesToHand, send: send)
    }
}

/// The draft-empty reader: watches `draft.isEmpty` alone, so a keystroke toggles the
/// button without re-running the hands state or the help text above it.
private struct SendDraftState: View {
    let runtime: ThreadRuntime
    let isRunning: Bool
    let handsWorking: Bool
    let queueGoesToHand: Bool
    let send: () -> Void

    var body: some View {
        let draftEmpty = runtime.draft.isEmpty
        let isEnabled = isRunning || !draftEmpty
        Button {
            if isRunning { runtime.interrupt() } else { send() }
        } label: {
            Image(systemName: isRunning ? "stop.fill" : "arrow.up")
                .font(Chrome.iconFont)
                .foregroundStyle(.white)
                .contentTransition(.symbolEffect(.replace))
                .frame(width: 28, height: 28)
                .background(isEnabled ? AnyShapeStyle(.tint) : AnyShapeStyle(.quaternary), in: .circle)
        }
        .buttonStyle(.plain)
        .focusable(false)
        .disabled(!isEnabled || runtime.isReverting)
        .padding(.leading, 4)
        .help(helpText(isRunning: isRunning, handsWorking: handsWorking))
        .accessibilityLabel(Text(isRunning ? "Stop" : "Send"))
    }

    /// The chords come from the one cached lookup above, so the help never promises
    /// a key the store no longer answers to within a launch.
    private func helpText(isRunning: Bool, handsWorking: Bool) -> String {
        let queues = SendShortcutText.queue
        let sends = SendShortcutText.send
        if isRunning {
            // The send chord does not steer while hands work, it queues: the help says where
            // the message lands rather than promising a stop that never comes.
            if handsWorking {
                return queueGoesToHand
                    ? "Hands are at work: your message goes to a hand"
                    : "Hands are at work: your message waits for the lead"
            }
            var parts = ["Stop\(SendShortcutText.stop)", "\(sends) steers"]
            if let queues { parts.append("\(queues) queues") }
            return parts.joined(separator: " · ")
        }
        var parts = ["Send (\(sends))"]
        if let queues { parts.append(handsWorking ? "\(queues) queues until the hands report" : "\(queues) queues while running") }
        return parts.joined(separator: " · ")
    }
}

private struct DraftChips: View {
    let runtime: ThreadRuntime

    private struct ChipKey: Equatable {
        var quoteIDs: [UUID]
        var commandName: String?
    }

    var body: some View {
        let quotes = runtime.draft.quotes
        let command = runtime.draft.command
        if !quotes.isEmpty || command != nil {
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 6) {
                    if let command {
                        HStack(spacing: 5) {
                            Image(systemName: command.isBuiltIn ? "command" : "sparkles")
                                .zoomedFont(size: 10, weight: .semibold)
                                .foregroundStyle(Chrome.secondaryText)
                            Text(verbatim: "/" + command.name)
                                .zoomedFont(size: 11, weight: .medium)
                                .foregroundStyle(Chrome.primaryText.opacity(0.9))
                            Button {
                                withAnimation(Chrome.panelSlide) {
                                    runtime.draft.command = nil
                                }
                            } label: {
                                Image(systemName: "xmark")
                                    .zoomedFont(size: 9, weight: .bold)
                                    .foregroundStyle(Chrome.secondaryText)
                            }
                            .buttonStyle(.plain)
                            .focusable(false)
                            .help("Remove command")
                            .accessibilityLabel(Text("Remove command"))
                        }
                        .modifier(ChipChrome())
                        .help(command.detail)
                        .accessibilityElement(children: .combine)
                    }
                    ForEach(quotes) { quote in
                        HStack(spacing: 5) {
                            Image(systemName: "text.quote")
                                .zoomedFont(size: 10, weight: .semibold)
                                .foregroundStyle(Chrome.secondaryText)
                            Text(verbatim: quote.excerpt)
                                .zoomedFont(size: 11)
                                .foregroundStyle(Chrome.primaryText.opacity(0.9))
                                .lineLimit(1)
                                .truncationMode(.tail)
                                .frame(maxWidth: 260, alignment: .leading)
                            Button {
                                withAnimation(Chrome.panelSlide) {
                                    runtime.draft.quotes.removeAll { $0.id == quote.id }
                                }
                            } label: {
                                Image(systemName: "xmark")
                                    .zoomedFont(size: 9, weight: .bold)
                                    .foregroundStyle(Chrome.secondaryText)
                            }
                            .buttonStyle(.plain)
                            .focusable(false)
                            .help("Remove quote")
                            .accessibilityLabel(Text("Remove quote"))
                        }
                        .modifier(ChipChrome())
                        .help(quote.text)
                        .accessibilityElement(children: .combine)
                    }
                }
            }
            .padding(.bottom, 8)
            .transition(.modifier(active: ChipRowCollapse(collapsed: true), identity: ChipRowCollapse(collapsed: false)))
            .animation(Chrome.panelSlide, value: ChipKey(quoteIDs: quotes.map(\.id), commandName: command?.name))
        }
    }
}

/// The chip row's arrival and departure: it opens from and closes to nothing, clipped
/// from the bottom and fading, on the transaction's own animation. A fade in place
/// left the row hanging over the text view as that slid up into its room, so the
/// placeholder seemed to fade in through the chip.
private struct ChipRowCollapse: ViewModifier {
    let collapsed: Bool

    func body(content: Content) -> some View {
        content
            .frame(height: collapsed ? 0 : nil, alignment: .top)
            .clipped()
            .opacity(collapsed ? 0 : 1)
    }
}

/// The capsule behind a composer chip: the reply quotes and the slash command in the chat box, and the same quotes again on the message once it is sent.
struct ChipChrome: ViewModifier {
    func body(content: Content) -> some View {
        content
            .padding(.leading, 9)
            .padding(.trailing, 7)
            .padding(.vertical, 5)
            .background(Capsule(style: .continuous).fill(Chrome.overlay(0.12)))
    }
}

private struct DraftAttachments: View {
    let runtime: ThreadRuntime

    /// Same shape as the chat strip: one preview panel for the strip, so every
    /// draft photo opens in a single tap.
    @State private var preview = AttachmentPreviewSlot()
    @State private var drawn: Set<Attachment.ID> = []

    var body: some View {
        let attachments = runtime.draft.attachments
        if !attachments.isEmpty {
            strip(attachments)
                // The strip's own gap to the text below it: the composer stacks with no
                // spacing, so an empty strip takes no room at all.
                .padding(.bottom, 8)
        }
    }

    private func strip(_ attachments: [Attachment]) -> some View {
        ScrollView(.horizontal, showsIndicators: false) {
            // The delete badge straddles the thumbnail's far top-right corner.
            // The cell's own top/trailing padding reserves that overhang, so the
            // badge sits inside its own cell — never in the gap where a later
            // sibling could cover it — and every photo stays deletable. On top
            // only the 2 pt the badge's circle shows above the photo: the pill
            // already leaves 12 above the strip, and 12 + 2 puts the photo 14
            // from the pill's top edge, the same inset it has from the leading
            // edge, where it lines up with the text.
            HStack(spacing: 0) {
                ForEach(attachments) { attachment in
                    AttachmentArrival(isNew: !drawn.contains(attachment.id)) {
                        AttachmentThumbnail(attachment: attachment, size: 48, preview: preview)
                            .overlay(alignment: .topTrailing) {
                                Button {
                                    runtime.draft.attachments.removeAll { $0.id == attachment.id }
                                } label: {
                                    Image(systemName: "xmark.circle.fill")
                                        .symbolRenderingMode(.palette)
                                        .foregroundStyle(.white, .black.opacity(0.6))
                                        .padding(4)
                                }
                                .buttonStyle(.plain)
                                .focusable(false)
                                .offset(x: 6, y: -6)
                                .accessibilityLabel(Text("Remove \(attachment.name)"))
                            }
                            .padding(.top, 2)
                            .padding(.trailing, 8)
                    }
                    .onAppear { drawn.insert(attachment.id) }
                }
            }
            .background {
                AttachmentAnchorCapture { preview.setAnchor($0) }
            }
        }
        .onChange(of: attachments) {
            preview.retire(except: Set(attachments.map(\.id)))
        }
        .onDisappear { preview.close() }
    }
}

// MARK: - Suggestions

struct SuggestionState: Equatable {
    enum Kind {
        case file
        case command
    }

    var kind: Kind = .file
    var range = NSRange(location: 0, length: 0)
    var items: [Suggestion] = []
    var selected = 0

    var isVisible: Bool { !items.isEmpty }
}

struct Suggestion: Identifiable, Equatable {
    var value: String
    var title: String
    var detail: String
    var symbol: String
    var command: SlashCommand? = nil

    var id: String { value }
}

private struct SuggestionPopoverRow: View {
    let item: Suggestion
    let isSelected: Bool
    let select: () -> Void
    let pick: () -> Void

    @Environment(\.chatZoom) private var zoom
    @State private var isHovering = false

    var body: some View {
        Button(action: pick) {
            HStack(spacing: 8) {
                Image(systemName: item.symbol)
                    .zoomedFont(size: 12)
                    .frame(width: ChatZoom.scaled(16, by: zoom))
                Text(verbatim: item.title)
                    .zoomedFont(size: 13)
                    .lineLimit(1)
                    .truncationMode(.middle)
                Spacer(minLength: 16)
                Text(verbatim: item.detail)
                    .zoomedFont(size: 12)
                    .foregroundStyle(Chrome.secondaryText)
                    .lineLimit(1)
            }
            .foregroundStyle(Chrome.primaryText)
            .padding(.horizontal, 8)
            .frame(height: ChatZoom.scaled(ComposerController.suggestionRowHeight, by: zoom))
            .background {
                RoundedRectangle(cornerRadius: 6, style: .continuous)
                    .fill(isSelected || isHovering ? Chrome.overlay(0.1) : Color.clear)
            }
            .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .focusable(false)
        .onHover { hovering in
            withAnimation(Chrome.hover) { isHovering = hovering }
            if hovering { select() }
        }
    }
}

/// Paths in the working directory for `@` mentions.
@MainActor
@Observable
final class FileIndex {
    nonisolated private struct Entry: Sendable {
        var path: String
        var lowercased: String
        var name: String
        /// How much the path's length costs its score, taken at build time: a Swift string's
        /// count walks the whole string, and the search used to take it for every match.
        var lengthPenalty: Int
        /// Whether the path is plain ASCII, so the scan can match on its bytes.
        var isASCII: Bool
    }

    @ObservationIgnored private var entries: [Entry] = []
    @ObservationIgnored private var directory: String?

    /// Built indexes per directory, shared by every composer, so switching threads in a project
    /// never lists and lowercases its files again. A stale index is served and rebuilt behind it.
    /// A composer that finds its directory already building joins that build rather than
    /// waiting on nothing; two builds run at most, and two indexes are kept.
    private static var built = RecentCache<String, (entries: [Entry], builtAt: Date)>(limit: 2)
    private static var building: [String: Task<[Entry], Never>] = [:]
    private static let freshness: TimeInterval = 60

    func prepare(_ directory: String) async {
        guard !Task.isCancelled else { return }
        self.directory = directory
        while !Task.isCancelled, self.directory == directory {
            if let cached = Self.built.value(for: directory) {
                entries = cached.entries
                guard Date.now.timeIntervalSince(cached.builtAt) > Self.freshness else { return }
            } else {
                entries = []
            }
            let task: Task<[Entry], Never>
            if let pending = Self.building[directory] {
                task = pending
            } else if Self.building.count >= 2, let pending = Self.building.values.first {
                _ = await pending.value
                continue
            } else {
                task = Task {
                    let fresh = await Self.build(directory)
                    Self.built.insert((fresh, .now), for: directory)
                    Self.building.removeValue(forKey: directory)
                    return fresh
                }
                Self.building[directory] = task
            }
            let fresh = await task.value
            guard !Task.isCancelled, self.directory == directory else { return }
            entries = fresh
            return
        }
    }

    @concurrent
    private nonisolated static func build(_ directory: String) async -> [Entry] {
        await list(directory).map { path in
            let lowercased = path.lowercased()
            return Entry(
                path: path,
                lowercased: lowercased,
                name: (lowercased as NSString).lastPathComponent,
                lengthPenalty: min(path.count, 99),
                isASCII: lowercased.utf8.allSatisfy { $0 < 128 }
            )
        }
    }

    /// The best matches for a query. Scored off the main actor: the index holds up to
    /// 60,000 paths and every keystroke after an `@` scored each of them, which held the
    /// main thread for as long as that took in a large project. The caller drops the answer
    /// when the caret has moved on (the task is cancelled), so the scan stops early too.
    func search(_ query: String, limit: Int = 8) async -> [String] {
        let needle = query.lowercased()
        guard !needle.isEmpty else { return entries.prefix(limit).map(\.path) }
        return await Self.search(needle, in: entries, limit: limit)
    }

    @concurrent
    private nonisolated static func search(_ needle: String, in entries: [Entry], limit: Int) async -> [String] {
        // The best `limit` so far, highest score first. An insertion into a list this short
        // beats collecting every match and sorting them all.
        guard limit > 0 else { return [] }
        var best: [(path: String, score: Int)] = []
        best.reserveCapacity(min(limit, entries.count) + 1)
        let isASCII = needle.utf8.allSatisfy { $0 < 128 }
        for (offset, entry) in entries.enumerated() {
            if offset % 2_048 == 0, Task.isCancelled { return [] }
            let ascii = isASCII && entry.isASCII
            let score: Int
            if entry.name == needle {
                score = 1_000
            } else if entry.name.hasPrefix(needle) {
                score = 800
            } else if contains(needle, in: entry.name, ascii: ascii) {
                score = 600
            } else if contains(needle, in: entry.lowercased, ascii: ascii) {
                score = 400
            } else if isSubsequence(needle, of: entry.lowercased, ascii: ascii) {
                score = 100
            } else {
                continue
            }
            let scored = score - entry.lengthPenalty
            if best.count == limit, scored <= best[limit - 1].score { continue }
            let index = best.firstIndex { $0.score < scored } ?? best.count
            best.insert((entry.path, scored), at: index)
            if best.count > limit { best.removeLast() }
        }
        return best.map(\.path)
    }

    /// Substring search on the bytes when both sides are ASCII: `String.contains` walks
    /// graphemes, and it ran twice per entry per keystroke, which was most of a scan. A
    /// carriage return in the path is the one byte a grapheme can span, so it takes the
    /// slow path, as in `isSubsequence`.
    private nonisolated static func contains(_ needle: String, in haystack: String, ascii: Bool) -> Bool {
        guard ascii else { return haystack.contains(needle) }
        let found: Bool? = haystack.utf8.withContiguousStorageIfAvailable { bytes in
            needle.utf8.withContiguousStorageIfAvailable { pattern -> Bool? in
                guard let base = bytes.baseAddress, let start = pattern.baseAddress else { return nil }
                if memchr(base, 13, bytes.count) != nil { return nil }
                return memmem(base, bytes.count, start, pattern.count) != nil
            } ?? nil
        } ?? nil
        return found ?? haystack.contains(needle)
    }

    nonisolated static func isSubsequence(_ needle: String, of haystack: String, ascii: Bool) -> Bool {
        if ascii {
            var remaining = needle.utf8.makeIterator()
            var next = remaining.next()
            for byte in haystack.utf8 {
                if byte == 13 { return isSubsequence(needle, of: haystack, ascii: false) }
                if byte == next {
                    next = remaining.next()
                    if next == nil { return true }
                }
            }
            return next == nil
        }
        var remaining = needle[...]
        for character in haystack where character == remaining.first {
            remaining = remaining.dropFirst()
            if remaining.isEmpty { return true }
        }
        return remaining.isEmpty
    }

    @concurrent
    private nonisolated static func list(_ directory: String) async -> [String] {
        let git = Git(directory)
        if await git.isRepository() {
            let files = await git.listFiles()
            if !files.isEmpty { return Array(files.prefix(60_000)) }
        }
        return enumerate(directory)
    }

    private nonisolated static func enumerate(_ directory: String) -> [String] {
        let root = URL(fileURLWithPath: directory)
        let skipped: Set<String> = ["node_modules", "build", "dist", ".build", "DerivedData", "Pods", "target"]
        guard let enumerator = FileManager.default.enumerator(
            at: root,
            includingPropertiesForKeys: [.isDirectoryKey],
            options: [.skipsHiddenFiles, .skipsPackageDescendants]
        ) else { return [] }
        var results: [String] = []
        while let url = enumerator.nextObject() as? URL {
            // Nothing a mention would want sits that deep, and a tree that deep is usually
            // a vendored or generated one whose name the list above does not know.
            if skipped.contains(url.lastPathComponent) || enumerator.level > 16 {
                enumerator.skipDescendants()
                continue
            }
            if (try? url.resourceValues(forKeys: [.isDirectoryKey]))?.isDirectory == true { continue }
            results.append(ToolTitles.relativePath(url.path, to: directory))
            if results.count >= 20_000 { break }
        }
        return results
    }
}

/// The chat box's surface: Liquid Glass, or, inside a floating glass panel (a helper's,
/// a hand's), the panel's flat control fill, since glass on the panel's glass sampled
/// the same pixels twice on every frame the transcript moved under it.
private struct ComposerSurface: ViewModifier {
    @Environment(\.isOnGlassPanel) private var isOnGlassPanel
    @Environment(\.colorScheme) private var colorScheme

    func body(content: Content) -> some View {
        let shape = RoundedRectangle(cornerRadius: Chrome.composerCornerRadius, style: .continuous)
        if isOnGlassPanel {
            content
                .background(shape.fill(Chrome.panelControlFill(isDark: colorScheme == .dark)))
                .clipShape(shape)
        } else {
            content.glassEffect(.regular, in: shape)
        }
    }
}
