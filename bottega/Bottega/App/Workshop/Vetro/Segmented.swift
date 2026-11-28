// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import SwiftUI

/// One row of choices in a capsule; a pill of glass slides to the one picked. Every switcher in the window is this
/// one (the sidebar's sections, the panel's pages, Timeline / Lanes, Board / List), so they are one family with the
/// composer and the other capsules. The pill is glass white, never the accent colour: blue here would read as a
/// button to press, and red and amber already mean something.
struct VetroSegmented<Value: Hashable>: View {
    struct Item {
        let value: Value
        let title: String
        /// Shown instead of the title, which VoiceOver still reads.
        var symbol: String?
        /// The tooltip; the title when there is none.
        var help: String?
        /// How many wait on the user there, in red as everywhere.
        var waiting = 0
    }

    /// What VoiceOver calls the whole row.
    let label: String
    let items: [Item]
    let selected: Value
    /// All of it, outline included.
    var height: CGFloat = 28
    /// Segments share the row's width, rather than each being as wide as what it says.
    var fillsWidth = false
    var fontSize: CGFloat = 12
    let select: (Value) -> Void

    @Environment(Look.self) private var look
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Namespace private var space

    var body: some View {
        HStack(spacing: 0) {
            ForEach(items, id: \.value) { segment($0) }
        }
        .padding(VetroSegmentedMetrics.inset)
        .frame(height: height)
        .background(look.surface, in: .capsule)
        .vetroEdge(in: Capsule())
        // The pill slides however the choice was made: a click here, a menu command, ⌘K.
        .animation(reduceMotion ? nil : .smooth(duration: 0.25), value: selected)
        .accessibilityElement(children: .contain)
        .accessibilityLabel(label)
    }

    private func segment(_ item: Item) -> some View {
        let isOn = item.value == selected
        return Button { select(item.value) } label: {
            Group {
                if let symbol = item.symbol {
                    Image(systemName: symbol).font(.system(size: fontSize + 1, weight: .medium))
                } else {
                    // One weight for both states: a bolder picked word would be wider and push its neighbours.
                    // And never cut short: a row that shares its width gives a long word more of it.
                    Text(item.title).font(.system(size: fontSize, weight: .medium)).lineLimit(1).fixedSize()
                }
            }
            .foregroundStyle(isOn ? .primary : .secondary)
            .padding(.horizontal, item.symbol != nil ? 8 : fillsWidth ? 5 : 12)
            .frame(maxWidth: fillsWidth ? .infinity : nil, maxHeight: .infinity)
            .background { if isOn { pill } }
            .overlay(alignment: .top) { WaitingBadge(count: item.waiting).offset(x: 10, y: -3) }
            .contentShape(.capsule)
        }
        .buttonStyle(.plain)
        .help(item.help ?? item.title)
        .accessibilityLabel(item.title)
        .accessibilityAddTraits(isOn ? .isSelected : [])
    }

    /// With less motion it is simply at the picked one, not on its way there.
    @ViewBuilder private var pill: some View {
        let glass = Capsule()
            .fill(look.selection)
            .vetroEdge(in: Capsule())
            .shadow(color: .black.opacity(0.14), radius: 1.5, y: 1)
        if reduceMotion { glass } else { glass.matchedGeometryEffect(id: "pill", in: space) }
    }
}

enum VetroSegmentedMetrics {
    /// Between the capsule's outline and the pill.
    static let inset: CGFloat = 2
}
