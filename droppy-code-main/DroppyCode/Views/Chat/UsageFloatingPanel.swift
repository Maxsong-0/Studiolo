import AppKit
import SwiftUI

/// The chat's usage as a floating glass panel, the helper panel's size and recipe: the
/// context window, the plan's limits and the credits of the model in use, or of the lead
/// and the heads when a pair sends them out on another provider. It sits across the
/// column from the heads, at the left by default. The strip along the top is the handle,
/// with the title at its left and the close button at its right.
struct UsageFloatingPanel: View {
    @Environment(WindowLiveResize.self) private var liveResize
    @Environment(\.colorScheme) private var colorScheme
    let runtime: ThreadRuntime
    let provider: ProviderKind
    let headsProvider: ProviderKind?
    let size: CGSize
    /// The height the content wants, strip and paddings included, reported whenever it
    /// changes; the placer sizes the panel to it, up to `size.height`.
    let onContentHeight: (CGFloat) -> Void
    /// The pointer's travel since the handle was grabbed.
    let onDrag: (CGSize) -> Void
    let onDragEnd: () -> Void
    let dismiss: () -> Void

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: HydraPanel.cornerRadius, style: .continuous)
        let usage = runtime.usage
        let hasSomething = usage?.fraction != nil
            || PlanLimitsReader.exposesLimits(provider) || CreditsReader.exposesCredits(provider)
            || headsProvider.map { PlanLimitsReader.exposesLimits($0) || CreditsReader.exposesCredits($0) } == true
        Group {
            if hasSomething {
                ScrollView(.vertical) {
                    UsagePanel(usage: usage, provider: provider, headsProvider: headsProvider, fillsWidth: true)
                        .onGeometryChange(for: CGFloat.self, of: { $0.size.height }) { onContentHeight($0 + HydraPanel.stripHeight + 8) }
                        .padding(.top, HydraPanel.stripHeight)
                        .padding(.bottom, 8)
                }
            } else {
                Text(verbatim: "Nothing to show for \(provider.displayName) yet.")
                    .font(.system(size: 13))
                    .foregroundStyle(Chrome.secondaryText)
                    .padding(.top, HydraPanel.stripHeight)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .onAppear { onContentHeight(HydraPanel.stripHeight + 44) }
            }
        }
        .overlay(alignment: .top) {
            strip
        }
        .frame(width: size.width, height: size.height)
        // The frame follows the pane every frame of a live resize.
        .animation(nil, value: liveResize.isActive)
        .background {
            // The helper panel's recipe: one glass surface, a scrim for the text over
            // whatever the panel floats above, and the theme's tint.
            let isDark = colorScheme == .dark
            shape
                .fill(.clear)
                .glassEffect(.regular, in: shape)
                .overlay {
                    shape.fill(Chrome.glassTint.opacity(isDark ? 0.22 : 0.16))
                }
        }
        // Glass supplies the edge (as in `WindowBackdrop`): a hairline on top of it
        // read as a second ring around the panel whenever its window was key.
        .clipShape(shape)
        // The scrim sits under the glass and carries the panel's shadow, drawn once.
        .background {
            let isDark = colorScheme == .dark
            shape
                .fill((isDark ? Color.black : Color.white).opacity(isDark ? 0.3 : 0.34))
                .shadow(color: .black.opacity(isDark ? 1 : 0.65), radius: 28, y: 10)
        }
    }

    /// The handle across the top: the title at the left, the close button at the right,
    /// and the room between them drags the panel.
    private var strip: some View {
        HStack(alignment: .top, spacing: 8) {
            HStack(spacing: 6) {
                ProviderIcon(provider: provider, size: 12)
                if let headsProvider {
                    ProviderIcon(provider: headsProvider, size: 12)
                }
                Text("Usage")
                    .font(.system(size: 12.5, weight: .medium))
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
                .frame(height: HydraPanel.stripHeight)
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
