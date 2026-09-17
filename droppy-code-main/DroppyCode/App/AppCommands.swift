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
            Button("Command palette") { model.isCommandPalettePresented.toggle() }
                .keyboardShortcut(shortcuts.keyboardShortcut(for: .commandPalette))
            Button("Toggle terminal") { runtime?.isTerminalVisible.toggle() }
                .keyboardShortcut(shortcuts.keyboardShortcut(for: .toggleTerminal))
                .disabled(model.selectedThreadID == nil)
            Button("Toggle changes") { runtime?.toggleDiff() }
                .keyboardShortcut(shortcuts.keyboardShortcut(for: .toggleChanges))
                .disabled(model.selectedThreadID == nil)
            Button("Toggle activity view") {
                withAnimation(Chrome.panelSlide) { model.settings.sidebarActivityView.toggle() }
            }
            .keyboardShortcut(shortcuts.keyboardShortcut(for: .toggleActivityView))
            Divider()
        }

        CommandMenu("Thread") {
            Button("Stop") { runtime?.interrupt() }
                .keyboardShortcut(shortcuts.keyboardShortcut(for: .stopTurn))
                .disabled(!(runtime?.isRunning ?? false))
            // Off while the thread is waiting on an approval: the approve button ships with
            // the same ⌘Return, and a menu command always wins over a button's shortcut, so
            // the queue would swallow the chord the card is asking for. On with nothing to
            // queue behind: the draft then just goes out (see `queueDraftAsFollowUp`),
            // rather than the chord doing nothing at all.
            Button("Queue a chat") { runtime?.queueDraftAsFollowUp() }
                .keyboardShortcut(shortcuts.keyboardShortcut(for: .queueChat))
                .disabled((runtime?.draftIsEmpty ?? true)
                    || !(runtime?.approvals.isEmpty ?? true))
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

        CommandGroup(replacing: .help) {
            Button("Welcome tour") { Tour.present(model: model) }
        }
    }
}
