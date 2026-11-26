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

    /// Pinned ones first, put away ones last, newest first among them.
    public func threads(of kind: LineKind) -> [Line] {
        func rank(_ line: Line) -> Int {
            let arrangement = arrangement(of: line.id)
            return arrangement.isArchived ? 0 : arrangement.isPinned ? 2 : 1
        }
        return threads.values.filter { self.kind(of: $0) == kind }
            .sorted { (rank($0), $0.lastAt, $0.id) > (rank($1), $1.lastAt, $1.id) }
    }

    /// What its limit on letters counts: the user's own do not.
    public func lettersThatCount(on thread: Line) -> Int {
        thread.messages.count { id in messages[id].map { !$0.isFromUser } ?? false }
    }

    public func role(of session: String) -> String? { sessions[session]?.role }

    /// The line's name: the one the user gave it, the title of the card it carries, or the first line of its first
    /// letter.
    public func title(of thread: Line) -> String {
        if let title = arrangement(of: thread.id).title.map({ UntrustedText.line($0, limit: 80) }), !title.isEmpty { return title }
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

    /// Who the user can start a line with. A direct line's session is registered when it opens (section 11) and
    /// closed when it is put away or never got ready.
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

    /// A stretch of the lanes during which a member was in a turn. Where LMAS tells no turns (of the Planner, and
    /// of anyone before LMAS told them), during which it had something to answer.
    public struct Stretch: Equatable, Sendable {
        public let lane: Int
        /// In rows: 0 is the top of the first letter's row, 0.5 its middle, 1 its bottom.
        public let top: Double
        /// nil while it is still going on.
        public let bottom: Double?
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
        for (lane, member) in lanes.members.enumerated() {
            let turns = turns(of: member)
            lanes.working += answering(lane, lanes.letters, position, before: turns.compactMap(\.startedAt).min())
            lanes.working += turns.compactMap { stretch(of: $0, lane, lanes.letters, position) }
        }
        return lanes
    }

    /// From a TASK to what answers it, for those delivered before any turn of the member was told.
    private func answering(_ lane: Int, _ letters: [Letter], _ position: [String: Int], before first: UInt64?) -> [Stretch] {
        letters.enumerated().compactMap { index, letter in
            guard letter.to == lane, letter.kind == "task", let message = messages[letter.id] else { return nil }
            if let first, message.handedAt ?? first >= first || !turns(with: message).isEmpty { return nil }
            let top = Double(index) + 0.5
            return switch message.state {
            case .handed: Stretch(lane: lane, top: top, bottom: nil)
            case .answered(let reply): Stretch(lane: lane, top: top, bottom: Double(position[reply] ?? index) + 0.5)
            case .queued, .failed: nil
            }
        }
    }

    /// Where a turn is beside the letters: after those sent before it, down to those sent before it ended. nil when it
    /// had nothing to do with this line.
    private func stretch(of turn: Turn, _ lane: Int, _ letters: [Letter], _ position: [String: Int]) -> Stretch? {
        let pushed = turn.messages.compactMap { position[$0] }.min()
        // Most turns of a busy member are over before this line began, or began after it went quiet.
        if pushed == nil, let first = letters.first.flatMap({ messages[$0.id]?.at }), let end = turn.end?.at, end < first { return nil }
        let isOnThisLine = pushed != nil || letters.contains { letter in
            guard let message = messages[letter.id] else { return false }
            return letter.from == lane && turn.wasRunning(at: message.at)
                || letter.to == lane && message.handedAt.map(turn.wasRunning(at:)) ?? false
        }
        guard isOnThisLine else { return nil }
        func sent(before moment: UInt64) -> Int { letters.count { messages[$0.id].map { $0.at <= moment } ?? false } }
        let top = pushed.map { Double($0) + 0.5 } ?? Double(turn.startedAt.map { $0 == 0 ? 0 : sent(before: $0 - 1) } ?? 0)
        return Stretch(lane: lane, top: top, bottom: turn.end.map { max(Double(sent(before: $0.at)), top + 0.5) })
    }

    // MARK: Turns

    /// A session's turns, oldest first.
    public func turns(of session: String) -> [Turn] { (turnsBySession[session] ?? []).compactMap { turns[$0] } }

    /// The turn a session is in.
    public func runningTurn(of session: String) -> Turn? { turns(of: session).last { $0.isRunning } }

    /// The turns in which a letter was taken in.
    public func turns(with message: Message) -> [Turn] {
        turns(of: message.to).filter { turn in
            turn.messages.contains(message.id) || message.handedAt.map(turn.wasRunning(at:)) ?? false
        }
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
