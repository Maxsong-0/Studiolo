// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import BottegaKit
import SwiftUI

/// The cards as a list (Bottega-5 §7, ADR-047 items 17 and 18, canvas version 10 `CardList`): one row a card, the one
/// something last happened to on top. Which card is on top and what its tests-and-merge cell says are the core
/// package's (`cardsByRecentActivity`, `progress(of:)`); this only lays them out.
struct CardListView: View {
    @Environment(WorkshopModel.self) private var model
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    /// Rows rise in one after another the first time the list shows.
    @State private var hasRisen = false

    var body: some View {
        let cards = model.state.cardsByRecentActivity
        if cards.isEmpty {
            ContentUnavailableView(
                "No Cards", systemImage: Page.board.symbol, description: Text("Cards the dispatcher hands out show up here."))
        } else {
            // Often enough for "3m" to turn into "4m" without a redraw for nothing.
            TimelineView(.periodic(from: .now, by: 30)) { timeline in
                ScrollView {
                    VStack(spacing: 0) {
                        ForEach(Array(cards.enumerated()), id: \.element.id) { index, card in
                            CardListRow(card: card, project: project, now: timeline.date, isFirst: index == 0)
                                .modifier(RisesIn(delay: CardsMotion.listDelay(row: index), hasRisen: hasRisen, reduceMotion: reduceMotion))
                        }
                    }
                    .padding(.vertical, 2)
                    .vetroSurface(cornerRadius: 12)
                    .padding(EdgeInsets(top: 14, leading: 20, bottom: 18, trailing: 20))
                }
            }
            .listMovement(ids: cards.map(\.id), selection: selection)
            .onAppear { hasRisen = true }
            .accessibilityElement(children: .contain)
            .accessibilityLabel("Cards, most recent first")
        }
    }

    /// LMAS names one repository for all its cards for now (Bottega-8 A2): every row gets the same tag.
    private var project: String? { model.overview?.cards?.projectLabel }

    private var selection: Binding<String?> {
        Binding {
            if case .card(let id) = model.detail { id } else { nil }
        } set: { id in
            if let id { model.detail = .card(id) }
        }
    }
}

/// One card: stage, number, title and project, tests and merge, who has it, lines changed, how long ago.
private struct CardListRow: View {
    @Environment(WorkshopModel.self) private var model
    @Environment(Look.self) private var look
    let card: Card
    let project: String?
    let now: Date
    let isFirst: Bool
    @State private var isHovered = false

    var body: some View {
        let column = model.state.column(of: card)
        let progress = model.state.progress(of: card)
        let isSelected = model.detail == .card(card.id)
        HStack(spacing: 12) {
            Stage(column: column).frame(width: 12)
            Text(verbatim: UntrustedText.line(card.id, limit: 24))
                .font(.system(size: 11).monospaced())
                .foregroundStyle(.secondary)
                .lineLimit(1)
                .truncationMode(.middle)
                .frame(width: 58, alignment: .leading)
            HStack(spacing: 8) {
                Text(verbatim: UntrustedText.line(card.title ?? "Untitled", limit: 160)).lineLimit(1)
                if let project {
                    Text(verbatim: project)
                        .font(.system(size: 10.5))
                        .foregroundStyle(.secondary)
                        .padding(.horizontal, 7)
                        .frame(height: 18)
                        .background(look.selection, in: .capsule)
                        .fixedSize()
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            ProgressCell(progress: progress).frame(width: 172, alignment: .leading)
            who.frame(width: 18)
            Text(verbatim: size)
                .font(.system(size: 11).monospaced())
                .foregroundStyle(.secondary)
                .frame(width: 74, alignment: .trailing)
            Text(verbatim: age)
                .font(.system(size: 11).monospaced())
                .foregroundStyle(.secondary)
                .frame(width: 34, alignment: .trailing)
        }
        .font(.system(size: 12.5))
        .padding(.horizontal, 14)
        .frame(height: 40)
        .background { if !isSelected, isHovered { look.selection.opacity(0.5) } }
        .vetroPicked(isSelected, in: Rectangle())
        .overlay(alignment: .top) { if !isFirst { Divider().opacity(0.6) } }
        .contentShape(.rect)
        .onHover { isHovered = $0 }
        .onTapGesture { model.detail = .card(card.id) }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(Text(verbatim: label(column, progress)))
        .accessibilityAddTraits(isSelected ? [.isButton, .isSelected] : .isButton)
        .accessibilityAction { model.detail = .card(card.id) }
    }

    /// The agent on it, once one has taken it up; before that, the vendor it is for.
    @ViewBuilder private var who: some View {
        if let worker = card.work?.worker {
            MemberGlyph(name: worker, size: 18)
        } else if let maker = card.vendor.flatMap(Maker.init(vendor:)) {
            MakerTile(maker: maker, size: 18).help("For \(maker.title)")
        }
    }

    private var size: String {
        model.state.size(of: card).map { "+\($0.insertions) −\($0.deletions)" } ?? ""
    }

    private var age: String { model.state.age(of: card, now: UInt64(max(0, now.timeIntervalSince1970 * 1000))) }

    private func label(_ column: WorkshopState.Column, _ progress: CardProgress) -> String {
        [UntrustedText.line(card.id, limit: 24), UntrustedText.line(card.title ?? "Untitled", limit: 160),
         Wording.column(column), Wording.progress(progress), size]
            .filter { !$0.isEmpty }.joined(separator: ", ")
    }
}

/// Where a card stands, as a mark: grey ring queued, amber dots working, red dot needs you, tick done, cross failed.
private struct Stage: View {
    let column: WorkshopState.Column

    var body: some View {
        switch column {
        case .queued: Image(systemName: "circle").font(.system(size: 9)).foregroundStyle(.secondary)
        case .working: WorkingMatrix(side: 3)
        case .needsYou: Circle().fill(.waiting).frame(width: 7, height: 7)
        case .done: Image(systemName: "checkmark").font(.system(size: 9, weight: .bold)).foregroundStyle(.secondary)
        case .failed: Image(systemName: "xmark").font(.system(size: 9, weight: .bold)).foregroundStyle(.trouble)
        }
    }
}

/// Tests and merge: a grey spinner while the testbox runs (a program, not an AI thinking), a cross when tests failed,
/// a red dot when the merge waits for the user; the words alone otherwise, with the mark's room kept so they line up.
private struct ProgressCell: View {
    let progress: CardProgress

    var body: some View {
        let text = Wording.progress(progress)
        HStack(spacing: 7) {
            mark.frame(width: 10, height: 10)
            Text(verbatim: text).lineLimit(1).truncationMode(.tail)
        }
        .font(.system(size: 11.5, weight: progress.mark == .waitsForYou ? .semibold : .regular))
        .foregroundStyle(progress.mark == .waitsForYou ? AnyShapeStyle(.waiting) : AnyShapeStyle(.secondary))
        .help(Text(verbatim: text))
    }

    @ViewBuilder private var mark: some View {
        switch progress.mark {
        case .none: Color.clear
        case .spinner: ProgressView().controlSize(.mini).scaleEffect(0.8)
        case .cross: Image(systemName: "xmark").font(.system(size: 9, weight: .bold))
        case .waitsForYou: Circle().fill(.waiting).frame(width: 8, height: 8)
        }
    }
}

