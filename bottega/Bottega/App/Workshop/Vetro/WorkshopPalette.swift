// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import BottegaKit
import SwiftUI

/// ⌘K (vault Bottega-6, U4; the canvas's `.palw`): lines, cards, pages, bots and what can be done here, found by
/// typing. Which rows come first is `PaletteSearch`'s. What a row does that gets signed goes through
/// `WorkshopModel` as from anywhere else; Return and a click are both the user's own press.
struct WorkshopPalette: View {
    @Environment(WorkshopModel.self) private var model
    @Environment(Look.self) private var look
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    @State private var query = Self.typedFirst
    /// The row picked, by what it is rather than where: rows come and go as LMAS's events do, and Return must run
    /// the one that was lit, or nothing.
    @State private var chosen: Row.ID?
    @FocusState private var isFocused: Bool

    #if DEBUG
    /// `BOTTEGA_PALETTE=<text>`: what is typed when it first comes up, for a screenshot.
    private static var typedFirst: String { ProcessInfo.processInfo.environment["BOTTEGA_PALETTE"] ?? "" }
    /// Not for a screenshot: a field taking the keyboard brings the app forward, and Stage Manager then shrinks its
    /// window to a thumbnail.
    private static var takesKeyboard: Bool { ProcessInfo.processInfo.environment["BOTTEGA_PALETTE"] == nil }
    #else
    private static let typedFirst = ""
    private static let takesKeyboard = true
    #endif

    /// The canvas's sizes, in one place.
    private enum Metrics {
        static let width: CGFloat = 520
        static let top: CGFloat = 120
        static let fieldHeight: CGFloat = 48
        static let rowHeight: CGFloat = 34
        static let cornerRadius: CGFloat = 18
        static let maxRows = 12
        /// With nothing typed: the latest few, the rest once something is.
        static let recentLines = 4
        static let recentCards = 2
    }

    var body: some View {
        let rows = rows
        ZStack(alignment: .top) {
            // Nothing is drawn over the page (no dark veil, T-245): the panel's own shadow sets it apart, as
            // Spotlight's does. A click anywhere off the panel still closes it.
            Color.clear
                .contentShape(.rect)
                .onTapGesture { close() }
                .accessibilityHidden(true)
            panel(rows)
                .padding(.top, Metrics.top)
                .transition(
                    .asymmetric(
                        insertion: reduceMotion ? .opacity : .modifier(active: Rising(isOn: false), identity: Rising(isOn: true)),
                        removal: .opacity))
        }
        .onAppear { isFocused = Self.takesKeyboard }
        // What is typed picks the first of what it finds, even when that is what was there before.
        .onChange(of: query) { chosen = self.rows.first?.id }
        // Nothing picked yet: the first row is, by name, from the moment it is shown. A lit row that an event takes
        // away (Pin turning into Unpin) leaves nothing lit, on purpose: falling to the first row could light the
        // opposite of what was meant just as Return goes down. The arrow keys start again from the top.
        .onChange(of: rows.map(\.id), initial: true) { _, ids in if chosen == nil { chosen = ids.first } }
    }

    private func panel(_ rows: [Row]) -> some View {
        VStack(spacing: 0) {
            HStack(spacing: 10) {
                Image(systemName: "magnifyingglass").foregroundStyle(.secondary)
                TextField("Search lines, cards, bots, commands", text: $query)
                    .textFieldStyle(.plain)
                    .font(.system(size: 14))
                    .focused($isFocused)
                    .onSubmit { run(rows) }
                    .onKeyPress(.upArrow) {
                        move(by: -1, in: rows)
                        return .handled
                    }
                    .onKeyPress(.downArrow) {
                        move(by: 1, in: rows)
                        return .handled
                    }
                    .onKeyPress(.escape) {
                        close()
                        return .handled
                    }
                    .accessibilityLabel("Search")
                Key("⌘K")
            }
            .padding(.horizontal, 16)
            .frame(height: Metrics.fieldHeight)
            Divider().padding(.bottom, 6)
            if rows.isEmpty {
                // Never a blank box.
                Text(verbatim: "Nothing matches “\(UntrustedText.line(query, limit: 60))”")
                    .font(.system(size: 13))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .frame(maxWidth: .infinity, minHeight: Metrics.rowHeight, alignment: .leading)
                    .padding(.horizontal, 16)
            } else {
                VStack(spacing: 0) {
                    ForEach(rows) { row in
                        line(row, isSelected: row.id == chosen)
                            // The pointer and the arrow keys never disagree about what Return runs.
                            .onHover { if $0 { chosen = row.id } }
                            .onTapGesture { run(row) }
                    }
                }
                .padding(.horizontal, 6)
            }
        }
        .padding(.bottom, 6)
        .frame(width: Metrics.width)
        .background(look.surface, in: .rect(cornerRadius: Metrics.cornerRadius, style: .continuous))
        .vetroEdge(in: RoundedRectangle(cornerRadius: Metrics.cornerRadius, style: .continuous))
        .background {
            // The shadow is the glass's, not the rows': thrown by what is on the panel it would sit under every
            // word.
            Color.clear
                .glassEffect(.regular.tint(look.glassTint), in: .rect(cornerRadius: Metrics.cornerRadius, style: .continuous))
                .shadow(color: .black.opacity(0.28), radius: 30, y: 12)
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Search and commands")
    }

    private func line(_ row: Row, isSelected: Bool) -> some View {
        HStack(spacing: 10) {
            glyph(row.glyph).frame(minWidth: 18)
            Text(verbatim: row.title).font(.system(size: 13)).lineLimit(1)
            if !row.subtitle.isEmpty {
                Text(verbatim: row.subtitle).font(.system(size: 11.5)).foregroundStyle(.secondary).lineLimit(1)
            }
            Spacer(minLength: 8)
            if let key = row.key { Key(key) }
        }
        .padding(.horizontal, 10)
        .frame(height: Metrics.rowHeight)
        .background(isSelected ? look.selection : .clear, in: .rect(cornerRadius: 9, style: .continuous))
        .contentShape(.rect)
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(isSelected ? [.isButton, .isSelected] : .isButton)
    }

    @ViewBuilder private func glyph(_ glyph: Row.Glyph) -> some View {
        switch glyph {
        case .symbol(let name):
            Image(systemName: name).font(.system(size: 12.5)).foregroundStyle(.secondary)
        case .maker(let maker):
            MakerTile(maker: maker, size: 16)
        case .waits:
            Circle().fill(.red).frame(width: 8, height: 8).accessibilityLabel("Waiting for you")
        case .card(let id):
            Text(verbatim: UntrustedText.line(id, limit: 12))
                .font(.system(size: 11, design: .monospaced))
                .foregroundStyle(.secondary)
        }
    }

    /// Return: the row lit, if it is still there.
    private func run(_ rows: [Row]) {
        guard let row = rows.first(where: { $0.id == chosen }) else { return }
        run(row)
    }

    private func run(_ row: Row) {
        close()
        row.run()
    }

    /// From the row lit, or from the top when the one lit has gone.
    private func move(by step: Int, in rows: [Row]) {
        guard !rows.isEmpty else { return }
        let index = rows.firstIndex { $0.id == chosen }.map { min(max(0, $0 + step), rows.count - 1) } ?? 0
        chosen = rows[index].id
    }

    private func close() { model.showsPalette = false }

    // MARK: Rows

    struct Row: Identifiable {
        enum Glyph {
            case symbol(String)
            case maker(Maker)
            case waits
            case card(String)
        }

        let id: String
        let title: String
        var subtitle = ""
        /// Looked in ahead of the title, never shown as text: a card's number, which its glyph shows already. Typing
        /// one finds that card before a line that only mentions it.
        var terms = ""
        let glyph: Glyph
        var key: String?
        let run: () -> Void
    }

    /// In the order they are listed with nothing typed; what is typed reorders them by how well they answer.
    private var rows: [Row] {
        let isTyped = !query.allSatisfy(\.isWhitespace)
        let all = waiting + here + lines(all: isTyped) + cards(all: isTyped) + bots + pages + view
        return Array(
            PaletteSearch.ranked(all, for: query, title: { $0.terms.isEmpty ? $0.title : "\($0.terms) \($0.title)" }, subtitle: \.subtitle)
                .prefix(Metrics.maxRows))
    }

    /// The oldest thing waiting for the user, opened where it can be answered.
    private var waiting: [Row] {
        guard let item = model.waiting.first else { return [] }
        return [
            Row(id: "waiting:\(item.what)", title: "Review the oldest request", subtitle: Wording.waiting(item, in: model.state), glyph: .waits) {
                switch item.what {
                case .ask(let id): model.askBeingRead = .init(id: id)
                case .question(let id): model.page = model.state.messages[id].map { .line($0.thread) } ?? .waiting
                case .trip: model.page = .waiting
                }
            }
        ]
    }

    /// What can be done to the line that is open.
    private var here: [Row] {
        var rows: [Row] = []
        if let id = model.page.line, model.state.threads[id] != nil {
            rows.append(Row(id: "rename:\(id)", title: "Rename line…", glyph: .symbol("character.cursor.ibeam")) {
                model.arranging = .init(line: id, inSidebar: false)
            })
        }
        // Only with a link to send them on (not for a recording): the panel says why its buttons are off, a palette
        // row would do nothing and say nothing.
        if let id = model.page.line, model.state.threads[id] != nil, model.client != nil {
            let arrangement = model.state.arrangement(of: id)
            // Each says what it does, and its name changes with it, so a row lit as Pin never runs as Unpin.
            let pin = !arrangement.isPinned
            rows.append(
                Row(id: "\(pin ? "pin" : "unpin"):\(id)", title: pin ? "Pin line" : "Unpin line", glyph: .symbol(pin ? "pin" : "pin.slash")) {
                    model.setPinned(id, pin)
                })
            let archive = !arrangement.isArchived
            rows.append(
                Row(
                    id: "\(archive ? "archive" : "unarchive"):\(id)", title: archive ? "Archive line" : "Unarchive line",
                    glyph: .symbol(archive ? "archivebox" : "tray.and.arrow.up")
                ) { model.setArchived(id, archive) })
        }
        if let turn = model.turnToStop, model.isLive {
            rows.append(Row(id: "stop:\(turn.id)", title: "Stop this turn", glyph: .symbol("stop.circle"), key: "⌘.") { model.stop(turn) })
        }
        return rows
    }

    /// The latest lines, the one open aside; every line once something is typed, put away ones too.
    private func lines(all: Bool) -> [Row] {
        let open = model.page.line
        let lines = WorkshopState.LineKind.allCases.flatMap { kind in model.state.threads(of: kind).map { (kind, $0) } }
            .filter { all || ($0.1.id != open && !model.state.arrangement(of: $0.1.id).isArchived) }
            .sorted { ($0.1.lastAt, $0.1.id) > ($1.1.lastAt, $1.1.id) }
        return (all ? lines : Array(lines.prefix(Metrics.recentLines))).map { kind, line in
            let arrangement = model.state.arrangement(of: line.id)
            let place = arrangement.isArchived ? " · Archived" : arrangement.isPinned ? " · Pinned" : ""
            return Row(
                id: "line:\(line.id)", title: model.state.title(of: line), subtitle: Wording.lineKind(kind) + place,
                glyph: .symbol(arrangement.isPinned ? "pin" : "text.bubble")
            ) { model.page = .line(line.id) }
        }
    }

    /// The cards most lately moved, or every card once something is typed.
    private func cards(all: Bool) -> [Row] {
        let cards = model.state.cardsByRecentActivity
        return (all ? cards : Array(cards.prefix(Metrics.recentCards))).map { card in
            Row(
                id: "card:\(card.id)", title: UntrustedText.line(card.title ?? "Untitled", limit: 80),
                subtitle: Wording.column(model.state.column(of: card)), terms: card.id, glyph: .card(card.id)
            ) {
                model.page = .board
                model.detail = .card(card.id)
            }
        }
    }

    /// A line straight to a bot starts at its home.
    private var bots: [Row] {
        BotGroups(model.state, configured: model.overview?.cards?.vendors ?? []).makers.map { group in
            Row(id: "bot:\(group.maker.vendor)", title: "New line with \(group.maker.title)", glyph: .maker(group.maker)) {
                model.page = .bot(group.maker.vendor)
            }
        }
    }

    private var pages: [Row] {
        let sections = SidebarSection.allCases.map { section in
            Row(
                id: "section:\(section.rawValue)", title: "Go to \(section.title)", glyph: .symbol(section.symbol),
                key: "⌘\(section.rawValue + 1)"
            ) { model.show(section) }
        }
        let others: [(Page, String)] = [(.dispatcher, "⌘0"), (.devices, "⌘6"), (.projects, "⌘7")]
        return sections + others.map { page, key in
            Row(id: "page:\(page.fixedTitle)", title: "Go to \(page.fixedTitle)", glyph: .symbol(page.symbol), key: key) {
                model.page = page
            }
        }
    }

    private var view: [Row] {
        let isList = model.cardLayout == .list
        let layout: WorkshopModel.CardLayout = isList ? .board : .list
        let showsInspector = !model.showsInspector
        let showsSidebar = !model.showsSidebar
        var rows = [
            Row(id: "layout:\(layout.rawValue)", title: isList ? "Show cards as a board" : "Show cards as a list", glyph: .symbol(isList ? "rectangle.split.3x1" : "list.bullet")) {
                // The page first: going to it forgets a switch (`CardsMotion.Cue`), so the other way round the board
                // would not rise for this one.
                model.page = .board
                withAnimation(CardsMotion.switching(reduceMotion: reduceMotion)) { model.cardLayout = layout }
            },
            Row(
                id: "sidebar:\(showsSidebar)", title: showsSidebar ? "Show the sidebar" : "Hide the sidebar",
                glyph: .symbol("sidebar.leading"), key: "⌃⌘S"
            ) { model.showsSidebar = showsSidebar },
            Row(
                id: "panel:\(showsInspector)", title: showsInspector ? "Show the panel" : "Hide the panel",
                glyph: .symbol("sidebar.trailing"), key: "⌥⌘0"
            ) { model.showsInspector = showsInspector },
        ]
        if model.hasLink {
            rows.append(Row(id: "reconnect", title: "Reconnect to LMAS", glyph: .symbol("arrow.triangle.2.circlepath")) { model.reconnect() })
            rows.append(Row(id: "reload", title: "Reload all events", glyph: .symbol("arrow.clockwise")) { model.reload() })
        }
        return rows
    }
}

/// A keyboard shortcut, as the canvas's `.kbd`.
private struct Key: View {
    @Environment(Look.self) private var look
    let text: String

    init(_ text: String) { self.text = text }

    var body: some View {
        Text(verbatim: text)
            .font(.system(size: 11, design: .monospaced))
            .foregroundStyle(.secondary)
            .padding(.horizontal, 6)
            .padding(.vertical, 2)
            .background(look.selection, in: .rect(cornerRadius: 5, style: .continuous))
            .accessibilityHidden(true)
    }
}

/// The canvas's `palin`: up from a little smaller and out of focus.
private struct Rising: ViewModifier {
    let isOn: Bool

    func body(content: Content) -> some View {
        content
            .opacity(isOn ? 1 : 0)
            .scaleEffect(isOn ? 1 : 0.97, anchor: .top)
            .blur(radius: isOn ? 0 : 3)
    }
}
