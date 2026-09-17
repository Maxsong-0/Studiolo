import AppKit
import SwiftUI

/// One attachment preview panel per thumbnail strip. A custom NSPopover (not
/// SwiftUI's `.popover`) because SwiftUI resolves several sibling popovers to
/// a single presentation, so with several pics only the last photo opened.
/// Here one panel is shared and every tap swaps its content, so any photo
/// opens in a single tap.
///
/// The panel anchors to the tapped thumbnail's own view, so the arrow lands
/// on the photo every time. While open, tapping another photo swaps the
/// content and re-shows at the new anchor, so the panel follows the tap
/// instead of staying stranded at the previous photo.
///
/// The panel is application-defined: taps elsewhere in the window still reach
/// their target, and the panel closes on its own monitors instead — a tap
/// outside the strip dismisses it, as does Escape, the window losing key or
/// moving, tapping the same thumbnail again, or the strip going away.
@MainActor
final class AttachmentPreviewCoordinator: NSObject, NSPopoverDelegate {
    private let popover = NSPopover()
    private var anchor: WeakView?
    /// The strip's place in its window when it has no view of its own to hang from
    /// (see `WindowRectAnchor`).
    private var anchorRect: CGRect?
    private var currentID: Attachment.ID?
    private enum Monitor {
        case event(Any)
        case observer(NSObjectProtocol)
    }
    private var monitors: [Monitor] = []

    override init() {
        super.init()
        popover.behavior = .applicationDefined
        popover.animates = true
        popover.delegate = self
    }

    /// The strip's own view, captured from the strip's background. One stable
    /// view per strip: never a recycled row, never a stale id.
    func setAnchor(_ view: NSView) {
        anchor = WeakView(view)
    }

    /// The strip's frame in the window's hosting view, for a strip that must not mount
    /// an anchor view of its own. The panel then hangs from that rect on the window's
    /// content view; a view anchor, when there is one, still wins.
    func setAnchor(windowRect: CGRect) {
        anchorRect = windowRect
    }

    /// Forget the panel when its attachment left
    /// (e.g. removed from the draft while previewing).
    func retire(except ids: Set<Attachment.ID>) {
        if let currentID, !ids.contains(currentID) { close() }
    }

    /// `view` is the tapped thumbnail's own view when it has one. The panel aims
    /// at this click's tap point first, so the arrow lands on the photo even if
    /// a stored view went stale; the thumbnail and strip anchors are fallbacks.
    /// `edge` is where the panel opens: above the anchor for a photo in a message, below
    /// a tool row whose chevron points down (`.minY` is below in these flipped views).
    func toggle(_ attachment: Attachment, over view: NSView? = nil, edge: NSRectEdge = .maxY) {
        if currentID == attachment.id, popover.isShown {
            close()
        } else {
            show(attachment, over: view, edge: edge)
        }
    }

    func close() {
        stopMonitors()
        currentID = nil
        if popover.isShown { popover.performClose(nil) }
    }

    private func show(_ attachment: Attachment, over view: NSView?, edge: NSRectEdge) {
        let imageSize: CGSize? = if attachment.isImage {
            AttachmentLargePreview.imageDisplaySize(for: attachment)
        } else if attachment.isVideo {
            AttachmentLargePreview.videoDisplaySize(for: attachment)
        } else {
            nil
        }
        let content = AttachmentLargePreview(attachment: attachment, imageSize: imageSize)
        let size = Self.contentSize(for: attachment, imageSize: imageSize)
        currentID = attachment.id
        guard let (anchor, rect) = anchorTarget(thumbnailView: view, edge: edge) else { return }
        // The panel is sized here, once, and the hosting controller never
        // publishes a size of its own (see setFixedContent): the only size
        // AppKit ever positions against is this one, so the arrow stays on the
        // tapped photo instead of drifting when the content settles.
        //
        // Already open for another photo: re-showing a shown popover moves it
        // above the tapped photo, and the content swap is the backstop so the
        // new photo shows even if a reposition were ever ignored.
        popover.setFixedContent(content, size: size)
        if popover.isShown {
            popover.show(relativeTo: rect, of: anchor, preferredEdge: edge)
        } else if popover.contentViewController?.view.window != nil {
            popover.animates = false
            popover.show(relativeTo: rect, of: anchor, preferredEdge: edge)
            popover.animates = true
        } else {
            popover.show(relativeTo: rect, of: anchor, preferredEdge: edge)
        }
        startMonitors()
    }

    /// The view and rect to anchor the panel to, best proof first. A stored
    /// thumbnail view can go stale (recycled rows, unregistered ids) and land
    /// the arrow far from the tapped photo, so the first choice is this
    /// click's own tap point inside the strip anchor: exactly where the finger
    /// is. The thumbnail's own bounds and the strip's bounds stay as fallbacks
    /// (keyboard/VoiceOver activation carries no click), and nil blocks.
    private func anchorTarget(thumbnailView: NSView?, edge: NSRectEdge) -> (NSView, NSRect)? {
        if let strip = anchor?.value, strip.window != nil,
           strip.bounds.width >= 8, strip.bounds.height >= 8,
           let event = NSApp.currentEvent, event.type == .leftMouseUp,
           event.window === strip.window {
            var point = strip.convert(event.locationInWindow, from: nil)
            point.x = min(max(point.x, strip.bounds.minX + 2), strip.bounds.maxX - 2)
            // A hairline the strip's full height, at the tap's x: the panel opens off
            // whichever edge AppKit picks. Anchored to the edge it was asked for, a panel
            // flipped to the other side for want of room opened over the photo itself.
            let rect = NSRect(x: point.x - 1, y: strip.bounds.minY, width: 2, height: strip.bounds.height)
            return (strip, rect)
        }
        if let thumbnailView, thumbnailView.window != nil {
            return (thumbnailView, thumbnailView.bounds)
        }
        if let strip = anchor?.value, strip.window != nil {
            return (strip, strip.bounds)
        }
        if let anchorRect, let target = WindowRectAnchor.target(for: anchorRect) {
            // The tap's own x again, the strip's full height, when the click that opened
            // it is at hand; the whole strip otherwise.
            if let event = NSApp.currentEvent, event.type == .leftMouseUp, event.window === target.view.window {
                var point = target.view.convert(event.locationInWindow, from: nil)
                point.x = min(max(point.x, target.rect.minX + 2), target.rect.maxX - 2)
                return (target.view, NSRect(x: point.x - 1, y: target.rect.minY, width: 2, height: target.rect.height))
            }
            return (target.view, target.rect)
        }
        return nil
    }

    // MARK: - Dismissal

    nonisolated func popoverDidClose(_ notification: Notification) {
        MainActor.assumeIsolated { [weak self] in
            guard let self else { return }
            if self.popover.isShown { return }
            self.stopMonitors()
            self.currentID = nil
        }
    }

    private func startMonitors() {
        guard monitors.isEmpty else { return }
        if let monitor = NSEvent.addLocalMonitorForEvents(matching: [.leftMouseDown, .rightMouseDown], handler: { [weak self] event in
            self?.handleMouseDown(event) ?? event
        }) {
            monitors.append(.event(monitor))
        }
        if let monitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown, handler: { [weak self] event in
            self?.handleKeyDown(event) ?? event
        }) {
            monitors.append(.event(monitor))
        }
        let anchorWindow = anchor?.value?.window ?? popover.contentViewController?.view.window?.parent
        if let anchorWindow {
            let resign = NotificationCenter.default.addObserver(forName: NSWindow.didResignKeyNotification, object: anchorWindow, queue: .main) { [weak self] _ in
                MainActor.assumeIsolated { self?.close() }
            }
            monitors.append(.observer(resign))
            let willMove = NotificationCenter.default.addObserver(forName: NSWindow.willMoveNotification, object: anchorWindow, queue: .main) { [weak self] _ in
                MainActor.assumeIsolated { self?.close() }
            }
            monitors.append(.observer(willMove))
            let didMove = NotificationCenter.default.addObserver(forName: NSWindow.didMoveNotification, object: anchorWindow, queue: .main) { [weak self] _ in
                MainActor.assumeIsolated { self?.close() }
            }
            monitors.append(.observer(didMove))
        }
    }

    /// Clicks inside the panel or anywhere on the strip pass through (a
    /// thumbnail button toggles or switches the panel); any other click
    /// dismisses first.
    private func handleMouseDown(_ event: NSEvent) -> NSEvent? {
        if event.window === popover.contentViewController?.view.window { return event }
        if let window = event.window,
           let view = anchor?.value, view.window === window, !view.isHiddenOrHasHiddenAncestor,
           view.bounds.contains(view.convert(event.locationInWindow, from: nil)) { return event }
        if let anchorRect, let target = WindowRectAnchor.target(for: anchorRect), target.view.window === event.window,
           target.rect.contains(target.view.convert(event.locationInWindow, from: nil)) { return event }
        close()
        return event
    }

    private func handleKeyDown(_ event: NSEvent) -> NSEvent? {
        guard event.keyCode == 53 else { return event } // Escape
        close()
        return nil
    }

    private func stopMonitors() {
        for monitor in monitors {
            switch monitor {
            case .event(let token): NSEvent.removeMonitor(token)
            case .observer(let token): NotificationCenter.default.removeObserver(token)
            }
        }
        monitors.removeAll()
    }

    /// Fixed size per attachment kind. The large preview loads its image
    /// asynchronously, so the panel cannot size itself to content after it
    /// appears; sizing up front from the same numbers the content lays out
    /// with keeps it from ever resizing once shown.
    private static func contentSize(for attachment: Attachment, imageSize: CGSize?) -> NSSize {
        let chrome: CGFloat = 60 // outer padding + name/footer row + spacing
        if let imageSize {
            return NSSize(width: AttachmentLargePreview.imageBounds.width + 24, height: ceil(imageSize.height) + chrome)
        }
        if let text = textSample(for: attachment) {
            // Measured overhead: outer padding (24) + spacing (10) + footer (~18).
            return NSSize(width: 464, height: min(320, max(120, CGFloat(text.count / 4))) + 60)
        }
        // Measured content (~84: padding + 32pt icon row + spacing + footer), so the
        // footer sits just above the photo instead of floating over dead space.
        return NSSize(width: 344, height: 100)
    }

    private static func textSample(for attachment: Attachment) -> String? {
        let url = attachment.url
        guard let size = try? url.resourceValues(forKeys: [.fileSizeKey]).fileSize, size < 500_000 else { return nil }
        guard let text = try? String(contentsOf: url, encoding: .utf8), !text.isEmpty else { return nil }
        return text
    }
}

/// A weak box for an NSView anchor, so coordinators never keep views alive.
final class WeakView {
    weak var value: NSView?
    init(_ value: NSView? = nil) { self.value = value }
}

/// A row's preview panel, and the view it hangs from, built only once the row needs them.
///
/// A row holds this in `@State`, and SwiftUI builds a row's state defaults every time the
/// row struct is made, not once per row on screen: a coordinator written as the default
/// made an `NSPopover` for every tool row on every rebuild of the timeline, hundreds at a
/// time, for a panel almost none of them ever shows. This box is a pointer and a weak
/// pointer; the panel behind it is made the first time one is opened.
@MainActor
final class AttachmentPreviewSlot {
    /// The view the panel anchors to, captured while the row is under the pointer.
    let anchor = WeakView()
    private var anchorRect: CGRect?
    private var coordinator: AttachmentPreviewCoordinator?

    var panel: AttachmentPreviewCoordinator {
        if let coordinator { return coordinator }
        let made = AttachmentPreviewCoordinator()
        if let view = anchor.value { made.setAnchor(view) }
        if let anchorRect { made.setAnchor(windowRect: anchorRect) }
        coordinator = made
        return made
    }

    func setAnchor(_ view: NSView) {
        anchor.value = view
        coordinator?.setAnchor(view)
    }

    /// The strip's frame in its window, for a strip with no anchor view (see `WindowRectAnchor`).
    func setAnchor(windowRect: CGRect) {
        anchorRect = windowRect
        coordinator?.setAnchor(windowRect: windowRect)
    }

    /// Closes a panel that was opened. A slot that never made one does nothing.
    func close() {
        coordinator?.close()
    }

    /// Forgets a panel whose attachment has left the draft. Nothing to forget until one
    /// has been opened, so a slot that never made a panel does nothing here either.
    func retire(except ids: Set<Attachment.ID>) {
        coordinator?.retire(except: ids)
    }
}

/// A popover anchor with no view of its own: where a SwiftUI view sits in its window, as
/// its geometry reports it, for a popover hung from that rect on the window's content
/// view instead of from a view mounted in the SwiftUI tree.
///
/// Used where an AppKit view must not be placed. An `NSViewRepresentable` joins the walk
/// SwiftUI makes over a window's focus items to rebuild the key view loop, and inside the
/// composer's follow-up queue tab that walk never ended on macOS 26: it handed back the
/// tab's first participant (its chevron, then the anchor view behind its pencil once the
/// buttons had opted out) forever, and the main thread stood still until a force quit
/// (the Sept 15 2026 freezes). The tab holds nothing the walk can visit now; this is how
/// its popovers still find their place.
enum WindowRectAnchor {
    /// The window's content view and `rect` (a `.global` SwiftUI frame, in the hosting
    /// view's flipped coordinates) converted into it. The window is the one the current
    /// event is in, else the key window: a rect only means something in the window whose
    /// hosting view reported it.
    @MainActor
    static func target(for rect: CGRect) -> (view: NSView, rect: NSRect)? {
        guard rect.width > 0, rect.height > 0,
              let window = NSApp.currentEvent?.window ?? NSApp.keyWindow ?? NSApp.mainWindow,
              let content = window.contentView else { return nil }
        let converted = content.isFlipped
            ? rect
            : NSRect(x: rect.minX, y: content.bounds.height - rect.maxY, width: rect.width, height: rect.height)
        return (content, converted)
    }
}

extension View {
    /// Reports this view's frame in its window (the hosting view's coordinates) whenever
    /// it changes, for a `WindowRectAnchor` target. Costs a geometry read, never a view.
    func windowRectAnchor(_ onChange: @escaping (CGRect) -> Void) -> some View {
        onGeometryChange(for: CGRect.self, of: { $0.frame(in: .global) }, action: onChange)
    }
}

/// Captures the strip's own NSView so the preview panel can anchor to it.
/// Mounted as the strip's background, so it fills exactly the strip.
struct AttachmentAnchorCapture: NSViewRepresentable {
    let onResolve: (NSView) -> Void

    func makeNSView(context: Context) -> NSView {
        AnchorView(onResolve: onResolve)
    }

    func updateNSView(_ nsView: NSView, context: Context) {
        (nsView as? AnchorView)?.onResolve = onResolve
        onResolve(nsView)
    }

    private final class AnchorView: NSView {
        var onResolve: ((NSView) -> Void)?

        init(onResolve: @escaping (NSView) -> Void) {
            self.onResolve = onResolve
            super.init(frame: .zero)
        }

        @available(*, unavailable)
        required init?(coder: NSCoder) { nil }

        /// Only an anchor: never in the way of a click or a cursor update.
        override func hitTest(_ point: NSPoint) -> NSView? { nil }

        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            if window != nil { onResolve?(self) }
        }
    }
}
