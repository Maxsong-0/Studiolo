// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import AppKit
import BottegaKit
import os

/// The app, keeping track of the event it is handling. AppKit's own `currentEvent` outlives the handling of it.
final class WorkshopApplication: NSApplication {
    /// The event being handled right now, as long as handling it has not started waiting for others (a menu, a
    /// control following the mouse, a modal panel): from then on, what runs need not be its doing.
    fileprivate private(set) var handling: NSEvent?
    private var depth = 0

    override func sendEvent(_ event: NSEvent) {
        depth += 1
        handling = depth == 1 ? event : nil
        defer {
            depth -= 1
            handling = nil
        }
        super.sendEvent(event)
    }

    override func nextEvent(
        matching mask: NSEvent.EventTypeMask, until expiration: Date?, inMode mode: RunLoop.Mode, dequeue: Bool
    ) -> NSEvent? {
        handling = nil
        return super.nextEvent(matching: mask, until: expiration, inMode: mode, dequeue: dequeue)
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

    /// Uses the press up: one press, one thing signed. Throws why it does not count.
    static func take() throws(InputGate.Refusal) {
        do throws(InputGate.Refusal) {
            try gate.admit(current, isInFront: NSApp.isActive, now: ProcessInfo.processInfo.systemUptime)
        } catch {
            logger.notice("refused: \(error.rawValue, privacy: .public)")
            throw error
        }
    }

    private static var current: InputGate.Input? {
        guard let app = NSApp as? WorkshopApplication, let event = app.handling else { return nil }
        let isPress = switch event.type {
        // A button acts when the mouse is let go.
        case .leftMouseUp: true
        // A key held down repeats: that must not open an ask and approve it in one go.
        case .keyDown: !event.isARepeat
        default: false
        }
        return InputGate.Input(
            isPress: isPress, sourceProcess: event.cgEvent?.getIntegerValueField(.eventSourceUnixProcessID),
            timestamp: event.timestamp, isInKeyWindow: event.window != nil && event.window === app.keyWindow)
    }
}
