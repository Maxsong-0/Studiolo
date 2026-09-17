import SwiftUI

@main
struct DroppyCodeApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var delegate

    init() {
        // Provider processes can exit while we write to them; report EPIPE instead of crashing.
        signal(SIGPIPE, SIG_IGN)
        // Before anything reads settings or the library.
        WebsiteCaptures.prepare()
        LegacyMigration.run()
        // A frozen main thread gets its stacks written to ~/Library/Logs/Droppy Code.
        HangWatchdog.start()
    }

    var body: some Scene {
        // The windows themselves are AppKit windows owned by WindowManager. This scene only carries the menus.
        Settings { EmptyView() }
            .commands { AppCommands(model: WindowManager.shared.model) }
    }
}
