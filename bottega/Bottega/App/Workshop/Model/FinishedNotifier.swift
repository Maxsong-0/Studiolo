// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import AppKit
import UserNotifications

/// "Work finished", with how many and nothing else: no line, card or word of anyone's (vault Bottega-6, D10;
/// Bottega-4, section 3). Only while the app is not in front. What comes in a row is one notification whose count
/// grows, until the app is in front again. What counts is `WorkshopState.finished(since:)`.
@MainActor
final class FinishedNotifier {
    /// Settings › Notify when work finishes. On unless turned off.
    static let isOnKey = "notifiesWhenFinished"
    /// One for all of them: a new one takes the place of the one before.
    private static let identifier = "work-finished"

    private var count = 0
    private var hasAsked = false

    static var isOn: Bool { UserDefaults.standard.object(forKey: isOnKey) as? Bool ?? true }

    /// Whether what changes now could be told.
    var isListening: Bool { Self.isOn && !NSApp.isActive }

    /// Once a launch, and only while on. The system asks the user the first time only; turned down, nothing is shown
    /// and nothing asks again.
    func askPermission() {
        guard Self.isOn, !hasAsked else { return }
        hasAsked = true
        Task { _ = try? await UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .sound]) }
    }

    func tell(_ finished: Int) {
        guard finished > 0, isListening else { return }
        count += finished
        let content = UNMutableNotificationContent()
        content.title = "Work finished"
        content.body = Wording.finished(count)
        content.sound = .default
        // Clicking it brings the app forward, which is all it is for.
        let request = UNNotificationRequest(identifier: Self.identifier, content: content, trigger: nil)
        Task { try? await UNUserNotificationCenter.current().add(request) }
    }

    /// The app is in front: what was told is seen.
    func cameToFront() {
        guard count > 0 else { return }
        count = 0
        UNUserNotificationCenter.current().removeDeliveredNotifications(withIdentifiers: [Self.identifier])
    }
}
