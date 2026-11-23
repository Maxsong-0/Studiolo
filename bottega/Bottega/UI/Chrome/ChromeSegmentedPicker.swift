// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import SwiftUI

// A row control that shows every option as a small button in one glass capsule strip,
// the chosen one filled. For rows whose options are a few short words or numbers and
// need no picture; an option may draw an SF Symbol in place of its word.

/// One option of the strip: its title, read out and shown unless `symbol` draws in its place.
struct ChromeSegmentedOption<Value: Hashable> {
    let value: Value
    let title: String
    /// An SF Symbol shown instead of the title; the title stays the accessibility label.
    var symbol: String? = nil
}

/// The row control: one small button per option, the chosen one filled.
struct ChromeSegmentedPicker<Value: Hashable>: View {
    let options: [ChromeSegmentedOption<Value>]
    @Binding var selection: Value

    init(options: [ChromeSegmentedOption<Value>], selection: Binding<Value>) {
        self.options = options
        self._selection = selection
    }

    init(options: [(value: Value, title: String)], selection: Binding<Value>) {
        self.init(options: options.map { ChromeSegmentedOption(value: $0.value, title: $0.title) }, selection: selection)
    }

    var body: some View {
        HStack(spacing: 2) {
            ForEach(Array(options.enumerated()), id: \.offset) { _, option in
                ChromeSegment(title: option.title, symbol: option.symbol, isSelected: option.value == selection) {
                    withAnimation(Chrome.hover) { selection = option.value }
                }
            }
        }
        .padding(.horizontal, 3)
        .frame(height: Chrome.capsuleContentHeight)
        .chromeGlassCapsule()
        .fixedSize()
        .accessibilityElement(children: .contain)
        .background { NoWindowDragArea() }
    }
}

/// One segment: a small capsule button, filled while chosen.
private struct ChromeSegment: View {
    let title: String
    let symbol: String?
    let isSelected: Bool
    let select: () -> Void
    @State private var isHovering = false

    var body: some View {
        Button(action: select) {
            label
                .font(.system(size: 12, weight: isSelected ? .semibold : .medium))
                .foregroundStyle(isSelected ? Chrome.primaryText : Chrome.secondaryText)
                .lineLimit(1)
                .padding(.horizontal, 9)
                .frame(minWidth: 26)
                .frame(height: 22)
                .background(Capsule(style: .continuous).fill(isSelected ? Chrome.accent.opacity(0.22) : Chrome.overlay(isHovering ? 0.08 : 0)))
                .contentShape(Capsule(style: .continuous))
        }
        .buttonStyle(.plain)
        .onHover { hovering in
            withAnimation(Chrome.hover) { isHovering = hovering }
        }
        .accessibilityAddTraits(isSelected ? .isSelected : [])
        .accessibilityLabel(Text(verbatim: title))
    }

    @ViewBuilder
    private var label: some View {
        if let symbol {
            Image(systemName: symbol)
        } else {
            Text(verbatim: title)
        }
    }
}
