import AppKit
import SwiftUI

/// The thread's changes as a small tab rising from the top of the chat box. The composer draws
/// over its lower edge, so it reads as part of the box. Clicking it opens the changes popover.
struct ThreadChangesTab: View {
    static let overlap: CGFloat = 14

    let stats: FileChangeSummary
    /// Receives the tab's own view, so the popover can anchor to it.
    let anchor: (NSView) -> Void
    let action: () -> Void

    var body: some View {
        let shape = UnevenRoundedRectangle(topLeadingRadius: 12, topTrailingRadius: 12, style: .continuous)
        Button(action: action) {
            HStack(spacing: 7) {
                Image(systemName: "plusminus")
                    .font(Chrome.inlineIconFont)
                    .foregroundStyle(Chrome.secondaryText)
                Text(verbatim: stats.files.count == 1 ? "1 file" : "\(stats.files.count) files")
                    .foregroundStyle(Chrome.primaryText.opacity(0.9))
                // The same hues as every other +/− in the app (see DiffStatLabel), so a
                // theme recolours the tab with the diffs it stands for.
                Text(verbatim: "+\(stats.additions)")
                    .foregroundStyle(Chrome.success)
                Text(verbatim: "\u{2212}\(stats.deletions)")
                    .foregroundStyle(Chrome.danger)
            }
            .font(.system(size: 12, weight: .medium).monospacedDigit())
            .padding(.horizontal, 12)
            .padding(.top, 7)
            .padding(.bottom, 7 + Self.overlap)
            .contentShape(shape)
        }
        .buttonStyle(.plain)
        .focusable(false)
        .glassEffect(.regular.interactive(), in: shape)
        .background {
            AttachmentAnchorCapture(onResolve: anchor)
        }
        .fixedSize()
        .help("Show this thread's changes\(ShortcutStore.hint(for: .toggleChanges))")
        .accessibilityLabel(Text(verbatim: "\(stats.files.count) files changed, \(stats.additions) added, \(stats.deletions) removed"))
    }
}
