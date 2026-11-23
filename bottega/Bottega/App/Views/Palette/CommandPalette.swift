// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import SwiftUI

struct CommandPalette: View {
    @Environment(AppModel.self) private var model

    @Environment(\.colorScheme) private var colorScheme
    /// The window's text size: the rows' icon column grows with their type.
    @Environment(\.chatZoom) private var zoom

    @State private var query = ""
    @State private var selection = 0
    /// The rows for the query as the library stands: built when either changes, not on
    /// every arrow key or selection change.
    @State private var items: [Item] = []
    @FocusState private var isFocused: Bool

    /// The palette's size and place, how many rows it lists and how a row answers the
    /// query, in one place to tune.
    private enum Metrics {
        static let width: CGFloat = 600
        /// How far below the window's top edge the panel sits.
        static let topInset: CGFloat = 72
        /// The wash over the window behind the panel. A dark window needs more of one than a
        /// light window to push the palette forward; a single fixed value read as nothing at
        /// all in dark themes.
        static let scrimDark = 0.24
        static let scrimLight = 0.08
        static let rowCornerRadius: CGFloat = 12
        /// Threads listed with nothing typed (the most recent) and for a query (the best matches).
        static let recentThreads = 6
        static let matchingThreads = 10
        /// Rows the panel lists at most, everything together.
        static let maxRows = 12

        /// Whether a row answers the query: the query, lowercased, inside any of its texts.
        /// A plain substring, so what matches is always what was typed.
        static func matches(_ needle: String, _ texts: String?...) -> Bool {
            needle.isEmpty || texts.contains { $0?.lowercased().contains(needle) == true }
        }
    }

    private struct ResultsKey: Equatable {
        var query: String
        var threads: [ChatThread]
        /// The projects with their scripts, so a script added, renamed or dropped shows here.
        var projects: [Project]
        /// The global scripts, listed ahead of each project's (see `AppModel.scripts(for:)`).
        var globalScripts: [ProjectScript]
        var selectedThreadID: UUID?
        var finishActionTitle: String
        /// The chat on screen's own state the actions read: whether a turn runs (Stop, Take
        /// back), whether its draft holds anything to queue, and its provider's commands.
        var isRunning: Bool
        var draftIsEmpty: Bool
        var commands: [SlashCommand]
    }

    private struct Item: Identifiable {
        let id: String
        let title: String
        var subtitle: String?
        var icon: ProjectIcon?
        let symbol: String
        var shortcut: String?
        /// Words the row answers to beyond its title and subtitle, never shown: a prompt
        /// macro's command, so typing `/sop` finds it as it does in the chat box.
        var terms: String?
        let perform: () -> Void
    }

    var body: some View {
        ZStack(alignment: .top) {
            Color.black.opacity(colorScheme == .dark ? Metrics.scrimDark : Metrics.scrimLight)
                .ignoresSafeArea()
                .contentShape(.rect)
                .onTapGesture { close() }
                .accessibilityHidden(true)
            VStack(spacing: 0) {
                HStack(spacing: 10) {
                    Image(systemName: "magnifyingglass")
                        .zoomedFont(.body)
                        .foregroundStyle(.secondary)
                    TextField("Search threads, projects, actions and commands", text: $query)
                        .textFieldStyle(.plain)
                        .zoomedFont(.title3)
                        .focused($isFocused)
                        .onSubmit { run(items) }
                        .onKeyPress(.upArrow) {
                            selection = max(0, selection - 1)
                            return .handled
                        }
                        .onKeyPress(.downArrow) {
                            selection = min(max(0, items.count - 1), selection + 1)
                            return .handled
                        }
                        .onKeyPress(.escape) {
                            close()
                            return .handled
                        }
                }
                .padding(.horizontal, 18)
                .padding(.vertical, 15)
                if items.isEmpty {
                    // Never a blank box: a query that matches nothing says so rather
                    // than leaving the field alone above an empty panel.
                    Text("Nothing matches “\(query)”")
                        .zoomedFont(size: 13)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .truncationMode(.middle)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(.horizontal, 18)
                        .padding(.bottom, 16)
                } else {
                    VStack(spacing: 2) {
                        ForEach(Array(items.enumerated()), id: \.element.id) { index, item in
                            row(item, isSelected: index == selection)
                                // Hovering moves the selection, so the pointer and the
                                // arrow keys never disagree about what Return will run.
                                .onHover { hovering in
                                    if hovering { selection = index }
                                }
                                .onTapGesture {
                                    selection = index
                                    run(items)
                                }
                        }
                    }
                    .padding(.horizontal, 6)
                    .padding(.bottom, 6)
                }
            }
            .frame(width: Metrics.width)
            .glassEffect(.regular, in: .rect(cornerRadius: Chrome.paletteCornerRadius, style: .continuous))
            .padding(.top, Metrics.topInset)
        }
        .onAppear {
            isFocused = true
            // The scripts files as they stand now, as the Workflow menu and the ▶ menu read
            // them when they open: a file edited since shows here without a reload.
            model.refreshScripts(for: model.selectedThreadID)
        }
        .onChange(of: query) { selection = 0 }
        .onChange(of: resultsKey, initial: true) { _, _ in
            items = results
        }
        // The theme rows mark the theme in use, which a theme chord can change under them.
        .onChange(of: model.settings.theme) {
            items = results
        }
    }

    /// Everything the rows are built from, read in the body, so any change rebuilds them.
    private var resultsKey: ResultsKey {
        let runtime = model.selectedThreadID.flatMap(model.existingRuntime(for:))
        return ResultsKey(
            query: query, threads: model.threads, projects: model.projects, globalScripts: model.globalScripts,
            selectedThreadID: model.selectedThreadID, finishActionTitle: model.finishActionTitle,
            isRunning: runtime?.isRunning ?? false, draftIsEmpty: runtime?.draftIsEmpty ?? true,
            commands: providerCommands
        )
    }

    /// The slash commands and skills the selected chat's provider lists for its folder, as
    /// the chat box loaded them: the user's own custom commands among them. A name a prompt
    /// macro answers to is left out, since the macro is what that command sends.
    private var providerCommands: [SlashCommand] {
        guard let thread = model.selectedThread,
              let directory = thread.worktreePath ?? model.project(thread.projectID)?.path else { return [] }
        var seen = Set<String>()
        return (model.providers.commands[ProviderRegistry.commandKey(thread.provider, directory: directory)] ?? [])
            .filter { PromptMacro(rawValue: $0.name) == nil && seen.insert($0.name).inserted }
    }

    private func row(_ item: Item, isSelected: Bool) -> some View {
        HStack(spacing: 12) {
            Group {
                if item.icon != nil {
                    ProjectIconMark(icon: item.icon, size: 13)
                } else {
                    Image(systemName: item.symbol)
                        .zoomedFont(.body)
                        .foregroundStyle(isSelected ? AnyShapeStyle(.tint) : AnyShapeStyle(.secondary))
                }
            }
            .frame(width: ChatZoom.scaled(18, by: zoom))
            VStack(alignment: .leading, spacing: 1) {
                Text(item.title)
                    .zoomedFont(.body)
                    .lineLimit(1)
                if let subtitle = item.subtitle {
                    Text(subtitle)
                        .zoomedFont(.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .truncationMode(.middle)
                }
            }
            Spacer(minLength: 8)
            if let shortcut = item.shortcut {
                Text(shortcut)
                    .zoomedFont(.caption)
                    .foregroundStyle(.tertiary)
            }
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .background(isSelected ? AnyShapeStyle(.tint.opacity(0.16)) : AnyShapeStyle(Color.clear), in: .rect(cornerRadius: Metrics.rowCornerRadius, style: .continuous))
        .contentShape(.rect)
        // A tap gesture on its own is invisible to VoiceOver: the row reads as one
        // element that says it can be run, and says which one Return will run.
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(isSelected ? [.isButton, .isSelected] : .isButton)
    }

    private var results: [Item] {
        let needle = query.trimmingCharacters(in: .whitespaces).lowercased()
        // Chords come from the shortcut store, so the column never shows a key the user
        // has remapped away, and shows nothing at all once a chord is cleared.
        var actions = [
            Item(id: "new-thread", title: "New thread", symbol: "square.and.pencil", shortcut: ShortcutStore.label(for: .newThread)) {
                model.newThread()
            },
            Item(id: "new-worktree", title: "New thread in worktree", symbol: "arrow.triangle.branch", shortcut: ShortcutStore.label(for: .newWorktreeThread)) {
                model.newThread(workspace: .worktree)
            },
            Item(id: "add-project", title: "Add project", symbol: "folder.badge.plus", shortcut: ShortcutStore.label(for: .addProject)) {
                model.chooseProjectFolder()
            },
        ]
        var commands: [Item] = []
        if let threadID = model.selectedThreadID, let thread = model.thread(threadID) {
            let runtime = model.runtime(for: threadID)
            actions += threadActions(thread, runtime: runtime)
            commands = commandItems(runtime: runtime)
        }
        actions += windowActions
        actions.append(Item(id: "settings", title: "Settings", symbol: "gearshape", shortcut: ShortcutStore.label(for: .openSettings)) {
            WindowManager.shared.showSettings()
        })
        actions += themeActions(listingEachTheme: !needle.isEmpty)

        // An action answers to its subtitle too, so a script is found by what it runs.
        let matchingActions = actions.filter { Metrics.matches(needle, $0.title, $0.subtitle, $0.terms) }
        let threads = model.threads
            .filter { !$0.isArchived && !$0.isInPanel && Metrics.matches(needle, $0.title) }
            .sorted { $0.updatedAt > $1.updatedAt }
            .prefix(needle.isEmpty ? Metrics.recentThreads : Metrics.matchingThreads)
            .map { thread in
                Item(id: thread.id.uuidString, title: thread.title, subtitle: model.project(thread.projectID)?.name, symbol: "bubble.left") {
                    model.selectedThreadID = thread.id
                }
            }
        let projects = needle.isEmpty ? [] : model.projects
            .filter { Metrics.matches(needle, $0.name) }
            .map { project in
                Item(id: project.id.uuidString, title: "New thread in \(project.name)", subtitle: project.path, icon: project.icon, symbol: "folder") {
                    model.newThread(in: project)
                }
            }
        guard !needle.isEmpty else {
            return Array((threads + matchingActions).prefix(Metrics.maxRows))
        }
        // The provider's commands come last, and only for a query: a long list of skills must
        // never push the actions, threads and projects off the panel.
        let matchingCommands = commands.filter { Metrics.matches(needle, $0.title, $0.subtitle) }
        var ordered = matchingActions
        ordered += threads
        ordered += projects
        ordered += matchingCommands
        return Array(ordered.prefix(Metrics.maxRows))
    }

    /// The selected chat's own actions, in the order an empty query lists them: the ones for
    /// a running turn and its hands first, then its panels, its place in the list, its turns,
    /// the prompt macros and the scripts.
    private func threadActions(_ thread: ChatThread, runtime: ThreadRuntime) -> [Item] {
        var actions: [Item] = []
        if runtime.isRunning {
            actions.append(Item(id: "stop", title: "Stop the running turn", symbol: "stop.circle", shortcut: ShortcutStore.label(for: .stopTurn)) {
                runtime.interrupt()
            })
            actions.append(Item(id: "take-back", title: "Stop and take back the message", symbol: "arrow.uturn.backward.circle", shortcut: ShortcutStore.label(for: .takeBackTurn)) {
                runtime.takeBackRunningTurn()
            })
        }
        // The chat's hands still at work, as the Thread menu's Stop all hands: the lead's own
        // turn goes on. The hands' records are among the threads the rows are built from.
        if model.runningCrewHands(of: thread.id) > 0 {
            actions.append(Item(id: "stop-hands", title: "Stop all hands", symbol: "hand.raised", shortcut: ShortcutStore.label(for: .stopAllHands)) {
                model.stopAllCrewHands(of: thread.id)
            })
        }
        if !runtime.draftIsEmpty {
            actions.append(Item(id: "queue", title: "Queue a chat", symbol: "text.badge.plus", shortcut: ShortcutStore.label(for: .queueChat)) {
                runtime.queueDraftAsFollowUp()
            })
        }
        actions.append(Item(id: "terminal", title: "Toggle terminal", symbol: "terminal", shortcut: ShortcutStore.label(for: .toggleTerminal)) {
            runtime.isTerminalVisible.toggle()
        })
        actions.append(Item(id: "changes", title: "Toggle changes", symbol: "plusminus", shortcut: ShortcutStore.label(for: .toggleChanges)) {
            runtime.toggleDiff()
        })
        actions.append(Item(id: "finish", title: model.finishActionTitle, symbol: model.finishActionSymbol, shortcut: ShortcutStore.label(for: .finishThread)) {
            model.finishSelectedThread()
        })
        // Settle has a row of its own unless Finish already is that very command; a helper
        // has no place to settle into. Archive asks first, with Delete beside it, as the
        // Thread menu's Archive thread… does, so it keeps its row whatever Finish does.
        if !thread.isHelper, model.settings.threadFinishAction != .settle {
            actions.append(Item(id: "settle", title: thread.isSettled ? "Reopen thread" : "Settle thread", symbol: thread.isSettled ? "arrow.uturn.backward" : "checkmark.circle", shortcut: ShortcutStore.label(for: .settleThread)) {
                model.settleSelectedThread()
            })
        }
        actions.append(Item(id: "archive", title: "Archive thread…", symbol: "archivebox", shortcut: ShortcutStore.label(for: .archiveThread)) {
            model.askToArchiveSelectedThread()
        })
        actions.append(Item(id: "plan", title: "Toggle plan mode", subtitle: thread.interactionMode == .plan ? "Plan mode is on" : nil, symbol: "list.bullet.clipboard", shortcut: ShortcutStore.label(for: .togglePlanMode)) {
            model.updateThread(thread.id) { $0.interactionMode = $0.interactionMode == .plan ? .build : .plan }
        })
        actions.append(Item(id: "crew", title: "Toggle Crew", symbol: "person.2", shortcut: ShortcutStore.label(for: .toggleCrewPanel)) {
            runtime.showsCrewDeck.toggle()
        })
        actions.append(Item(id: "fold-turns", title: "Fold all turns", symbol: "rectangle.compress.vertical", shortcut: ShortcutStore.label(for: .foldAllTurns)) {
            runtime.foldAllTurns()
        })
        // The prompt macros go to the lead as they are, the way their commands do: at once
        // while the chat is idle, else queued behind the running turn.
        for macro in PromptMacro.allCases {
            actions.append(Item(id: "macro-\(macro.rawValue)", title: macro.title, subtitle: macro.detail, symbol: macro.symbol, shortcut: macro.command, terms: macro.command) {
                runtime.sendMacro(macro)
            })
        }
        // Every script the Workflow menu lists, the global ones first, found by its name or by
        // what it runs, with the ⌥⌘ number it has there (see `ScriptShortcuts`); the project's
        // first keeps the Run top script chord when it has no number.
        let topScriptID = model.project(thread.projectID)?.scripts.first?.id
        for (index, script) in model.scripts(for: thread.id).enumerated() {
            let shortcut = ScriptShortcuts.chord(forNumber: index + 1)?.description
                ?? (script.id == topScriptID ? ShortcutStore.label(for: .runScript) : nil)
            actions.append(Item(id: "script-\(script.id.uuidString)", title: "Run \(script.name)", subtitle: script.command, symbol: script.symbol, shortcut: shortcut) {
                model.runScript(script, threadID: thread.id)
            })
        }
        // The Workflow menu's way to the chat's project's scripts, as the menu opens it.
        actions.append(Item(id: "edit-project-scripts", title: "Edit project scripts…", symbol: "slider.horizontal.3", terms: "workflow") {
            model.requestScriptsEditor(.project(thread.projectID))
        })
        return actions
    }

    /// The actions that need no chat: moving through the list, the window's panels, search
    /// and the thinking switch. Settings follows them.
    private var windowActions: [Item] {
        [
            Item(id: "previous-thread", title: "Previous thread", symbol: "chevron.up", shortcut: ShortcutStore.label(for: .previousThread)) {
                model.selectThread(offset: -1)
            },
            Item(id: "next-thread", title: "Next thread", symbol: "chevron.down", shortcut: ShortcutStore.label(for: .nextThread)) {
                model.selectThread(offset: 1)
            },
            Item(id: "sidebar", title: "Toggle sidebar", symbol: "sidebar.left", shortcut: ShortcutStore.label(for: .toggleSidebar)) {
                model.sidebar.toggle()
            },
            Item(id: "activity", title: "Toggle activity view", symbol: "clock", shortcut: ShortcutStore.label(for: .toggleActivityView)) {
                withAnimation(Chrome.panelSlide) { model.settings.sidebarActivityView.toggle() }
            },
            Item(id: "search", title: "Search threads", subtitle: "In the sidebar", symbol: "magnifyingglass", shortcut: ShortcutStore.label(for: .focusSearch)) {
                model.focusSidebarSearch()
            },
            Item(id: "thinking", title: "Toggle thinking", symbol: "brain", shortcut: ShortcutStore.label(for: .toggleReasoning)) {
                model.settings.showReasoning.toggle()
            },
            // The Workflow menu's two scripts actions that need no chat.
            Item(id: "edit-global-scripts", title: "Edit global scripts…", symbol: "globe", terms: "workflow") {
                model.requestScriptsEditor(.global)
            },
            Item(id: "reload-scripts", title: "Reload scripts", symbol: "arrow.clockwise", terms: "workflow") {
                model.reloadScripts()
            },
        ]
    }

    /// The provider's commands and skills for the selected chat, the user's custom ones among
    /// them. Picking one puts it in the chat box as its chip, as the `/` list does, to go out
    /// with whatever is typed after it; the caret follows it there.
    private func commandItems(runtime: ThreadRuntime) -> [Item] {
        providerCommands.map { command in
            Item(id: "command-\(command.name)", title: "/\(command.name)", subtitle: command.detail.isEmpty ? nil : command.detail, symbol: command.isBuiltIn ? "command" : "sparkles") {
                withAnimation(Chrome.panelSlide) {
                    runtime.draft.command = DraftCommand(name: command.name, detail: command.detail, isBuiltIn: command.isBuiltIn)
                }
                runtime.requestComposerFocus()
            }
        }
    }

    /// The two theme switches, and a row per theme once something is typed, so "dark" or
    /// "theme" finds them without the unfiltered list growing by five rows.
    private func themeActions(listingEachTheme: Bool) -> [Item] {
        let settings = model.settings
        var rows = [
            Item(id: "theme-next", title: "Next theme", symbol: "paintpalette", shortcut: ShortcutStore.label(for: .cycleTheme)) {
                settings.cycleTheme()
            },
            Item(id: "theme-toggle", title: "Toggle light and dark", symbol: "circle.lefthalf.filled", shortcut: ShortcutStore.label(for: .toggleAppearance)) {
                settings.toggleAppearance()
            },
        ]
        guard listingEachTheme else { return rows }
        let current = settings.theme
        rows += AppTheme.available(current: current).map { theme in
            Item(id: "theme-\(theme.rawValue)", title: "Theme: \(theme.displayName)", subtitle: theme.detail, symbol: theme == current ? "checkmark" : "paintbrush") {
                settings.theme = theme
            }
        }
        return rows
    }

    private func run(_ items: [Item]) {
        guard items.indices.contains(selection) else { return }
        let item = items[selection]
        close()
        item.perform()
    }

    private func close() {
        model.isCommandPalettePresented = false
    }
}
