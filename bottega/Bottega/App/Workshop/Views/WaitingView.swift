// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import BottegaKit
import SwiftUI

/// What waits for the user, the longest wait first.
struct WaitingView: View {
    @Environment(WorkshopModel.self) private var model
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        let items = model.waiting
        if items.isEmpty {
            ContentUnavailableView(
                "Nothing Is Waiting", systemImage: "checkmark.circle",
                description: Text("Approvals, questions and letters held back by a limit show up here."))
        } else {
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 16) {
                    ForEach(items, id: \.what) { item in
                        WaitingItem(item: item)
                            .transition(.asymmetric(insertion: .opacity, removal: reduceMotion ? .opacity : .opacity.combined(with: .offset(y: -8))))
                    }
                }
                .frame(maxWidth: 720)
                .frame(maxWidth: .infinity)
                .padding([.horizontal, .bottom], 24)
                .padding(.top, FloatingTopRoom.reserved)
                .animation(.easeOut(duration: 0.16), value: items)
            }
        }
    }
}

private struct WaitingItem: View {
    @Environment(WorkshopModel.self) private var model
    let item: Waiting

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(spacing: 8) {
                Image(systemName: "hand.raised.fill").foregroundStyle(.waiting)
                Text(verbatim: title).font(.headline)
                Spacer()
                TimelineView(.everyMinute) { context in
                    Text(verbatim: Wording.waited(since: item.at, now: context.date))
                        .font(.callout)
                        .foregroundStyle(.secondary)
                        .monospacedDigit()
                }
                .help(Wording.exactTime(item.at))
            }
            content
        }
        .padding(16)
        .vetroSurface(cornerRadius: 14)
    }

    private var title: String { Wording.waiting(item, in: model.state) }

    @ViewBuilder private var content: some View {
        switch item.what {
        case .ask(let id):
            if let ask = model.state.asks[id] { AskPanel(ask: ask) }
        case .question(let id):
            if let message = model.state.messages[id] {
                Text(verbatim: UntrustedText.clean(message.body)).textSelection(.enabled).lineSpacing(2)
                    .onAppear { model.letter(id, isOnScreen: true) }
                    .onDisappear { model.letter(id, isOnScreen: false) }
                ForEach(message.files, id: \.self) { FileChip(file: $0) }
                DraftField(key: .question(id), placeholder: "Answer…")
                HStack {
                    Button("Open Line") { model.page = .line(message.thread) }.controlSize(.small)
                    Spacer()
                    SendDraftButton(key: .question(id), title: "Answer")
                }
            }
        case .trip(let id):
            if let trip = model.state.trips[id] { TripFacts(trip: trip) }
        }
    }
}
