// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import AppKit
import SwiftUI

struct AppCommands: Commands {
    let model: AppModel
    private let shortcuts = ShortcutStore.shared

    private var runtime: ThreadRuntime? {
        model.selectedThreadID.map(model.runtime(for:))
    }

    var body: some Commands {
        CommandGroup(replacing: .appSettings) {
            Button("Settings…") { WindowManager.shared.showSettings() }
                .keyboardShortcut(shortcuts.keyboardShortcut(for: .openSettings))
        }

        // File's stock Close sits before the Thread menu and would take ⌘W from Archive thread…,
        // closing the main window instead. Secondary windows close on ⌘W themselves (SecondaryWindow).
        CommandGroup(replacing: .saveItem) {}

        CommandGroup(before: .windowArrangement) {
            Button(AppInfo.name) { WindowManager.shared.showMain() }
                .keyboardShortcut(shortcuts.keyboardShortcut(for: .showMainWindow))
            Divider()
        }

        CommandGroup(replacing: .newItem) {
            Button("New thread") { model.newThread() }
                .keyboardShortcut(shortcuts.keyboardShortcut(for: .newThread))
            Button("New thread in worktree") { model.newThread(workspace: .worktree) }
                .keyboardShortcut(shortcuts.keyboardShortcut(for: .newWorktreeThread))
            Divider()
            Button("Add project…") { model.chooseProjectFolder() }
                .keyboardShortcut(shortcuts.keyboardShortcut(for: .addProject))
        }

        CommandGroup(after: .sidebar) {
            Button("Toggle sidebar") { model.sidebar.toggle() }
                .keyboardShortcut(shortcuts.keyboardShortcut(for: .toggleSidebar))
            // The palette lives over the main window alone. From Settings, or any window but
            // the main one, the chord brings the main window forward with the palette open;
            // flipping the flag there showed nothing, and left the palette open for later.
            Button("Command palette") {
                if WindowManager.shared.isMainWindowKey {
                    model.isCommandPalettePresented.toggle()
                } else {
                    WindowManager.shared.showMain()
                    model.isCommandPalettePresented = true
                }
            }
            .keyboardShortcut(shortcuts.keyboardShortcut(for: .commandPalette))
            Button("Toggle terminal") { runtime?.isTerminalVisible.toggle() }
                .keyboardShortcut(shortcuts.keyboardShortcut(for: .toggleTerminal))
                .disabled(model.selectedThreadID == nil)
            Button("Toggle changes") { runtime?.toggleDiff() }
                .keyboardShortcut(shortcuts.keyboardShortcut(for: .toggleChanges))
                .disabled(model.selectedThreadID == nil)
            Button("Toggle Crew") { runtime?.showsCrewDeck.toggle() }
                .keyboardShortcut(shortcuts.keyboardShortcut(for: .toggleCrewPanel))
                .disabled(model.selectedThreadID == nil)
            Button("Toggle activity view") {
                withAnimation(Chrome.panelSlide) { model.settings.sidebarActivityView.toggle() }
            }
            .keyboardShortcut(shortcuts.keyboardShortcut(for: .toggleActivityView))
            Button("Search threads") { model.focusSidebarSearch() }
                .keyboardShortcut(shortcuts.keyboardShortcut(for: .focusSearch))
            Button("Toggle thinking") { model.settings.showReasoning.toggle() }
                .keyboardShortcut(shortcuts.keyboardShortcut(for: .toggleReasoning))
            Button("Fold all turns") { runtime?.foldAllTurns() }
                .keyboardShortcut(shortcuts.keyboardShortcut(for: .foldAllTurns))
                .disabled(model.selectedThreadID == nil)
            Divider()
            ThemeMenuItems(settings: model.settings, shortcuts: shortcuts)
            Divider()
        }

        CommandMenu("Thread") {
            Button("Stop") { runtime?.interrupt() }
                .keyboardShortcut(shortcuts.keyboardShortcut(for: .stopTurn))
                .disabled(!(runtime?.isRunning ?? false))
            // The selected chat's hands still at work, native and hosted; the lead's own
            // turn goes on. Read off the hands' own records, which change as a hand starts
            // or finishes, never with its stream.
            Button("Stop all hands") {
                guard let id = model.selectedThreadID else { return }
                model.stopAllCrewHands(of: id)
            }
            .keyboardShortcut(shortcuts.keyboardShortcut(for: .stopAllHands))
            .disabled(model.selectedThreadID.map { model.runningCrewHands(of: $0) == 0 } ?? true)
            // On with nothing to queue behind: the draft then just goes out (see
            // `queueDraftAsFollowUp`), rather than the chord doing nothing at all. A chat box
            // with the keyboard takes the chord before this item (`ComposerNSTextView`), so a
            // hand's box queues its own draft; this serves the chat on screen otherwise. It no
            // longer steps aside for an approval: the chord is off ⌘Return, which the chat box
            // keeps for sending and, with nothing typed, approving.
            Button("Queue a chat") { runtime?.queueDraftAsFollowUp() }
                .keyboardShortcut(shortcuts.keyboardShortcut(for: .queueChat))
                .disabled(runtime?.draftIsEmpty ?? true)
            // Escape no longer does this, so a stray one never costs the turn: the turn stops
            // and its message comes back to the box, the thread rewound to before it.
            Button("Stop and take back") { runtime?.takeBackRunningTurn() }
                .keyboardShortcut(shortcuts.keyboardShortcut(for: .takeBackTurn))
                .disabled(!(runtime?.isRunning ?? false))
            Button("Toggle plan mode") {
                guard let id = model.selectedThreadID else { return }
                model.updateThread(id) { $0.interactionMode = $0.interactionMode == .plan ? .build : .plan }
            }
            .keyboardShortcut(shortcuts.keyboardShortcut(for: .togglePlanMode))
            .disabled(model.selectedThreadID == nil)
            Divider()
            Button("Previous thread") { model.selectThread(offset: -1) }
                .keyboardShortcut(shortcuts.keyboardShortcut(for: .previousThread))
            Button("Next thread") { model.selectThread(offset: 1) }
                .keyboardShortcut(shortcuts.keyboardShortcut(for: .nextThread))
            Button(model.topScript.map { "Run \($0.name)" } ?? "Run top script") {
                guard let script = model.topScript, let threadID = model.selectedThreadID else { return }
                model.runScript(script, threadID: threadID)
            }
            .keyboardShortcut(shortcuts.keyboardShortcut(for: .runScript))
            .disabled(model.topScript == nil)
            ForEach(1...9, id: \.self) { number in
                Button("Thread \(number)") { model.selectThread(number: number) }
                    .keyboardShortcut(KeyEquivalent(Character(String(number))), modifiers: .command)
            }
            Divider()
            Button(model.finishActionTitle) { model.finishSelectedThread() }
                .keyboardShortcut(shortcuts.keyboardShortcut(for: .finishThread))
                .disabled(model.selectedThreadID == nil)
            Button("Archive thread…") { model.askToArchiveSelectedThread() }
                .keyboardShortcut(shortcuts.keyboardShortcut(for: .archiveThread))
                .disabled(model.selectedThreadID == nil)
            Button(model.selectedThread?.isSettled == true ? "Reopen thread" : "Settle thread") { model.settleSelectedThread() }
                .keyboardShortcut(shortcuts.keyboardShortcut(for: .settleThread))
                .disabled(model.selectedThread.map { $0.isHelper } ?? true)
        }

        // The scripts, one keystroke each: the global ones first, so each keeps its ⌥⌘ number
        // in every project, then the selected thread's project's (see `AppModel.scripts(for:)`).
        // Built from the lists as they stand; a file edited since is read again as the app
        // comes to the front, as the chat's ▶ menu opens and before a script runs.
        CommandMenu("Workflow") {
            let threadID = model.selectedThreadID
            let global = model.globalScripts
            let project = model.selectedThread.flatMap { model.project($0.projectID) }
            let projectScripts = project?.scripts ?? []
            if !global.isEmpty {
                Section("Global scripts") {
                    ForEach(Array(global.enumerated()), id: \.element.id) { index, script in
                        scriptButton(script, number: index + 1, threadID: threadID)
                    }
                }
            }
            if let project, !projectScripts.isEmpty {
                Section(project.name) {
                    ForEach(Array(projectScripts.enumerated()), id: \.element.id) { index, script in
                        scriptButton(script, number: global.count + index + 1, threadID: threadID)
                    }
                }
            }
            if global.isEmpty, projectScripts.isEmpty {
                Button("No scripts yet") {}
                    .disabled(true)
            }
            // The prompt macros, as the palette and their commands send them: to the lead, at
            // once while the chat is idle, else queued behind the running turn.
            Section("Prompts") {
                ForEach(PromptMacro.allCases) { macro in
                    Button {
                        runtime?.sendMacro(macro)
                    } label: {
                        Label(macro.title, systemImage: macro.symbol)
                    }
                    .disabled(model.selectedThreadID == nil)
                }
            }
            Divider()
            // The editor opens over the main window's chat, so the window comes forward first.
            Button("Edit project scripts…") {
                guard let project else { return }
                WindowManager.shared.showMain()
                model.requestScriptsEditor(.project(project.id))
            }
            .disabled(project == nil)
            Button("Edit global scripts…") {
                WindowManager.shared.showMain()
                model.requestScriptsEditor(.global)
            }
            Button("Reload scripts") { model.reloadScripts() }
        }

        // The welcome tour went with the rest of the onboarding. The group is still
        // replaced, so macOS does not put its own Help menu back in its place.
        CommandGroup(replacing: .help) {}
    }

    /// A script's row in the Workflow menu, with its ⌥⌘ number while it is among the first
    /// nine (see `ScriptShortcuts`). Off with no thread selected: a script runs in a thread's
    /// terminal. Pressed as its chord, the row runs the script at its number as the files
    /// have it now: a file changed with the app in front has not rebuilt this menu yet, and
    /// ⌥⌘N means the N-th script. Picked from the menu, it runs the script the row showed.
    private func scriptButton(_ script: ProjectScript, number: Int, threadID: UUID?) -> some View {
        Button {
            guard let threadID else { return }
            if NSApp.currentEvent?.type == .keyDown {
                model.runScript(number: number, threadID: threadID)
            } else {
                model.runScript(script, threadID: threadID)
            }
        } label: {
            Label(script.name, systemImage: script.symbol)
        }
        .keyboardShortcut(ScriptShortcuts.chord(forNumber: number)?.keyboardShortcut)
        .disabled(threadID == nil)
    }
}

/// The View menu's theme items: every theme as a checked choice, and the two switches with
/// the chords recorded for them. A view of its own, so the menu reads the theme (and the
/// custom theme's name) as it changes.
private struct ThemeMenuItems: View {
    @Bindable var settings: AppSettings
    let shortcuts: ShortcutStore

    var body: some View {
        Picker("Theme", selection: $settings.theme) {
            ForEach(AppTheme.available(current: settings.theme)) { theme in
                Text(verbatim: theme.displayName).tag(theme)
            }
        }
        Button("Next theme") { settings.cycleTheme() }
            .keyboardShortcut(shortcuts.keyboardShortcut(for: .cycleTheme))
        Button("Toggle light and dark") { settings.toggleAppearance() }
            .keyboardShortcut(shortcuts.keyboardShortcut(for: .toggleAppearance))
    }
}
