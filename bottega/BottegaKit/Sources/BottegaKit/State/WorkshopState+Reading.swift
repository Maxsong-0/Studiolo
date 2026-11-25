// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.

// What the pages ask of the state. Nothing here changes it.

extension WorkshopState {
    // MARK: Lines

    public enum LineKind: String, CaseIterable, Sendable {
        /// The user talking to the dispatcher.
        case dispatcher
        /// The user talking to one agent.
        case direct
        /// Started by the Planner, the Claude app session.
        case planner
        /// Between agents.
        case agents
    }

    public func kind(of thread: Line) -> LineKind {
        guard let first = thread.messages.first.flatMap({ messages[$0] }) else { return .agents }
        if first.isFromUser { return role(of: first.to) == "dispatcher" ? .dispatcher : .direct }
        if role(of: first.from) == "planner" { return .planner }
        return .agents
    }

    /// Newest first.
    public func threads(of kind: LineKind) -> [Line] {
        threads.values.filter { self.kind(of: $0) == kind }
            .sorted { ($0.lastAt, $0.id) > ($1.lastAt, $1.id) }
    }

    public func role(of session: String) -> String? { sessions[session]?.role }

    /// The line's name: the first line of its first letter, or the title of the card it carries.
    public func title(of thread: Line) -> String {
        if let title = cardsByTask[thread.id].flatMap({ cards[$0]?.title }) {
            return UntrustedText.line(title, limit: 80)
        }
        guard let first = thread.messages.first.flatMap({ messages[$0] }) else { return thread.id }
        let line = first.body.split(whereSeparator: \.isNewline).first.map(String.init) ?? ""
        let title = UntrustedText.line(String(line.drop { $0 == "#" || $0 == " " }), limit: 80)
        return title.isEmpty ? thread.id : title
    }

    /// Who the user can talk to on a line: its members that are still open, agents only.
    public func recipients(on thread: Line) -> [Session] {
        thread.members.compactMap { sessions[$0] }.filter { $0.isOpen && canBeSpokenTo($0) }
    }

    /// Who the user can start a line with.
    public var recipients: [Session] {
        sessions.values.filter { $0.isOpen && canBeSpokenTo($0) }
            .sorted { ($0.role == "dispatcher" ? 0 : 1, $0.name) < ($1.role == "dispatcher" ? 0 : 1, $1.name) }
    }

    private func canBeSpokenTo(_ session: Session) -> Bool { session.role != "user" && session.name != Session.user }

    // MARK: Cards

    public enum Column: String, CaseIterable, Sendable {
        case queued, working, needsYou, done, failed
    }

    public func column(of card: Card) -> Column {
        if isWaitingOnUser(card) { return .needsYou }
        switch card.end {
        case .done: return .done
        case .failed: return .failed
        case nil: return card.work == nil ? .queued : .working
        }
    }

    /// Cards in a column, oldest first.
    public func cards(in column: Column) -> [Card] {
        cardOrder.compactMap { cards[$0] }.filter { self.column(of: $0) == column }
    }

    private func isWaitingOnUser(_ card: Card) -> Bool {
        waiting.contains { item in
            switch item.what {
            case .ask(let id): asks[id]?.about.merge.map(card.merges.contains) ?? false
            case .question(let id): card.work.map { $0.worker == messages[id]?.from } ?? false
            case .trip: false
            }
        }
    }

    // MARK: Lanes

    /// One letter on a line, as the lanes draw it.
    public struct Letter: Equatable, Sendable, Identifiable {
        public let id: String
        public let from, to: Int
        public let kind: String
        /// Not one of the three that ask for something: a RESULT or a notification.
        public let isReply: Bool
    }

    /// A stretch of letters during which a member had something to answer.
    public struct Stretch: Equatable, Sendable {
        public let lane: Int
        /// Positions in `letters`.
        public let start: Int
        /// nil while it is still going on.
        public let end: Int?
    }

    public struct Lanes: Equatable, Sendable {
        public var members: [String] = []
        public var letters: [Letter] = []
        public var working: [Stretch] = []
    }

    public func lanes(of thread: Line) -> Lanes {
        var lanes = Lanes(members: thread.members)
        var position: [String: Int] = [:]
        for id in thread.messages {
            guard
                let message = messages[id], let from = lanes.members.firstIndex(of: message.from),
                let to = lanes.members.firstIndex(of: message.to)
            else { continue }
            position[id] = lanes.letters.count
            lanes.letters.append(
                Letter(
                    id: id, from: from, to: to, kind: message.kind,
                    isReply: message.kind == "result" || message.kind == "notification"))
        }
        for (index, letter) in lanes.letters.enumerated() {
            guard letter.kind == "task", let message = messages[letter.id] else { continue }
            switch message.state {
            case .handed: lanes.working.append(Stretch(lane: letter.to, start: index, end: nil))
            case .answered(let reply): lanes.working.append(Stretch(lane: letter.to, start: index, end: position[reply] ?? index))
            case .queued, .failed: break
            }
        }
        return lanes
    }

    // MARK: Seen

    /// Up to which record everything the user has been shown reaches, given the messages to them that have been on
    /// screen: just before the first one that waits to be delivered and has not. nil when saying so would deliver
    /// nothing; what is delivered already counts as shown, whoever showed it.
    public func seenThrough(shown: Set<String>) -> UInt64? {
        var delivers = false
        for id in messagesToUser {
            guard let message = messages[id], message.state == .queued else { continue }
            guard shown.contains(id) else { return delivers ? max(message.seq, 1) - 1 : nil }
            delivers = true
        }
        return delivers ? through : nil
    }
}
