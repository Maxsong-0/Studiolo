import AppKit
import UserNotifications

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate, UNUserNotificationCenterDelegate {
    var model: AppModel?

    func applicationDidFinishLaunching(_ notification: Notification) {
        UNUserNotificationCenter.current().delegate = self
        let windows = WindowManager.shared
        model = windows.model
        if WebsiteCaptures.isEnabled {
            Task { await WebsiteCaptures.run(model: windows.model) }
            return
        }
        windows.showMain()
        windows.model.openThreadOnLaunch()
        if !windows.model.settings.hasSeenTour {
            Task { @MainActor in try? await Task.sleep(for: .milliseconds(700)); Tour.present(model: windows.model) }
        }
        Task { await windows.model.bootstrap() }
        UpdateChecker.shared.startBackgroundChecks()
        // The relaunch after an install: the update story finishes on About, where it began.
        if AppUpdater.shared.consumeRelaunchMarker() {
            UpdateInstallProgress.shared.armCelebration()
            windows.showSettings(page: .about)
        }
    }

    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        if !flag { WindowManager.shared.showMain() }
        return true
    }

    func applicationWillTerminate(_ notification: Notification) {
        model?.saveBeforeQuit()
        TokenLedger.shared.flushLiveSpend()
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
        let isUpdate = userInfo["update"] != nil
        await MainActor.run {
            if !isUpdate { WindowManager.shared.showMain() }
            if let identifier, let threadID = UUID(uuidString: identifier) {
                self.model?.selectedThreadID = threadID
            }
            NSApp.activate()
            if isUpdate { WindowManager.shared.showSettings(page: .about) }
        }
    }
}
