// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import AppKit
import SwiftUI

private struct RightClickPopoverModifier: ViewModifier {
    let isEnabled: Bool
    let actions: () -> [RowAction]

    func body(content: Content) -> some View {
        // The catcher and the popover live in an overlay of the view's own size, so a menu
        // that is not mounted yet (see `isEnabled`) costs the view nothing, and mounting it
        // later leaves the view itself as it was.
        content.overlay {
            if isEnabled {
                RightClickMenuLayer(actions: actions)
            }
        }
    }
}

/// The catcher over the view, and the popover it opens at the pointer.
private struct RightClickMenuLayer: View {
    let actions: () -> [RowAction]
    @State private var anchor: CGPoint?

    var body: some View {
        RightClickCatcher { point in anchor = point }
            .popover(
                isPresented: Binding(get: { anchor != nil }, set: { if !$0 { anchor = nil } }),
                attachmentAnchor: .rect(.rect(CGRect(origin: anchor ?? .zero, size: CGSize(width: 1, height: 1)))),
                arrowEdge: .top
            ) {
                PopoverMenu { RowActionItems(actions: actions()) }
            }
    }
}

extension View {
    /// A right-click or control-click anywhere on the view opens a `PopoverMenu` of `actions` at the pointer.
    /// While `isEnabled` is false nothing is mounted: a kind of row a thread holds by the
    /// hundred passes true only once the pointer has been on it, since a right-click always
    /// follows a hover.
    func rightClickPopover(isEnabled: Bool = true, actions: @escaping () -> [RowAction]) -> some View {
        modifier(RightClickPopoverModifier(isEnabled: isEnabled, actions: actions))
    }
}

private struct RightClickCatcher: NSViewRepresentable {
    var onRightClick: (CGPoint) -> Void

    func makeNSView(context: Context) -> RightClickCatcherView {
        RightClickCatcherView(onRightClick: onRightClick)
    }

    func updateNSView(_ nsView: RightClickCatcherView, context: Context) {
        nsView.onRightClick = onRightClick
    }
}

/// A transparent overlay that only claims secondary clicks. It returns nil from
/// hit-testing for every other event, so text selection, links, buttons and
/// drags in the SwiftUI content underneath behave exactly as if it were absent.
private final class RightClickCatcherView: NSView {
    var onRightClick: (CGPoint) -> Void

    init(onRightClick: @escaping (CGPoint) -> Void) {
        self.onRightClick = onRightClick
        super.init(frame: .zero)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { nil }

    override var isFlipped: Bool { true }

    override var mouseDownCanMoveWindow: Bool { false }

    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }

    override func hitTest(_ point: NSPoint) -> NSView? {
        guard let event = NSApp.currentEvent else { return nil }
        let isSecondary = event.type == .rightMouseDown
            || (event.type == .leftMouseDown && event.modifierFlags.contains(.control))
        guard isSecondary, bounds.contains(convert(point, from: superview)) else { return nil }
        return self
    }

    override func rightMouseDown(with event: NSEvent) {
        onRightClick(convert(event.locationInWindow, from: nil))
    }

    override func mouseDown(with event: NSEvent) {
        if event.modifierFlags.contains(.control) {
            onRightClick(convert(event.locationInWindow, from: nil))
        } else {
            super.mouseDown(with: event)
        }
    }

    override func rightMouseUp(with event: NSEvent) {}
}
