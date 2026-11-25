// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import AppKit

/// Whether what is being handled right now came from the keyboard or mouse. Another program can post events too
/// (`osascript`, accessibility tools); the kernel notes which process did, and none did for the real ones. An action
/// pressed through the accessibility interface comes with no such event at all.
///
/// Anything that would be signed as the user asks this first (vault Bottega-3, section 2.5).
@MainActor
enum HumanInput {
    static var isGenuine: Bool {
        guard NSApp.isActive, let event = NSApp.currentEvent else { return false }
        switch event.type {
        case .leftMouseDown, .leftMouseUp, .keyDown:
            return isGenuine(sourceProcess: event.cgEvent?.getIntegerValueField(.eventSourceUnixProcessID))
        default:
            return false
        }
    }

    /// No source at all means AppKit made the event up.
    static func isGenuine(sourceProcess: Int64?) -> Bool { sourceProcess == 0 }
}
