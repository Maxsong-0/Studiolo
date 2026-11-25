// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import BottegaKit
import SwiftUI

/// A card in a timeline or on the board: what it is, who has it, where it stands.
struct CardTile: View {
    @Environment(WorkshopModel.self) private var model
    let card: Card

    var body: some View {
        let column = model.state.column(of: card)
        let isSelected = model.detail == .card(card.id)
        VStack(alignment: .leading, spacing: 8) {
            HStack(alignment: .firstTextBaseline) {
                Text(verbatim: UntrustedText.line(card.id, limit: 24)).font(.caption.monospaced()).foregroundStyle(.secondary)
                Spacer(minLength: 6)
                if column == .working {
                    WorkingLabel()
                } else {
                    StatusChip(text: Wording.column(column), symbol: Wording.columnSymbol(column), waits: column == .needsYou)
                }
            }
            Text(verbatim: UntrustedText.line(card.title ?? "Untitled", limit: 160)).fontWeight(.medium)
            if let who = card.work?.worker ?? card.vendor {
                Group {
                    if card.work != nil {
                        MemberName(name: who)
                    } else {
                        Text(verbatim: "for \(UntrustedText.line(who, limit: 24))")
                    }
                }
                .font(.caption)
                .foregroundStyle(.secondary)
            }
            if column == .working, let worker = card.work?.worker, let turn = model.state.runningTurn(of: worker) {
                DoingNow(turn: turn.id)
            }
            if let branch = card.work?.branch {
                Label {
                    Text(verbatim: UntrustedText.line(branch, limit: 80)).monospaced().lineLimit(1).truncationMode(.middle)
                } icon: {
                    Image(systemName: "arrow.triangle.branch")
                }
                .font(.caption)
                .foregroundStyle(.secondary)
            }
            if let size = lastSize {
                Label {
                    Text(verbatim: "\(Wording.count(size.files, "file", "files")), +\(size.insertions) −\(size.deletions)").monospacedDigit()
                } icon: {
                    Image(systemName: "plus.forwardslash.minus")
                }
                .font(.caption)
                .foregroundStyle(.secondary)
            }
            if let continues = card.continues {
                Label {
                    Text(verbatim: "Reworks \(UntrustedText.line(continues, limit: 24))")
                } icon: {
                    Image(systemName: "arrow.uturn.backward")
                }
                .font(.caption)
                .foregroundStyle(.secondary)
            }
            if case .failed(let why, _) = card.end {
                Text(verbatim: UntrustedText.line(why.text)).font(.caption).foregroundStyle(.secondary).lineLimit(3)
            }
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(.background.secondary, in: .rect(cornerRadius: 10))
        .overlay {
            RoundedRectangle(cornerRadius: 10)
                .strokeBorder(isSelected ? AnyShapeStyle(Color.accentColor) : AnyShapeStyle(.separator), lineWidth: isSelected ? 1.5 : 1)
        }
        .contentShape(.rect)
        .onTapGesture { model.detail = .card(card.id) }
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(.isButton)
        .accessibilityAction { model.detail = .card(card.id) }
    }

    /// Files changed is known once a merge has been checked.
    private var lastSize: Merge.Size? {
        card.merges.lazy.reversed().compactMap { model.state.merges[$0]?.size }.first
    }
}
