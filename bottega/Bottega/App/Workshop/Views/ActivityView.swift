// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import BottegaKit
import SwiftUI

/// Everything that happened, one line each, in order.
struct ActivityView: View {
    @Environment(WorkshopModel.self) private var model

    var body: some View {
        if model.state.log.isEmpty {
            ContentUnavailableView("No Events Yet", systemImage: Page.activity.symbol)
        } else {
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 0) {
                    ForEach(model.state.log) { happening in
                        HStack(alignment: .firstTextBaseline, spacing: 10) {
                            Text(verbatim: "\(happening.id.seq)")
                                .font(.caption.monospaced())
                                .foregroundStyle(.tertiary)
                                .frame(width: 44, alignment: .trailing)
                            NoteRow(summary: Summary(happening.event, in: model.state), at: happening.at)
                        }
                        .padding(.vertical, 5)
                        Divider()
                    }
                }
                .padding(.horizontal, 20)
                .padding(.vertical, 12)
            }
            .defaultScrollAnchor(.bottom)
        }
    }
}
