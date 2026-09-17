import AppKit
import SwiftUI

/// The sidebar's thread list as a floating glass panel over the chat, in place of the
/// popover from the toolbar button while the sidebar's column is collapsed. The strip
/// along the top is the handle, with the title at its left and, while the column is
/// still around, a button at its right that puts the list back into the column. It
/// stays where it was last left, and the grip at its bottom-right corner resizes it.
struct FloatingSidebarPanel: View {
    @Environment(AppModel.self) private var model
    @Environment(\.colorScheme) private var colorScheme
    /// The chat area it floats in.
    let area: CGSize
    let canClose: Bool
    let close: () -> Void

    static let defaultSize = CGSize(width: 300, height: 560)
    static let minSize = CGSize(width: 240, height: 320)
    static let margin: CGFloat = 12
    static let stripHeight: CGFloat = 44

    /// The panel's top-left while the strip is held.
    @State private var dragOrigin: CGPoint?
    @State private var grab: CGPoint?
    @State private var resizeStart: CGSize?

    var body: some View {
        let stored = model.settings.sidebarPanelSize ?? Self.defaultSize
        let size = CGSize(
            width: min(max(Self.minSize.width, stored.width), max(Self.minSize.width, area.width - Self.margin * 2)),
            height: min(max(Self.minSize.height, stored.height), max(Self.minSize.height, area.height - Self.margin * 2))
        )
        let rest = Self.clamp(model.settings.sidebarPanelOrigin ?? CGPoint(x: Self.margin, y: Chrome.contentTopInset), size: size, in: area)
        let origin = dragOrigin ?? rest
        let shape = RoundedRectangle(cornerRadius: HydraPanel.cornerRadius, style: .continuous)
        SidebarView(inPopover: true, dismiss: nil)
            .padding(.top, Self.stripHeight)
            .frame(width: size.width, height: size.height)
            .overlay(alignment: .top) {
                HStack(alignment: .top, spacing: 8) {
                    Text("Threads")
                        .font(.system(size: 12.5, weight: .medium))
                        .foregroundStyle(Chrome.primaryText.opacity(0.92))
                        .padding(.horizontal, Chrome.capsuleHorizontalPadding)
                        .frame(height: Chrome.capsuleContentHeight)
                        .padding(.vertical, Chrome.capsuleVerticalPadding)
                        .fixedSize()
                        .chromeGlassCapsule()
                        // The title is a label, not a control, so it drags the panel too.
                        .overlay {
                            PanelDragHandle(onDrag: onDrag(rest: rest, size: size), onDragEnd: onDragEnd)
                        }
                        .padding(.top, Chrome.chromeTopPadding)
                        .padding(.leading, Chrome.chromeHorizontalPadding)
                    PanelDragHandle(onDrag: onDrag(rest: rest, size: size), onDragEnd: onDragEnd)
                        .frame(maxWidth: .infinity)
                        .frame(height: Self.stripHeight)
                        .help("Drag to move")
                        .accessibilityLabel(Text("Drag to move"))
                    if canClose {
                        ChromeCircleButton(symbol: "xmark", help: "Back into the column") {
                            close()
                        }
                        .padding(.top, Chrome.chromeTopPadding)
                        .padding(.trailing, Chrome.chromeHorizontalPadding)
                    }
                }
            }
            .overlay(alignment: .bottomTrailing) {
                PanelDragHandle(
                    onDrag: { translation in
                        let start = resizeStart ?? size
                        resizeStart = start
                        var next = CGSize(width: start.width + translation.width, height: start.height + translation.height)
                        next.width = min(max(Self.minSize.width, next.width), area.width - origin.x - Self.margin)
                        next.height = min(max(Self.minSize.height, next.height), area.height - origin.y - Self.margin)
                        model.settings.sidebarPanelSize = next
                    },
                    onDragEnd: { resizeStart = nil },
                    cursor: .frameResize(position: .bottomRight, directions: .all),
                    dragCursor: .frameResize(position: .bottomRight, directions: .all)
                )
                .frame(width: 16, height: 16)
                .help("Drag to resize")
            }
            .background {
                let isDark = colorScheme == .dark
                shape
                    .fill(.clear)
                    .glassEffect(.regular, in: shape)
                    .overlay {
                        shape.fill(Chrome.glassTint.opacity(isDark ? 0.22 : 0.16))
                    }
            }
            .clipShape(shape)
            .background {
                let isDark = colorScheme == .dark
                shape
                    .fill((isDark ? Color.black : Color.white).opacity(isDark ? 0.3 : 0.34))
                    .shadow(color: .black.opacity(isDark ? 1 : 0.65), radius: 28, y: 10)
            }
            .offset(x: origin.x, y: origin.y)
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            .animation(dragOrigin == nil ? Chrome.panelSlide : nil, value: origin)
    }

    private func onDrag(rest: CGPoint, size: CGSize) -> (CGSize) -> Void {
        { translation in
            let start = grab ?? rest
            grab = start
            dragOrigin = Self.clamp(CGPoint(x: start.x + translation.width, y: start.y + translation.height), size: size, in: area)
        }
    }

    private func onDragEnd() {
        if let dragOrigin { model.settings.sidebarPanelOrigin = dragOrigin }
        dragOrigin = nil
        grab = nil
    }

    private static func clamp(_ origin: CGPoint, size: CGSize, in area: CGSize) -> CGPoint {
        CGPoint(
            x: min(max(margin, origin.x), max(margin, area.width - size.width - margin)),
            y: min(max(margin, origin.y), max(margin, area.height - size.height - margin))
        )
    }
}
