// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import AppKit

final class ThreadWindow: DCDisplayCycleGuardedWindow {
    var selectThread: ((Int) -> Void)?
    /// ⌘1 to ⌘9, taken before the event is dispatched. AppKit hands a chord that a menu item
    /// carries to the item without asking the window, and firing the item flashes its menu
    /// title, which holds the main thread for a tenth of a second on every switch.
    var selectThreadNumber: ((Int) -> Void)? {
        didSet {
            guard digitMonitor == nil else { return }
            digitMonitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak self] event in
                guard let self, event.window === self, attachedSheet == nil,
                      event.modifierFlags.intersection(KeyChord.allowedModifiers) == .command,
                      let number = Self.digitKeyCodes[event.keyCode], let selectThreadNumber else { return event }
                selectThreadNumber(number)
                return nil
            }
        }
    }
    private var digitMonitor: Any?
    /// Command-Return with the keyboard in no text being typed: approves the selected chat's
    /// top request, as the button's own shortcut did before the chord became the chat box's
    /// send (see `AppModel.approveSelectedRequest`). True when a request was approved.
    var approveRequest: (() -> Bool)?

    /// The number row, by key code, for ⌘1 to ⌘9.
    private static let digitKeyCodes: [UInt16: Int] = [18: 1, 19: 2, 20: 3, 21: 4, 23: 5, 22: 6, 26: 7, 28: 8, 25: 9]

    override func performKeyEquivalent(with event: NSEvent) -> Bool {
        guard attachedSheet == nil else { return super.performKeyEquivalent(with: event) }
        let chord = KeyChord(keyCode: Int(event.keyCode), modifiers: event.modifierFlags)
        // The thread switches alone may use Tab (see `ShortcutStore.refusal`).
        if chord.keyCode == KeyChord.tabKeyCode {
            if chord == ShortcutStore.shared.chord(for: .nextThread) {
                selectThread?(1)
                return true
            }
            if chord == ShortcutStore.shared.chord(for: .previousThread) {
                selectThread?(-1)
                return true
            }
        }
        // A box or field being typed in keeps the chord: a chat box sends with it, or
        // approves its own chat's request when empty. Anywhere else in the window it approves.
        if chord == .composerSend, (firstResponder as? NSTextView)?.isEditable != true, approveRequest?() == true {
            return true
        }
        return super.performKeyEquivalent(with: event)
    }
}

/// A window beside the main one, Settings say, where Command-W closes it as it does in
/// any Mac app. The chord is the archive shortcut in the main window and, left alone,
/// would reach the Thread menu from here too.
final class SecondaryWindow: DCDisplayCycleGuardedWindow {
    /// Escape closes it as well, like a sheet. Taken here rather than in `keyDown`, which the
    /// SwiftUI hosting view never passes on. A field being typed in keeps its Escape, and a
    /// shortcut being recorded takes it before any window sees it.
    override func sendEvent(_ event: NSEvent) {
        if event.type == .keyDown, event.keyCode == 53,
           event.modifierFlags.intersection(KeyChord.allowedModifiers).isEmpty,
           !(firstResponder is NSText) {
            performClose(nil)
            return
        }
        super.sendEvent(event)
    }

    override func performKeyEquivalent(with event: NSEvent) -> Bool {
        // The chord no command may take (see `ShortcutStore.refusal`), so this never swallows one.
        if attachedSheet == nil, KeyChord(keyCode: Int(event.keyCode), modifiers: event.modifierFlags) == .closeWindow {
            performClose(nil)
            return true
        }
        return super.performKeyEquivalent(with: event)
    }
}
