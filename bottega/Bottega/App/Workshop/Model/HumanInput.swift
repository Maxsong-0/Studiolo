// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import AppKit
import BottegaKit
import os

/// The app, keeping track of the event it is handling. AppKit's own `currentEvent` outlives the handling of it.
final class WorkshopApplication: NSApplication {
    /// Events being sent right now; more than one deep while a control tracks the mouse.
    fileprivate private(set) var sending = 0

    override func sendEvent(_ event: NSEvent) {
        sending += 1
        defer { sending -= 1 }
        super.sendEvent(event)
    }
}

/// Whether what is being handled right now came from the keyboard or mouse. Another program can post events too
/// (`osascript`, accessibility tools); the kernel notes which process did, and none did for the real ones. An action
/// pressed through the accessibility interface comes with no event at all.
///
/// Anything that would be signed as the user asks this first (vault Bottega-3, section 2.5).
@MainActor
enum HumanInput {
    private static var gate = InputGate()
    private static let logger = Logger(subsystem: AppInfo.logSubsystem, category: "input")

    /// Uses the press up: one press, one thing signed.
    static func take() -> Bool {
        do throws(InputGate.Refusal) {
            try gate.admit(current, isInFront: NSApp.isActive, now: ProcessInfo.processInfo.systemUptime)
            return true
        } catch {
            logger.notice("refused: \(error.rawValue, privacy: .public)")
            return false
        }
    }

    private static var current: InputGate.Input? {
        guard let app = NSApp as? WorkshopApplication, app.sending > 0, let event = app.currentEvent else { return nil }
        let isPress = switch event.type {
        case .leftMouseDown, .leftMouseUp: true
        // A key held down repeats: that must not open an ask and approve it in one go.
        case .keyDown: !event.isARepeat
        default: false
        }
        return InputGate.Input(
            isPress: isPress, sourceProcess: event.cgEvent?.getIntegerValueField(.eventSourceUnixProcessID),
            timestamp: event.timestamp, isInKeyWindow: event.window != nil && event.window === app.keyWindow)
    }
}
