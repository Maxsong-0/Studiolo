import AppKit
import SwiftUI
import UniformTypeIdentifiers

enum ComposerKey {
    case up
    case down
    case tab
    case escape
    case submit
    /// Command-Return: steer the running turn (queue as a follow-up).
    case steer
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

    /// Shows the slash/@ suggestions directly above the caret. A SwiftUI
    /// `.popover` anchors to the whole composer bubble, so it lands centered
    /// (or flipped to the side) instead of where the user is typing.
    func showSuggestions(_ content: AnyView, itemCount: Int) {
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
        // on the caret while the list changes under typing.
        let size = NSSize(width: 340, height: min(340, 44 + CGFloat(max(itemCount, 1)) * 26))
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

    var isShowingSuggestions: Bool {
        suggestionPopover?.isShown == true
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
        return super.performKeyEquivalent(with: event)
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
    /// floating panel's never does, so a head arriving or coming on stage while the user
    /// types in the chat leaves the caret where it is.
    var takesFocusOnAppear = true

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
        textView.font = .systemFont(ofSize: 14)
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
                let flags = NSApp.currentEvent?.modifierFlags ?? []
                if flags.contains(.shift) || flags.contains(.option) {
                    textView.insertNewlineIgnoringFieldEditor(nil)
                    return true
                }
                if flags.contains(.command) {
                    return parent.onKey(.steer)
                }
                return parent.onKey(.submit)
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
            let total = ceil(max(18, contentHeight) + textView.textContainerInset.height * 2)
            if abs(parent.height - total) > 0.5 { parent.height = total }
        }
    }
}
