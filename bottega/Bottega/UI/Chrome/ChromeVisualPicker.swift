// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import SwiftUI

// A row control of two or three tiles, each a single symbol standing for what the option
// does, the way macOS Appearance offers Light / Dark / Auto; the chosen tile wears an accent
// ring and takes the accent on its symbol. The symbol is the caller's: every picker hands in
// one `Image`, and the option's name sits under its tile so the symbol never has to be guessed.

/// The tile's measures: every tile is the same size, so a picker of two options and a picker
/// of three line up with each other and with the rows around them.
enum ChromeVisualTileMetrics {
    static let height: CGFloat = 48
    static let regularWidth: CGFloat = 76
    static let denseWidth: CGFloat = 76
    static let cornerRadius: CGFloat = 6
    static let ringInset: CGFloat = 3
    static let ringWidth: CGFloat = 2
    static let spacing: CGFloat = 12
    static let captionSpacing: CGFloat = 6
}

/// The row control: one tile per option, the chosen one ringed.
struct ChromeVisualPicker<Value: Hashable, Preview: View>: View {
    let options: [(value: Value, title: String)]
    @Binding var selection: Value
    @ViewBuilder var preview: (Value) -> Preview

    init(options: [(value: Value, title: String)], selection: Binding<Value>, @ViewBuilder preview: @escaping (Value) -> Preview) {
        self.options = options
        self._selection = selection
        self.preview = preview
    }

    private var tileWidth: CGFloat {
        options.count >= 3 ? ChromeVisualTileMetrics.denseWidth : ChromeVisualTileMetrics.regularWidth
    }

    var body: some View {
        HStack(alignment: .top, spacing: ChromeVisualTileMetrics.spacing) {
            ForEach(Array(options.enumerated()), id: \.offset) { _, option in
                ChromeVisualTile(title: option.title, isSelected: option.value == selection, width: tileWidth) {
                    withAnimation(Chrome.hover) { selection = option.value }
                } content: {
                    preview(option.value)
                }
            }
        }
        .fixedSize()
        .background { NoWindowDragArea() }
    }
}

/// One tile: a single symbol on a stage, accent on the symbol the chosen option carries.
struct ChromeVisualTile<Content: View>: View {
    let title: String
    let isSelected: Bool
    var width: CGFloat = ChromeVisualTileMetrics.regularWidth
    let action: () -> Void
    @ViewBuilder var content: () -> Content
    @State private var isHovering = false

    var body: some View {
        Button(action: action) {
            VStack(spacing: ChromeVisualTileMetrics.captionSpacing) {
                stage
                caption
            }
        }
        .buttonStyle(.plain)
        .onHover { hovering in
            withAnimation(Chrome.hover) { isHovering = hovering }
        }
        .accessibilityLabel(Text(verbatim: title))
        .accessibilityAddTraits(isSelected ? .isSelected : [])
    }

    /// The option's name under its tile: the symbol alone does not say what the choice is.
    private var caption: some View {
        Text(verbatim: title)
            .zoomedFont(size: 11)
            .foregroundStyle(isSelected ? Chrome.primaryText : Chrome.secondaryText)
            .multilineTextAlignment(.center)
            .lineLimit(2)
            .frame(width: width)
    }

    private var stage: some View {
        content()
            .font(.system(size: 17, weight: .medium))
            .foregroundStyle(isSelected ? Chrome.accent : Chrome.secondaryText)
            .frame(width: width, height: ChromeVisualTileMetrics.height)
            .background(
                RoundedRectangle(cornerRadius: ChromeVisualTileMetrics.cornerRadius, style: .continuous)
                    .fill(Chrome.overlay(isHovering || isSelected ? 0.10 : 0.06))
            )
            .clipShape(RoundedRectangle(cornerRadius: ChromeVisualTileMetrics.cornerRadius, style: .continuous))
            .padding(ChromeVisualTileMetrics.ringInset)
            .overlay(
                RoundedRectangle(
                    cornerRadius: ChromeVisualTileMetrics.cornerRadius + ChromeVisualTileMetrics.ringInset,
                    style: .continuous
                )
                .strokeBorder(isSelected ? Chrome.accent : Color.clear, lineWidth: ChromeVisualTileMetrics.ringWidth)
            )
            .contentShape(Rectangle())
    }
}
