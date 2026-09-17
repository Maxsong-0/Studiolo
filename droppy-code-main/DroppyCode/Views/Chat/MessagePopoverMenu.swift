import AppKit
import SwiftUI

private struct RightClickPopoverModifier: ViewModifier {
    let actions: () -> [RowAction]
    @State private var anchor: CGPoint?

    func body(content: Content) -> some View {
        content
            .overlay {
                RightClickCatcher { point in anchor = point }
            }
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
    func rightClickPopover(actions: @escaping () -> [RowAction]) -> some View {
        modifier(RightClickPopoverModifier(actions: actions))
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
