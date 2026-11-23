// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import AppKit
import SwiftUI

/// The chat's usage as a floating glass panel, the helper panel's size and recipe: the
/// context window, the plan's limits and the credits of the model in use, or of the lead
/// and the hands when a pair sends them out on another provider. It sits across the
/// column from the hands, at the left by default. The strip along the top is the handle,
/// with the title at its left and the close button at its right.
struct UsageFloatingPanel: View {
    @Environment(WindowLiveResize.self) private var liveResize
    let runtime: ThreadRuntime
    let provider: ProviderKind
    let handsProvider: ProviderKind?
    let size: CGSize
    /// The height the content wants, strip and paddings included, reported whenever it
    /// changes; the placer sizes the panel to it, up to `size.height`.
    let onContentHeight: (CGFloat) -> Void
    /// The pointer's travel since the handle was grabbed.
    let onDrag: (CGSize) -> Void
    let onDragEnd: () -> Void
    let dismiss: () -> Void
    /// The window's text size, for the room the empty panel's line takes.
    @Environment(\.chatZoom) private var zoom

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: FloatingPanelMetrics.cornerRadius, style: .continuous)
        let usage = runtime.usage
        let hasSomething = usage?.fraction != nil
            || PlanLimitsReader.exposesLimits(provider) || CreditsReader.exposesCredits(provider)
            || handsProvider.map { PlanLimitsReader.exposesLimits($0) || CreditsReader.exposesCredits($0) } == true
        Group {
            if hasSomething {
                ScrollView(.vertical) {
                    UsagePanel(usage: usage, provider: provider, handsProvider: handsProvider, fillsWidth: true)
                        .onGeometryChange(for: CGFloat.self, of: { $0.size.height }) { onContentHeight($0 + FloatingPanelMetrics.stripHeight + 8) }
                        .padding(.top, FloatingPanelMetrics.stripHeight)
                        .padding(.bottom, 8)
                }
            } else {
                Text(verbatim: "Nothing to show for \(provider.displayName) yet.")
                    .zoomedFont(size: 13)
                    .foregroundStyle(Chrome.secondaryText)
                    .padding(.top, FloatingPanelMetrics.stripHeight)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    // Again when the text size changes, since the line's height goes with it.
                    .onChange(of: zoom, initial: true) { _, zoom in
                        onContentHeight(FloatingPanelMetrics.stripHeight + ChatZoom.scaled(44, by: zoom))
                    }
            }
        }
        .overlay(alignment: .top) {
            strip
        }
        .frame(width: size.width, height: size.height)
        // The frame follows the pane every frame of a live resize.
        .animation(nil, value: liveResize.isActive)
        // The helper panel's recipe: one glass surface with the theme's tint, and under it
        // a scrim for the text over whatever the panel floats above.
        .floatingPanelSurface(in: shape)
    }

    /// The handle across the top: the title at the left, the close button at the right,
    /// and the room between them drags the panel.
    private var strip: some View {
        HStack(alignment: .top, spacing: 8) {
            HStack(spacing: 6) {
                ProviderIcon(provider: provider, size: 12)
                if let handsProvider {
                    ProviderIcon(provider: handsProvider, size: 12)
                }
                Text("Usage")
                    .zoomedFont(size: 12.5, weight: .medium)
                    .foregroundStyle(Chrome.primaryText.opacity(0.92))
            }
            .padding(.horizontal, Chrome.capsuleHorizontalPadding)
            .frame(height: Chrome.capsuleContentHeight)
            .padding(.vertical, Chrome.capsuleVerticalPadding)
            .fixedSize()
            .chromeGlassCapsule()
            // The title is a label, not a control, so it drags the panel too.
            .overlay {
                PanelDragHandle(onDrag: onDrag, onDragEnd: onDragEnd)
            }
            .padding(.top, Chrome.chromeTopPadding)
            .padding(.leading, Chrome.chromeHorizontalPadding)
            PanelDragHandle(onDrag: onDrag, onDragEnd: onDragEnd)
                .frame(maxWidth: .infinity)
                .frame(height: FloatingPanelMetrics.stripHeight)
                .help("Drag to move")
                .accessibilityLabel(Text("Drag to move"))
            ChromeCircleButton(symbol: "xmark", help: "Dismiss the usage panel; the usage popover opens it again") {
                dismiss()
            }
            .padding(.top, Chrome.chromeTopPadding)
            .padding(.trailing, Chrome.chromeHorizontalPadding)
        }
    }
}
