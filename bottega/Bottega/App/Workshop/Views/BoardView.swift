// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import BottegaKit
import SwiftUI

struct BoardView: View {
    @Environment(WorkshopModel.self) private var model
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Namespace private var space

    var body: some View {
        if model.state.cardOrder.isEmpty {
            ContentUnavailableView(
                "No Cards", systemImage: Page.board.symbol, description: Text("Cards the dispatcher hands out show up here."))
        } else {
            GeometryReader { geometry in
                ScrollView(.horizontal) {
                    HStack(alignment: .top, spacing: 14) {
                        ForEach(WorkshopState.Column.allCases, id: \.self) { column in
                            self.column(column).frame(width: width(in: geometry.size.width), alignment: .topLeading)
                        }
                    }
                    .padding(20)
                    .animation(.spring(response: 0.35, dampingFraction: 1), value: placement)
                }
            }
            .listMovement(ids: WorkshopState.Column.allCases.flatMap { model.state.cards(in: $0).map(\.id) }, selection: selection)
        }
    }

    private func column(_ column: WorkshopState.Column) -> some View {
        let cards = model.state.cards(in: column)
        return VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 6) {
                Image(systemName: Wording.columnSymbol(column))
                Text(Wording.column(column)).fontWeight(.medium)
                Text(cards.count, format: .number).foregroundStyle(.secondary).monospacedDigit()
            }
            .font(.callout)
            .foregroundStyle(column == .needsYou && !cards.isEmpty ? AnyShapeStyle(.red) : AnyShapeStyle(.primary))
            ScrollView {
                LazyVStack(spacing: 10) {
                    ForEach(cards) { card in
                        CardTile(card: card)
                            .modifier(MovesBetweenColumns(id: card.id, space: space, moves: !reduceMotion))
                    }
                }
                .padding(2)
            }
        }
    }

    /// All five side by side when they fit.
    private func width(in total: CGFloat) -> CGFloat {
        let count = CGFloat(WorkshopState.Column.allCases.count)
        return min(max((total - 40 - 14 * (count - 1)) / count, 190), 320)
    }

    /// Which card is where; the board animates only when that changes.
    private var placement: [String: WorkshopState.Column] {
        model.state.cards.mapValues(model.state.column(of:))
    }

    private var selection: Binding<String?> {
        Binding {
            if case .card(let id) = model.detail { id } else { nil }
        } set: { id in
            if let id { model.detail = .card(id) }
        }
    }
}

private struct MovesBetweenColumns: ViewModifier {
    let id: String
    let space: Namespace.ID
    let moves: Bool

    func body(content: Content) -> some View {
        if moves { content.matchedGeometryEffect(id: id, in: space) } else { content }
    }
}
