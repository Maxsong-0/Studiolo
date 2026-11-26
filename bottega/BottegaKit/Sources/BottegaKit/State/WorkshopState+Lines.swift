// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.

// How lines are arranged, lines straight to one agent, and models (LMAS `docs/bottega-protocol.md`, section 11).

/// What the user made of a line.
public struct Arrangement: Equatable, Sendable {
    /// nil when it has none of its own.
    public internal(set) var title: String?
    public internal(set) var isPinned = false
    public internal(set) var isArchived = false
    /// Up to which event it has been read; 0 when it has not.
    public internal(set) var readThrough: UInt64 = 0
}

/// A repository whose copies agents may work on.
public struct Project: Equatable, Sendable, Identifiable {
    public let name, path: String
    public let addedAt: UInt64

    public var id: String { name }
}

/// A line straight to one agent. Its number is also the agent's session name.
public struct DirectLine: Equatable, Sendable, Identifiable {
    public let id: String
    public internal(set) var project: String?
    public internal(set) var vendor: String?
    public internal(set) var role: String?
    /// What it was opened with; what it uses now is in `WorkshopState.models`.
    public internal(set) var model: ModelChoice?
    public internal(set) var openedAt: UInt64?
    public internal(set) var place: Place?
    public internal(set) var failure: Failure?
    /// nil until the agent first came up.
    public internal(set) var worker: Worker?

    public struct Place: Equatable, Sendable {
        public let worktree, gitDir, branch, base: String
        public let at: UInt64
    }

    public struct Failure: Equatable, Sendable {
        public let why: Why
        public let at: UInt64
    }

    public struct Worker: Equatable, Sendable {
        /// `up`, `down`.
        public let state: String
        /// Which start of it, from 1.
        public let run: Int
        public let why: Why?
        public let at: UInt64

        public var isUp: Bool { state == "up" }
    }

    /// Neither ready nor failed yet.
    public var isBeingSetUp: Bool { place == nil && failure == nil }

    init(id: String) { self.id = id }
}

public struct ModelChoice: Equatable, Hashable, Sendable {
    public var model: String
    /// nil when it was not said.
    public var effort: String?

    public init(model: String, effort: String? = nil) {
        self.model = model
        self.effort = effort
    }

    /// Whether `other` is what came of asking for this. The effort may have been left to the model.
    func isMet(by other: ModelChoice) -> Bool { model == other.model && (effort == nil || effort == other.effort) }
}

/// What a session uses, as far as LMAS has told.
public struct SessionModel: Equatable, Sendable {
    /// What its last turn ran with, or what it was changed to since.
    public internal(set) var current: ModelChoice?
    /// Asked for, not in use yet.
    public internal(set) var requested: ModelChoice?
    /// The last change asked for did not come about.
    public internal(set) var refusal: Refusal?

    public struct Refusal: Equatable, Sendable {
        public let choice: ModelChoice
        public let why: Why
        public let at: UInt64
    }
}

/// What a vendor offers.
public enum Catalog: Equatable, Sendable {
    case listed(Event.ModelsListed)
    /// Cannot be asked now: nothing of it to choose.
    case unavailable(Why, at: UInt64)

    public var listed: Event.ModelsListed? { if case .listed(let list) = self { list } else { nil } }
}

// MARK: Taking events

extension WorkshopState {
    mutating func arranged(_ event: Event.LineArranged) {
        var arrangement = arrangements[event.line] ?? Arrangement()
        if let title = event.title { arrangement.title = title.isEmpty ? nil : title }
        if let pinned = event.pinned { arrangement.isPinned = pinned }
        if let archived = event.archived { arrangement.isArchived = archived }
        arrangements[event.line] = arrangement
    }

    mutating func read(_ event: Event.LineRead) {
        var arrangement = arrangements[event.line] ?? Arrangement()
        arrangement.readThrough = max(arrangement.readThrough, event.through)
        arrangements[event.line] = arrangement
    }

    mutating func directLine(_ id: String, _ change: (inout DirectLine) -> Void) {
        change(&directLines[id, default: DirectLine(id: id)])
    }

    mutating func opened(_ event: Event.LineOpened) {
        directLine(event.line) {
            $0.project = event.project
            $0.vendor = event.vendor
            $0.role = event.role
            $0.model = event.model.map { ModelChoice(model: $0, effort: event.effort) }
            $0.openedAt = event.atMs
        }
        vendors[event.line] = event.vendor
    }

    mutating func ready(_ event: Event.LineReady) {
        directLine(event.line) {
            $0.place = .init(
                worktree: event.worktree, gitDir: event.gitDir, branch: event.branch, base: event.base, at: event.atMs)
        }
    }

    mutating func worker(_ event: Event.LineWorker) {
        directLine(event.line) { $0.worker = .init(state: event.state, run: event.run, why: event.why, at: event.atMs) }
    }

    mutating func uses(_ session: String, _ choice: ModelChoice?) {
        if let choice { models[session, default: SessionModel()].current = choice }
    }

    /// What was asked for later is still to come.
    mutating func changed(_ event: Event.ModelChanged) {
        let choice = ModelChoice(model: event.model, effort: event.effort)
        var model = models[event.session] ?? SessionModel()
        model.current = choice
        model.refusal = nil
        if model.requested?.isMet(by: choice) == true { model.requested = nil }
        models[event.session] = model
    }

    mutating func notChanged(_ event: Event.ModelNotChanged) {
        let choice = ModelChoice(model: event.model, effort: event.effort)
        var model = models[event.session] ?? SessionModel()
        model.refusal = .init(choice: choice, why: event.why, at: event.atMs)
        if model.requested?.isMet(by: choice) == true { model.requested = nil }
        models[event.session] = model
    }

    /// A direct line's agent is started again with what was asked for; anyone else starts as they were configured.
    mutating func forgetModelRequest(of session: String) {
        if directLines[session] == nil { models[session]?.requested = nil }
    }
}

// MARK: Reading

extension WorkshopState {
    public func arrangement(of line: String) -> Arrangement { arrangements[line] ?? Arrangement() }

    /// The session a line is part of the talk with: the one the user started it with. nil for everyone else's lines.
    public func conversation(of thread: Line) -> String? {
        thread.messages.first.flatMap { messages[$0] }.flatMap { $0.isFromUser ? $0.to : nil }
    }

    /// The user's lines with one session, oldest first.
    public func threads(with session: String) -> [Line] {
        threads.values.filter { conversation(of: $0) == session }
            .sorted { (firstSeq($0), $0.id) < (firstSeq($1), $1.id) }
    }

    private func firstSeq(_ thread: Line) -> UInt64 { thread.messages.first.flatMap { messages[$0]?.seq } ?? 0 }

    /// Up to which event a line has been read, on its own or as part of the talk with a session.
    public func readThrough(_ thread: Line) -> UInt64 {
        max(arrangement(of: thread.id).readThrough, conversation(of: thread).map { arrangement(of: $0).readThrough } ?? 0)
    }

    /// Whether anyone else has written on a line since it was read.
    public func hasUnread(_ thread: Line) -> Bool { unreadThrough(thread) != nil }

    /// Up to which event a line counts as read once everything on it now has been (`mark_read`): the latest letter
    /// anyone else wrote that is not read yet. nil with nothing unread.
    public func unreadThrough(_ thread: Line) -> UInt64? {
        let read = readThrough(thread)
        return thread.messages.lazy.compactMap { messages[$0] }.filter { !$0.isFromUser && $0.seq > read }.map(\.seq).max()
    }

    /// The models to choose from for a session, as its vendor lists them. Without, it cannot be changed.
    public func catalog(for session: String) -> Event.ModelsListed? { vendors[session].flatMap { catalogs[$0]?.listed } }

    /// Whether a session's model can be changed (`set_model`): an agent LMAS runs whose vendor has listed its models.
    /// The dispatcher and the Planner are not LMAS's to change.
    public func canChangeModel(of session: String) -> Bool {
        catalog(for: session) != nil && !["dispatcher", "planner"].contains(role(of: session) ?? "")
    }

    /// A card's own coder or reviewer. Its turns are not the user's to stop (`not_stoppable`): the card would be left
    /// without a result.
    public func isCardAgent(_ session: String) -> Bool {
        cards.values.contains { card in card.work?.worker == session || card.reviews.contains { $0.reviewer == session } }
    }

    /// Names in order.
    public var projectList: [Project] { projects.values.sorted { $0.name < $1.name } }

    /// Newest first.
    public var directLineList: [DirectLine] {
        directLines.values.sorted { ($0.openedAt ?? 0, $0.id) > ($1.openedAt ?? 0, $1.id) }
    }
}
