// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import BottegaKit
import SwiftUI

/// The dispatcher's page: a timeline and a composer.
struct TimelinePage<Empty: View>: View {
    @Environment(WorkshopModel.self) private var model
    let rows: [Row]
    let key: DraftKey
    let empty: Empty

    var body: some View {
        LineTimeline(rows: rows, empty: empty)
            .safeAreaInset(edge: .bottom, spacing: 0) { WorkshopComposer(key: key) }
    }
}

/// A line's page, as its timeline or as letters between agents.
struct LinePage: View {
    @Environment(WorkshopModel.self) private var model
    let line: BottegaKit.Line

    var body: some View {
        Group {
            if model.showsLanes {
                LanesView(line: line)
            } else {
                LineTimeline(rows: line.rows, empty: EmptyView())
            }
        }
        .safeAreaInset(edge: .bottom, spacing: 0) { WorkshopComposer(key: .line(line.id)) }
    }
}

/// Rows oldest first, resting at the bottom. Scrolled up, it stays where it is and says how many have come in.
struct LineTimeline<Empty: View>: View {
    @Environment(WorkshopModel.self) private var model
    let rows: [Row]
    let empty: Empty

    @State private var position = ScrollPosition(edge: .bottom)
    @State private var isAtBottom = true
    /// How many rows there were when the user scrolled up.
    @State private var countWhenLeft = 0

    var body: some View {
        if rows.isEmpty {
            empty.frame(maxWidth: .infinity, maxHeight: .infinity)
        } else {
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 14) {
                    ForEach(rows) { row in
                        TimelineRow(row: row)
                            .transition(.newRow)
                    }
                }
                .frame(maxWidth: 780, alignment: .leading)
                .frame(maxWidth: .infinity)
                .padding(.horizontal, 24)
                .padding(.top, FloatingTopRoom.reserved)
                .padding(.bottom, 20)
                .animation(.smooth(duration: 0.2), value: rows.last?.id)
            }
            .scrollPosition($position)
            .defaultScrollAnchor(.bottom, for: .initialOffset)
            .onScrollGeometryChange(for: Bool.self) { geometry in
                geometry.contentSize.height - geometry.visibleRect.maxY + geometry.contentInsets.bottom < 24
            } action: { _, atBottom in
                if isAtBottom, !atBottom { countWhenLeft = rows.count }
                isAtBottom = atBottom
            }
            .onChange(of: rows.count) {
                if isAtBottom { position.scrollTo(edge: .bottom) }
            }
            .overlay(alignment: .bottom) { newRows }
            .listMovement(ids: letters, selection: selectedLetter)
        }
    }

    @ViewBuilder private var newRows: some View {
        let count = rows.count - countWhenLeft
        if !isAtBottom, count > 0 {
            Button {
                position.scrollTo(edge: .bottom)
            } label: {
                Label(Wording.count(count, "new message", "new messages"), systemImage: "arrow.down")
                    .font(.callout)
            }
            .buttonStyle(.glass)
            .padding(.bottom, 10)
        }
    }

    private var letters: [String] {
        rows.compactMap { if case .message(let id) = $0.kind { id } else { nil } }
    }

    private var selectedLetter: Binding<String?> {
        Binding {
            if case .message(let id) = model.detail { id } else { nil }
        } set: { id in
            if let id { model.detail = .message(id) }
        }
    }
}

extension AnyTransition {
    /// A new message: fades in while rising 4 points; with less motion, only fades.
    @MainActor static var newRow: AnyTransition {
        NSWorkspace.shared.accessibilityDisplayShouldReduceMotion
            ? .opacity : .asymmetric(insertion: .opacity.combined(with: .offset(y: 4)), removal: .opacity)
    }
}

extension View {
    /// J and K move through a list. Straight away, no animation: it happens hundreds of times a day.
    func listMovement<ID: Equatable>(ids: [ID], selection: Binding<ID?>) -> some View {
        modifier(ListMovement(ids: ids, selection: selection))
    }
}

private struct ListMovement<ID: Equatable>: ViewModifier {
    let ids: [ID]
    @Binding var selection: ID?
    @FocusState private var isFocused: Bool

    func body(content: Content) -> some View {
        content
            .focusable()
            .focused($isFocused)
            .focusEffectDisabled()
            // Picking something with the mouse makes this list the one the keys move in.
            .onChange(of: selection) { if selection != nil { isFocused = true } }
            .onKeyPress("j") { move(by: 1) }
            .onKeyPress("k") { move(by: -1) }
    }

    private func move(by step: Int) -> KeyPress.Result {
        guard !ids.isEmpty else { return .ignored }
        let next = selection.flatMap(ids.firstIndex).map { $0 + step } ?? (step > 0 ? 0 : ids.count - 1)
        if ids.indices.contains(next) { selection = ids[next] }
        return .handled
    }
}

/// One row of a timeline, by what it shows.
struct TimelineRow: View {
    @Environment(WorkshopModel.self) private var model
    let row: Row

    var body: some View {
        switch row.kind {
        case .message(let id):
            if let message = model.state.messages[id] { LetterRow(message: message) } else { note }
        case .card(let id):
            if let card = model.state.cards[id] { CardTile(card: card) } else { note }
        case .test(let id):
            if let test = model.state.tests[id] { TestRow(test: test) } else { note }
        case .review(let id, let index):
            if let reviews = model.state.cards[id]?.reviews, reviews.indices.contains(index) {
                ReviewRow(card: id, review: reviews[index])
            } else {
                note
            }
        case .merge(let id):
            if let merge = model.state.merges[id] { MergeRow(merge: merge) } else { note }
        case .ask(let id):
            if let ask = model.state.asks[id] { AskRow(ask: ask) } else { note }
        case .trip(let id):
            if let trip = model.state.trips[id] { TripRow(trip: trip) } else { note }
        case .cardStarted, .cardEnded, .unreadable:
            note
        }
    }

    private var note: some View { NoteRow(summary: Summary(row, in: model.state), at: row.at) }
}

/// A row that is one line of plain words.
struct NoteRow: View {
    @Environment(WorkshopModel.self) private var model
    let summary: Summary
    let at: UInt64

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            Image(systemName: summary.symbol)
                .frame(width: 18)
                .foregroundStyle(summary.isFailure ? AnyShapeStyle(.orange) : AnyShapeStyle(.secondary))
            Text(verbatim: summary.text)
                .frame(maxWidth: .infinity, alignment: .leading)
            Text(verbatim: Wording.time(at)).foregroundStyle(.tertiary).monospacedDigit()
        }
        .font(.callout)
        .foregroundStyle(.secondary)
        .textSelection(.enabled)
        .contentShape(.rect)
        .onTapGesture { if let detail = summary.detail { model.detail = detail } }
    }
}

/// A letter. From the user on the right, in the accent; everyone else's on the left, plain.
struct LetterRow: View {
    @Environment(WorkshopModel.self) private var model
    let message: Message

    var body: some View {
        let isSelected = model.detail == .message(message.id)
        VStack(alignment: message.isFromUser ? .trailing : .leading, spacing: 5) {
            header
            VStack(alignment: .leading, spacing: 8) {
                Text(verbatim: UntrustedText.clean(message.body))
                    .textSelection(.enabled)
                    .lineSpacing(2)
                if !message.files.isEmpty {
                    ForEach(message.files, id: \.self) { FileChip(file: $0) }
                }
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 9)
            .background(background, in: .rect(cornerRadius: 12))
            .overlay {
                RoundedRectangle(cornerRadius: 12).strokeBorder(isSelected ? Color.accentColor : .clear, lineWidth: 1.5)
            }
            footer
        }
        .frame(maxWidth: .infinity, alignment: message.isFromUser ? .trailing : .leading)
        .padding(message.isFromUser ? .leading : .trailing, 60)
        .contentShape(.rect)
        .onTapGesture { model.detail = .message(message.id) }
        .onAppear { if message.isToUser { model.letter(message.id, isOnScreen: true) } }
        .onDisappear { if message.isToUser { model.letter(message.id, isOnScreen: false) } }
        .accessibilityElement(children: .contain)
    }

    private var header: some View {
        HStack(spacing: 5) {
            MemberName(name: message.from)
            Image(systemName: "arrow.right").imageScale(.small).foregroundStyle(.tertiary)
            MemberName(name: message.to)
            Text(verbatim: "\(Wording.messageKind(message.kind)) \(UntrustedText.line(message.id, limit: 24))")
                .font(.caption.monospaced())
                .foregroundStyle(.secondary)
                .padding(.leading, 4)
            if message.device != nil, !message.isFromUser {
                // Should not happen; worth seeing if it does.
                Image(systemName: "questionmark.diamond").help("Came through a device but is not from the user")
            }
        }
        .font(.callout)
    }

    private var footer: some View {
        let state = Wording.messageState(message.state)
        return HStack(spacing: 10) {
            if message.isToUser, message.isOpenQuestion {
                StatusChip(text: "Waiting for you", symbol: "hand.raised.fill", waits: true)
            } else if message.kind == "task", case .handed = message.state, !message.isToUser {
                WorkingLabel()
            } else {
                StatusChip(text: state.text, symbol: state.symbol)
            }
            Text(verbatim: Wording.time(message.at)).font(.caption).foregroundStyle(.tertiary).monospacedDigit()
        }
    }

    private var background: AnyShapeStyle {
        message.isFromUser ? AnyShapeStyle(Color.accentColor.opacity(0.16)) : AnyShapeStyle(.quaternary.opacity(0.6))
    }
}

struct FileChip: View {
    let file: TextFile
    @State private var isOpen = false

    var body: some View {
        DisclosureGroup(isExpanded: $isOpen) {
            PlainBlock(text: file.content, limit: 220)
        } label: {
            Label {
                Text(verbatim: UntrustedText.line(file.name, limit: 80))
            } icon: {
                Image(systemName: "doc.text")
            }
            .font(.callout)
        }
    }
}

/// Text as it came, in a box that scrolls once it is long.
struct PlainBlock: View {
    let text: String
    var limit: CGFloat = 320
    var monospaced = true

    var body: some View {
        // Prose wraps; output keeps its lines.
        ScrollView(monospaced ? [.vertical, .horizontal] : .vertical) {
            Text(verbatim: UntrustedText.clean(text))
                .font(monospaced ? .callout.monospaced() : .callout)
                .textSelection(.enabled)
                .padding(10)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
        .defaultScrollAnchor(.topLeading)
        .frame(maxHeight: limit)
        .fixedSize(horizontal: false, vertical: true)
        .background(.quaternary.opacity(0.4), in: .rect(cornerRadius: 8))
    }
}
