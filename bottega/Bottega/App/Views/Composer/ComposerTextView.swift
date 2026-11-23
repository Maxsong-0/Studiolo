// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import AppKit
import SwiftUI
import UniformTypeIdentifiers

enum ComposerKey {
    case up
    case down
    case tab
    case escape
    /// Command-Return (`KeyChord.composerSend`): send the draft, stopping a running turn for
    /// it; with nothing typed, approve the request the agent is waiting on.
    case submit
    /// The Queue a chat chord typed in the box: line the draft up behind the running turn.
    case queue
    /// Return on its own, or with Shift or Option: a new line, unless a list is open to pick
    /// from. Return never sends, so the one that commits an input method's word, or lands a
    /// beat early, never sends half a message.
    case newline
    /// Delete with the caret at the very start and nothing selected: the chips before the text take it.
    case deleteAtStart
}

/// Lets SwiftUI reach into the text view for focus and in-place replacements.
@MainActor
final class ComposerController {
    fileprivate weak var textView: ComposerNSTextView?
    private var suggestionPopover: NSPopover?
    private var suggestionHost: NSHostingController<AnyView>?
    /// The slash/@ refresh in flight for the latest keystroke; the next keystroke cancels it.
    /// The search for the caret's word, for the suggestions popover. One at a time: the
    /// next keystroke cancels it, and so does losing focus (see the composer's `onBlur`),
    /// so an answer for a word that is gone never shows.
    var suggestionRefresh: Task<Void, Never>?

    var cursorLocation: Int {
        textView?.selectedRange().location ?? 0
    }

    var hasFocus: Bool {
        guard let textView else { return false }
        return NSApp.isActive && textView.window?.isKeyWindow == true && textView.window?.firstResponder === textView
    }

    func focus() {
        guard let textView else { return }
        textView.window?.makeFirstResponder(textView)
    }

    func replaceCharacters(in range: NSRange, with replacement: String) {
        guard let textView, let storage = textView.textStorage, NSMaxRange(range) <= storage.length else { return }
        guard textView.shouldChangeText(in: range, replacementString: replacement) else { return }
        storage.replaceCharacters(in: range, with: replacement)
        textView.didChangeText()
        textView.setSelectedRange(NSRange(location: range.location + (replacement as NSString).length, length: 0))
    }

    /// A suggestion row's height at the stock text size (see `SuggestionPopoverRow`).
    static let suggestionRowHeight: CGFloat = 26

    /// Shows the slash/@ suggestions directly above the caret. A SwiftUI
    /// `.popover` anchors to the whole composer bubble, so it lands centered
    /// (or flipped to the side) instead of where the user is typing. `zoom` is the
    /// window's text size, which the rows are set in.
    func showSuggestions(_ content: AnyView, itemCount: Int, zoom: CGFloat) {
        let popover: NSPopover
        if let existing = suggestionPopover {
            popover = existing
        } else {
            popover = NSPopover()
            popover.behavior = .applicationDefined
            popover.animates = true
            suggestionPopover = popover
        }
        guard let textView = textView, textView.window != nil else { return }
        // Fixed width matching the old menu; height fits the rows so the
        // ScrollView inside never needs to scroll for the capped item count.
        // The host never publishes its own size (see setFixedContent), so this
        // is the only size the panel is ever positioned against and it stays
        // on the caret while the list changes under typing. Both are the stock
        // ones at the window's text size, which the rows are set in.
        let side = ChatZoom.scaled(340, by: zoom)
        let rows = CGFloat(max(itemCount, 1)) * ChatZoom.scaled(Self.suggestionRowHeight, by: zoom)
        let size = NSSize(width: side, height: min(side, ChatZoom.scaled(44, by: zoom) + rows))
        let sized = AnyView(content.frame(width: size.width, height: size.height).presentedChrome())
        if let host = suggestionHost {
            host.rootView = sized
        } else {
            let host = NSHostingController(rootView: sized)
            host.sizingOptions = []
            suggestionHost = host
            popover.contentViewController = host
        }
        popover.contentSize = size
        popover.show(relativeTo: caretRect(in: textView), of: textView, preferredEdge: .maxY)
        // The popover must never steal typing focus.
        textView.window?.makeFirstResponder(textView)
    }

    func hideSuggestions() {
        suggestionPopover?.performClose(nil)
    }

    /// True when the current click landed inside the suggestions popover,
    /// so losing text focus to pick a row must not dismiss the list first.
    func isClickInsideSuggestions() -> Bool {
        guard let popover = suggestionPopover, popover.isShown,
              let window = suggestionHost?.view.window else { return false }
        return window.frame.contains(NSEvent.mouseLocation)
    }

    private func caretRect(in textView: NSTextView) -> NSRect {
        let selected = textView.selectedRange()
        let screenRect = textView.firstRect(forCharacterRange: selected, actualRange: nil)
        if let window = textView.window, screenRect.width >= 0, screenRect.height > 0 {
            let windowRect = window.convertFromScreen(screenRect)
            var rect = textView.convert(windowRect, from: nil)
            if rect.width < 1 { rect.size.width = 1 }
            if rect.height < 4 { rect.size.height = 18 }
            return rect
        }
        // Fallback: leading edge of the visible text.
        var rect = textView.visibleRect
        rect.origin.x += 4
        rect.size = NSSize(width: 1, height: 18)
        return rect
    }
}

final class ComposerNSTextView: NSTextView {
    var placeholder = "" {
        didSet { needsDisplay = true }
    }
    var onAttach: (([AttachmentSource]) -> Void)?
    var onWidthChange: (() -> Void)?
    var onResign: (() -> Void)?
    /// The box's own chords, pressed while it has the keyboard: Command-Return
    /// (`KeyChord.composerSend`) and the Queue a chat chord. True when the box took the key.
    var onChord: (@MainActor (ComposerKey) -> Bool)?

    override func setFrameSize(_ newSize: NSSize) {
        let widthChanged = abs(newSize.width - frame.width) > 0.5
        super.setFrameSize(newSize)
        if widthChanged { onWidthChange?() }
    }

    override func resignFirstResponder() -> Bool {
        let ok = super.resignFirstResponder()
        if ok { onResign?() }
        return ok
    }

    /// The image types a paste attaches, best first: a PNG is stored as it is, the rest
    /// are re-encoded off the main thread.
    private static let imageTypes: [UTType] = [.png, .tiff, .jpeg, .heic]
    private static let imagePasteboardTypes = imageTypes.map { NSPasteboard.PasteboardType($0.identifier) }

    /// Files, or image data with no text beside it, which paste as attachments instead of text.
    private var pasteboardHoldsAttachment: Bool {
        let pasteboard = NSPasteboard.general
        if pasteboard.canReadObject(forClasses: [NSURL.self], options: [.urlReadingFileURLsOnly: true]) { return true }
        return pasteboard.availableType(from: Self.imagePasteboardTypes) != nil && pasteboard.string(forType: .string) == nil
    }

    /// A plain-text view disables Paste when the clipboard has no text, so an image never reached `paste(_:)`.
    override func validateUserInterfaceItem(_ item: any NSValidatedUserInterfaceItem) -> Bool {
        if item.action == #selector(paste(_:)), pasteboardHoldsAttachment { return true }
        return super.validateUserInterfaceItem(item)
    }

    override func performKeyEquivalent(with event: NSEvent) -> Bool {
        let flags = event.modifierFlags.intersection(.deviceIndependentFlagsMask)
        if window?.firstResponder === self, flags == .command, event.charactersIgnoringModifiers == "v", pasteboardHoldsAttachment {
            paste(nil)
            return true
        }
        // The box's chords reach the box with the keyboard before any menu or button in the
        // window: the send, which no command may take, so it never rests on how the text
        // system reads Command-Return; and the queue chord, whose menu item serves the chat
        // on screen, so typed in a hand's box it queues that box's draft. A word an input
        // method is still composing keeps its keys.
        if window?.firstResponder === self, !hasMarkedText(), let onChord, let key = Self.chordKey(for: event), onChord(key) {
            return true
        }
        return super.performKeyEquivalent(with: event)
    }

    /// The composer key a chord stands for, or nil for a chord the box leaves alone. Enter
    /// on the keypad reads as Return, the way the text system reads it.
    @MainActor
    private static func chordKey(for event: NSEvent) -> ComposerKey? {
        let keyCode = event.keyCode == 76 ? 36 : Int(event.keyCode)
        let chord = KeyChord(keyCode: keyCode, modifiers: event.modifierFlags)
        if chord == ShortcutStore.shared.chord(for: .queueChat) { return .queue }
        if chord == .composerSend { return .submit }
        return nil
    }

    override func paste(_ sender: Any?) {
        let pasteboard = NSPasteboard.general
        if let urls = pasteboard.readObjects(forClasses: [NSURL.self], options: [.urlReadingFileURLsOnly: true]) as? [URL],
           !urls.isEmpty {
            onAttach?(urls.map(AttachmentSource.file))
            return
        }
        if pasteboard.string(forType: .string) == nil,
           let available = pasteboard.availableType(from: Self.imagePasteboardTypes),
           let type = UTType(available.rawValue),
           let data = pasteboard.data(forType: available) {
            onAttach?([.image(data, type)])
            return
        }
        pasteAsPlainText(sender)
    }

    override func performDragOperation(_ sender: NSDraggingInfo) -> Bool {
        if let urls = sender.draggingPasteboard.readObjects(forClasses: [NSURL.self], options: [.urlReadingFileURLsOnly: true]) as? [URL],
           !urls.isEmpty {
            onAttach?(urls.map(AttachmentSource.file))
            return true
        }
        return super.performDragOperation(sender)
    }

    /// Whether the last draw showed the placeholder: the view is redrawn whole only when
    /// the text becomes empty or stops being empty; other keystrokes redraw their lines.
    private var placeholderShown = false

    override func didChangeText() {
        super.didChangeText()
        if string.isEmpty != placeholderShown { needsDisplay = true }
    }

    override func draw(_ dirtyRect: NSRect) {
        super.draw(dirtyRect)
        placeholderShown = string.isEmpty && !placeholder.isEmpty
        guard placeholderShown else { return }
        let attributes: [NSAttributedString.Key: Any] = [
            .font: font ?? NSFont.systemFont(ofSize: 14),
            .foregroundColor: NSColor.placeholderTextColor,
        ]
        let origin = NSPoint(
            x: textContainerInset.width + (textContainer?.lineFragmentPadding ?? 0),
            y: textContainerInset.height
        )
        (placeholder as NSString).draw(at: origin, withAttributes: attributes)
    }
}

struct ComposerTextView: NSViewRepresentable {
    @Binding var text: String
    @Binding var height: CGFloat
    var placeholder: String
    let controller: ComposerController
    var onKey: (ComposerKey) -> Bool
    var onAttach: ([AttachmentSource]) -> Void
    var onCursorChange: (Int) -> Void
    var onBlur: () -> Void = {}
    /// Whether the text takes typing focus as it appears. The chat's own box does; a
    /// floating panel's never does, so a hand arriving or coming on stage while the user
    /// types in the chat leaves the caret where it is.
    var takesFocusOnAppear = true

    /// The box's type at the stock step, in points.
    static let stockFontSize: CGFloat = 14

    /// The box's font at the window's text size (see `ChatZoom`): the stock 14 points at
    /// the stock step. The pill's height math reads the same font (see `ComposerTextColumn`).
    static func font(zoom: CGFloat) -> NSFont {
        .systemFont(ofSize: ChatZoom.scaled(stockFontSize, by: zoom))
    }

    func makeCoordinator() -> Coordinator {
        Coordinator(parent: self)
    }

    func makeNSView(context: Context) -> NSScrollView {
        let scrollView = NSScrollView()
        scrollView.drawsBackground = false
        scrollView.borderType = .noBorder
        scrollView.hasVerticalScroller = true
        scrollView.autohidesScrollers = true
        scrollView.scrollerStyle = .overlay

        let textView = ComposerNSTextView(frame: .zero)
        textView.delegate = context.coordinator
        textView.isRichText = false
        textView.importsGraphics = false
        textView.allowsUndo = true
        textView.drawsBackground = false
        textView.font = Self.font(zoom: context.environment.chatZoom)
        textView.textColor = .labelColor
        textView.insertionPointColor = Chrome.accentNSColor
        textView.textContainerInset = NSSize(width: 0, height: 2)
        textView.isAutomaticQuoteSubstitutionEnabled = false
        textView.isAutomaticDashSubstitutionEnabled = false
        textView.isAutomaticTextReplacementEnabled = false
        textView.isAutomaticSpellingCorrectionEnabled = false
        textView.isAutomaticTextCompletionEnabled = false
        // This field must never offer Write with Siri, writing-tools UI, inline
        // predictions, or Siri completion candidates: it has its own slash/@
        // popover. `.none` rather than `.limited`, which still offers the Writing
        // Tools panel and its Siri chip from the Edit and context menus. Scoped to
        // this view so every other system field is unaffected.
        textView.inlinePredictionType = .no
        textView.writingToolsBehavior = .none
        textView.isVerticallyResizable = true
        textView.isHorizontallyResizable = false
        textView.autoresizingMask = [.width]
        textView.minSize = .zero
        textView.maxSize = NSSize(width: CGFloat.greatestFiniteMagnitude, height: CGFloat.greatestFiniteMagnitude)
        textView.textContainer?.widthTracksTextView = true
        textView.textContainer?.lineFragmentPadding = 0
        textView.placeholder = placeholder
        textView.string = text
        textView.onAttach = onAttach

        scrollView.documentView = textView
        controller.textView = textView
        let coordinator = context.coordinator
        coordinator.textView = textView
        textView.onWidthChange = { [weak coordinator] in
            Task { @MainActor in coordinator?.updateHeight() }
        }
        textView.onResign = { [weak coordinator] in
            Task { @MainActor in coordinator?.parent.onBlur() }
        }
        textView.onChord = { [weak coordinator] key in
            coordinator?.parent.onKey(key) ?? false
        }
        Task { @MainActor in
            // And never from another box the user is typing in, whichever one this is.
            if takesFocusOnAppear, let window = textView.window, !(window.firstResponder is ComposerNSTextView) {
                window.makeFirstResponder(textView)
            }
            coordinator.updateHeight()
        }
        return scrollView
    }

    func updateNSView(_ scrollView: NSScrollView, context: Context) {
        context.coordinator.parent = self
        guard let textView = scrollView.documentView as? ComposerNSTextView else { return }
        textView.onAttach = onAttach
        if textView.placeholder != placeholder { textView.placeholder = placeholder }
        // The caret follows the theme's accent; a theme change never rebuilds the view.
        let caret = Chrome.accentNSColor
        if textView.insertionPointColor != caret { textView.insertionPointColor = caret }
        // The text size follows Settings the same way, and the pill regrows to the new lines.
        let font = Self.font(zoom: context.environment.chatZoom)
        if textView.font != font {
            textView.font = font
            textView.needsDisplay = true
            Task { @MainActor in context.coordinator.updateHeight() }
        }
        if !textView.hasMarkedText(), textView.string != text {
            textView.string = text
            textView.setSelectedRange(NSRange(location: (text as NSString).length, length: 0))
            Task { @MainActor in context.coordinator.updateHeight() }
        }
    }

    @MainActor
    final class Coordinator: NSObject, NSTextViewDelegate {
        var parent: ComposerTextView
        weak var textView: ComposerNSTextView?

        init(parent: ComposerTextView) {
            self.parent = parent
        }

        func textDidChange(_ notification: Notification) {
            guard let textView else { return }
            parent.text = textView.string
            updateHeight()
            parent.onCursorChange(textView.selectedRange().location)
        }

        func textViewDidChangeSelection(_ notification: Notification) {
            guard let textView else { return }
            parent.onCursorChange(textView.selectedRange().location)
        }

        func textView(_ textView: NSTextView, doCommandBy selector: Selector) -> Bool {
            if textView.hasMarkedText() { return false }
            switch selector {
            case #selector(NSResponder.insertNewline(_:)):
                // Command-Return sends and Return adds a line (see `ComposerKey`). The box
                // takes Command-Return and the queue chord as key equivalents (see
                // `ComposerNSTextView.performKeyEquivalent`), so they land here only when
                // nothing took them there.
                let flags = (NSApp.currentEvent?.modifierFlags ?? []).intersection(KeyChord.allowedModifiers)
                let key: ComposerKey
                if KeyChord(keyCode: 36, modifiers: flags) == ShortcutStore.shared.chord(for: .queueChat) {
                    key = .queue
                } else if flags.contains(.command) {
                    key = .submit
                } else {
                    key = .newline
                }
                if parent.onKey(key) { return true }
                guard key == .newline else { return false }
                textView.insertNewlineIgnoringFieldEditor(nil)
                return true
            case #selector(NSResponder.moveUp(_:)):
                return parent.onKey(.up)
            case #selector(NSResponder.moveDown(_:)):
                return parent.onKey(.down)
            case #selector(NSResponder.insertTab(_:)):
                return parent.onKey(.tab)
            case #selector(NSResponder.cancelOperation(_:)):
                return parent.onKey(.escape)
            case #selector(NSResponder.deleteBackward(_:)):
                if textView.selectedRange() == NSRange(location: 0, length: 0) {
                    return parent.onKey(.deleteAtStart)
                }
                return false
            default:
                return false
            }
        }

        func updateHeight() {
            guard let textView, textView.bounds.width > 24 else { return }
            var contentHeight: CGFloat = 18
            if let layoutManager = textView.textLayoutManager {
                layoutManager.ensureLayout(for: layoutManager.documentRange)
                contentHeight = layoutManager.usageBoundsForTextContainer.height
            } else if let layoutManager = textView.layoutManager, let container = textView.textContainer {
                // ComposerNSTextView overrides draw(_:) for the placeholder, and AppKit
                // answers any draw override by falling back to TextKit 1. With no
                // textLayoutManager the height stayed at the one-line default, so the
                // pill never grew.
                layoutManager.ensureLayout(for: container)
                contentHeight = layoutManager.usedRect(for: container).height
            }
            // One line's floor, 18 at the stock 14 points, grows with the box's font.
            let lineFloor = 18 * (textView.font?.pointSize ?? ComposerTextView.stockFontSize) / ComposerTextView.stockFontSize
            let total = ceil(max(lineFloor, contentHeight) + textView.textContainerInset.height * 2)
            if abs(parent.height - total) > 0.5 { parent.height = total }
        }
    }
}
