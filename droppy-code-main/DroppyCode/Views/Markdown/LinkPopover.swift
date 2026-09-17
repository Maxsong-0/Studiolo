import AppKit
import SwiftUI

/// The popover a link in the chat opens on a right-click, in place of the system's text menu:
/// the house rows, under the link. A merge or pull request link gets "Merge" first, wearing
/// the forge's favicon; every link gets Open Link and Copy Link. One popover serves every
/// link paragraph; opening it on another link moves it there.
@MainActor
final class LinkPopover: NSObject, NSPopoverDelegate {
    static let shared = LinkPopover()

    private let popover = NSPopover()
    private var escapeMonitor: Any?

    /// One box for the popover's life, so the rows in it never see a new one.
    private lazy var closeBox = PopoverCloseBox { [weak self] in self?.close() }

    private override init() {
        super.init()
        popover.behavior = .transient
        popover.animates = true
        popover.delegate = self
    }

    /// Opens under `rect` (in `view`'s coordinates), the link's own bounds, so the arrow
    /// points at the words that were clicked.
    func show(url: URL, mergeTarget: MergeRequestTarget?, relativeTo rect: NSRect, of view: NSView) {
        if popover.isShown { popover.close() }
        let request = mergeTarget == nil ? nil : MergeRequestLink(url: url)
        let content = LinkPopoverContent(url: url, request: request) { [weak self] request in
            self?.close()
            mergeTarget?.merge(request)
        }
        .environment(\.closePopoverBox, closeBox)
        var size = NSHostingView(rootView: content).intrinsicContentSize
        if size.width <= 0 || size.height <= 0 { size = NSSize(width: 220, height: 110) }
        popover.setFixedContent(content, size: size)
        // `.minY` is below the words in the flipped text view.
        popover.show(relativeTo: rect, of: view, preferredEdge: .minY)
        startEscapeMonitor()
    }

    func close() {
        stopEscapeMonitor()
        if popover.isShown { popover.performClose(nil) }
    }

    func popoverDidClose(_ notification: Notification) {
        stopEscapeMonitor()
    }

    /// Escape closes it like a menu; the rows take no focus, so the popover never sees the key itself.
    private func startEscapeMonitor() {
        guard escapeMonitor == nil else { return }
        escapeMonitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak self] event in
            guard event.keyCode == 53 else { return event }
            self?.close()
            return nil
        }
    }

    private func stopEscapeMonitor() {
        if let escapeMonitor { NSEvent.removeMonitor(escapeMonitor) }
        escapeMonitor = nil
    }
}

private struct LinkPopoverContent: View {
    let url: URL
    let request: MergeRequestLink?
    let merge: @MainActor (MergeRequestLink) -> Void

    @State private var favicon: NSImage?

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            if let request {
                PopoverItem(
                    request.title,
                    text: Text("\(Text("Merge").fontWeight(.semibold)) \(Text(verbatim: request.label))"),
                    symbol: favicon == nil ? "arrow.triangle.merge" : nil,
                    image: favicon
                ) {
                    merge(request)
                }
                PopoverDivider()
            }
            PopoverItem("Open Link", symbol: "safari") {
                NSWorkspace.shared.open(url)
            }
            PopoverItem("Copy Link", symbol: "link") {
                NSPasteboard.general.clearContents()
                NSPasteboard.general.setString(url.absoluteString, forType: .string)
            }
        }
        .padding(6)
        .frame(width: 220, alignment: .leading)
        .task {
            // The paragraph usually has the icon already; a link whose host has none yet gets it here.
            guard request != nil, let host = url.host?.lowercased() else { return }
            if let cached = FaviconCache.cached(host: host) {
                favicon = cached
            } else {
                favicon = await FaviconCache.image(for: host)
            }
        }
    }
}
