// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import SwiftUI

@main
struct BottegaApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var delegate

    init() {
        // Before anything posts a notification: a framework observer that raises while one is posted
        // (the popover window ordering on macOS 27) then costs a log line, not the app.
        DCInstallNotificationGuard()
        // Provider processes can exit while we write to them; report EPIPE instead of crashing.
        signal(SIGPIPE, SIG_IGN)
        // A frozen main thread gets its stacks written to ~/Library/Logs/<AppInfo.storageName>.
        HangWatchdog.start()
        SwitchLatency.start()
    }

    var body: some Scene {
        // The windows themselves are AppKit windows owned by WindowManager. This scene only carries the menus.
        Settings { EmptyView() }
            .commands { AppCommands(model: WindowManager.shared.model) }
    }
}
