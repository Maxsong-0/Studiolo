import AppKit

final class ThreadWindow: NSWindow {
    var selectThread: ((Int) -> Void)?

    override func performKeyEquivalent(with event: NSEvent) -> Bool {
        if event.keyCode == 48, attachedSheet == nil {
            let chord = KeyChord(keyCode: Int(event.keyCode), modifiers: event.modifierFlags)
            if chord == ShortcutStore.shared.chord(for: .nextThread) {
                selectThread?(1)
                return true
            }
            if chord == ShortcutStore.shared.chord(for: .previousThread) {
                selectThread?(-1)
                return true
            }
        }
        return super.performKeyEquivalent(with: event)
    }
}

/// A window beside the main one, Settings say, where Command-W closes it as it does in
/// any Mac app. The chord is the archive shortcut in the main window and, left alone,
/// would reach the Thread menu from here too.
final class SecondaryWindow: NSWindow {
    override func performKeyEquivalent(with event: NSEvent) -> Bool {
        if attachedSheet == nil, KeyChord(keyCode: Int(event.keyCode), modifiers: event.modifierFlags) == KeyChord(keyCode: 13, modifiers: .command) {
            performClose(nil)
            return true
        }
        return super.performKeyEquivalent(with: event)
    }
}
