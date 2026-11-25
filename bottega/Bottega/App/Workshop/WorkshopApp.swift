// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import AppKit
import SwiftUI

struct WorkshopApp: App {
    @NSApplicationDelegateAdaptor(WorkshopDelegate.self) private var delegate
    @State private var model = WorkshopModel.forThisLaunch()

    init() {
        // Writing to a socket LMAS has closed is an error to handle, not the end of the app.
        signal(SIGPIPE, SIG_IGN)
        HangWatchdog.start()
    }

    var body: some Scene {
        Window(AppInfo.name, id: "workshop") {
            WorkshopView()
                .environment(model)
                .frame(minWidth: 860, minHeight: 520)
                .task { await model.run() }
        }
        .defaultSize(width: 1280, height: 820)
        .commands { WorkshopCommands(model: model) }

        Settings {
            WorkshopSettings().environment(model)
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
        CommandMenu("Go") {
            ForEach(Array(Page.fixed.enumerated()), id: \.element) { index, page in
                Button(page.fixedTitle) { model.page = page }
                    .keyboardShortcut(KeyEquivalent(Character("\(index + 1)")), modifiers: .command)
            }
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
