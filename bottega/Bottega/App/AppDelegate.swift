// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import AppKit
import UserNotifications

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate, UNUserNotificationCenterDelegate {
    var model: AppModel?

    func applicationDidFinishLaunching(_ notification: Notification) {
        UNUserNotificationCenter.current().delegate = self
        let windows = WindowManager.shared
        model = windows.model
        windows.showMain()
        windows.model.openThreadOnLaunch()
        Task { await windows.model.bootstrap() }
    }

    /// Back from another app, where a scripts file may just have been edited: the selected
    /// thread's scripts and the global ones are read again where their files changed, so the
    /// Workflow menu and the chat's ▶ menu show them as they are now.
    func applicationDidBecomeActive(_ notification: Notification) {
        guard let model else { return }
        model.refreshScripts(for: model.selectedThreadID)
    }

    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        if !flag { WindowManager.shared.showMain() }
        return true
    }

    /// A quit while the team's work is going out waits for it: the merge is a minute or
    /// two of git and one model call, and losing it half-way leaves the work unmerged
    /// until the user's next message. Capped, so a stuck merge never holds the quit. An
    /// archive page's unsaved draft is saved first, or kept as a copy (see
    /// `AppModel.saveArchiveDraftsBeforeQuit`): a moment of git, never a question at quit.
    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        guard let model, model.isAnyCrewMergeRunning || model.hasUnsavedArchiveDrafts else { return .terminateNow }
        Task { @MainActor in
            await model.saveArchiveDraftsBeforeQuit()
            let deadline = Date.now.addingTimeInterval(4 * 60)
            while model.isAnyCrewMergeRunning, Date.now < deadline {
                try? await Task.sleep(for: .milliseconds(250))
            }
            sender.reply(toApplicationShouldTerminate: true)
        }
        return .terminateLater
    }

    func applicationWillTerminate(_ notification: Notification) {
        model?.saveBeforeQuit()
        TokenLedger.shared.flushLiveSpend()
        // The MCP servers the hub started go with the app.
        MCPHub.terminateAll()
    }

    nonisolated func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        willPresent notification: UNNotification
    ) async -> UNNotificationPresentationOptions {
        let threadIDString = notification.request.content.userInfo["threadID"] as? String
        if let threadIDString, let notifiedID = UUID(uuidString: threadIDString) {
            let isOpenChat = await MainActor.run {
                NSApp.isActive && self.model?.selectedThreadID == notifiedID
            }
            if isOpenChat { return [] }
        }
        return [.banner, .sound]
    }

    nonisolated func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        didReceive response: UNNotificationResponse
    ) async {
        let userInfo = response.notification.request.content.userInfo
        let identifier = userInfo["threadID"] as? String
        await MainActor.run {
            WindowManager.shared.showMain()
            if let identifier, let threadID = UUID(uuidString: identifier) {
                self.model?.selectedThreadID = threadID
            }
            NSApp.activate()
        }
    }
}
