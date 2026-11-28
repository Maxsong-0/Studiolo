// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import AppKit
import SwiftUI

struct WorkshopApp: App {
    @NSApplicationDelegateAdaptor(WorkshopDelegate.self) private var delegate
    @State private var model = WorkshopModel.forThisLaunch()
    // As the Mac is set when the app opens, so that the first frame is already right; the window keeps it in step.
    @State private var look = Look(
        increasedContrast: NSWorkspace.shared.accessibilityDisplayShouldIncreaseContrast,
        reducedTransparency: NSWorkspace.shared.accessibilityDisplayShouldReduceTransparency)

    init() {
        // Writing to a socket LMAS has closed is an error to handle, not the end of the app.
        signal(SIGPIPE, SIG_IGN)
        HangWatchdog.start()
    }

    var body: some Scene {
        Window(AppInfo.name, id: "workshop") {
            WorkshopView()
                .environment(model)
                .environment(look)
                .task { await model.run() }
        }
        // The photo runs to the window's edges. The toolbar is empty and clear: it is only there so the window's
        // buttons sit lower, inside the sidebar, as in the design.
        .windowToolbarStyle(.unified(showsTitle: false))
        .windowBackgroundDragBehavior(.enabled)
        .defaultSize(width: 1280, height: 800)
        .commands { WorkshopCommands(model: model) }

        Settings {
            WorkshopSettings().environment(model).environment(look)
        }
    }
}

final class WorkshopDelegate: NSObject, NSApplicationDelegate {
    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { false }
}

struct WorkshopCommands: Commands {
    let model: WorkshopModel

    var body: some Commands {
        CommandGroup(replacing: .newItem) {
            #if DEBUG
            Button("Open Recording…") { openRecording() }
            #endif
        }
        // In the View menu, where every Mac app keeps these two.
        CommandGroup(replacing: .sidebar) {
            Button(model.showsSidebar ? "Hide Sidebar" : "Show Sidebar") { model.showsSidebar.toggle() }
                .keyboardShortcut("s", modifiers: [.control, .command])
            Button(model.showsInspector ? "Hide Panel" : "Show Panel") { model.showsInspector.toggle() }
                .keyboardShortcut("0", modifiers: [.option, .command])
        }
        CommandMenu("Go") {
            Button("Search…") { model.showsPalette.toggle() }
                .keyboardShortcut("k", modifiers: .command)
            Divider()
            ForEach(SidebarSection.allCases) { section in
                Button(section.title) { model.show(section) }
                    .keyboardShortcut(KeyEquivalent(Character("\(section.rawValue + 1)")), modifiers: .command)
            }
            Divider()
            Button("Dispatcher") { model.page = .dispatcher }
                .keyboardShortcut("0", modifiers: .command)
            Button("Devices") { model.page = .devices }
                .keyboardShortcut("6", modifiers: .command)
            Button("Projects") { model.page = .projects }
                .keyboardShortcut("7", modifiers: .command)
            Divider()
            Button("Stop This Turn") { if let turn = model.turnToStop { model.stop(turn) } }
                .keyboardShortcut(".", modifiers: .command)
                .disabled(model.turnToStop == nil || !model.isLive)
        }
        CommandMenu("LMAS") {
            Button("Reconnect") { model.reconnect() }
                .disabled(!model.hasLink)
            Button("Reload All Events") { model.reload() }
                .disabled(!model.hasLink)
        }
    }

    #if DEBUG
    /// Shows recorded events, with nothing to send to. For looking at the interface without an LMAS.
    private func openRecording() {
        let panel = NSOpenPanel()
        panel.allowedContentTypes = [.init(filenameExtension: "jsonl") ?? .data, .json, .plainText]
        guard panel.runModal() == .OK, let url = panel.url else { return }
        model.play(url)
    }
    #endif
}
