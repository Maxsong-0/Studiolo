// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.

/// The records did not follow on from each other. Nothing after this may be shown as if it did.
public struct SequenceBreak: Error, Equatable, Sendable {
    public let expected: UInt64
    public let got: UInt64
}

/// What the events so far add up to. Pure: the same records in the same order always give the same state.
///
/// Events may name things this state has never heard of (history starts in the middle, or an event in between
/// was unreadable); such a thing is created from what the event says, so nothing that happened goes unshown.
public struct WorkshopState: Sendable {
    /// Up to which number everything is in here. One less than the next record's `seq`.
    public private(set) var through: UInt64
    /// Where history starts: 1, or later when LMAS has cleared what came before.
    public let start: UInt64
    private var isFresh = true

    public private(set) var sessions: [String: Session] = [:]
    public private(set) var messages: [String: Message] = [:]
    public private(set) var threads: [String: Line] = [:]
    public private(set) var cards: [String: Card] = [:]
    /// In the order they were first heard of.
    public private(set) var cardOrder: [String] = []
    /// The card whose TASK a message is.
    public private(set) var cardsByTask: [String: String] = [:]
    /// The agent family (`claude`, `codex`…) a card was given to or is reviewed by, by the session doing it.
    public private(set) var vendors: [String: String] = [:]
    public private(set) var tests: [String: TestRun] = [:]
    public private(set) var merges: [String: Merge] = [:]
    public private(set) var asks: [String: Ask] = [:]
    public private(set) var trips: [TripID: Trip] = [:]
    public private(set) var devices: [String: Device] = [:]
    /// By the intent's digest.
    public private(set) var echoes: [String: Echo] = [:]

    /// Every event as it happened, in order: the activity log.
    public private(set) var log: [Happening] = []
    /// What the dispatcher's page shows: the user's talk with the bus, and what comes of it.
    public private(set) var dispatcherRows: [Row] = []

    /// Messages to the user, in order; the app says which of them it has shown.
    public private(set) var messagesToUser: [String] = []
    private var openAsks: Set<String> = []
    private var openQuestions: Set<String> = []

    public init(startingAt start: UInt64 = 1) {
        self.start = max(1, start)
        through = self.start - 1
    }

    // MARK: Taking records

    public mutating func apply(_ record: Record) throws(SequenceBreak) {
        // The first record may begin before `start`: LMAS never cuts one in half.
        let follows = isFresh ? record.seq <= start && record.through >= start : record.seq == through + 1
        guard follows, record.through >= record.seq else {
            throw SequenceBreak(expected: through + 1, got: record.seq)
        }
        isFresh = false
        through = record.through
        for (index, event) in record.events.enumerated() {
            take(event, RowID(seq: record.seq, index: index))
        }
    }

    private mutating func take(_ event: Event, _ id: RowID) {
        // What cannot be read has no clock either.
        log.append(Happening(id: id, at: event.at ?? log.last?.at ?? 0, event: event))
        switch event {
        case .sessionRegistered(let event):
            sessions[event.name] = Session(name: event.name, role: event.role, openedAt: event.atMs)
        case .sessionClosed(let event):
            sessions[event.name, default: Session(name: event.name, role: "", openedAt: event.atMs)].closedAt = event.atMs
        case .messageQueued(let event): queued(event.message, id)
        case .messageHanded(let event):
            if case .queued = messages[event.id]?.state { messages[event.id]?.state = .handed(at: event.atMs) }
        case .messageFailed(let event): failed(event, id)
        case .cardCreated(let event): created(event, id)
        case .cardStarted(let event): started(event, id)
        case .cardDone(let event):
            touch(event.card) { $0.end = .done(result: event.result, at: event.atMs) }
            show(Row(id: id, at: event.atMs, kind: .cardEnded(event.card)), thread: cards[event.card]?.thread, dispatcher: true)
        case .cardFailed(let event):
            touch(event.card) { $0.end = .failed(event.why, at: event.atMs) }
            show(Row(id: id, at: event.atMs, kind: .cardEnded(event.card)), thread: cards[event.card]?.thread, dispatcher: true)
        case .limitTripped(let event):
            trips[id] = Trip(
                id: id, limit: event.limit, thread: event.thread, from: event.from, to: event.to, kind: event.kind,
                at: event.atMs)
            show(Row(id: id, at: event.atMs, kind: .trip(id)), thread: event.thread, dispatcher: true)
        case .testsSubmitted(let event):
            test(event.test) {
                $0.card = event.card
                $0.by = event.by
                $0.request = event.request
                $0.merge = event.merge
                $0.submittedAt = event.atMs
            }
            touch(event.card) { $0.tests.append(event.test) }
            show(Row(id: id, at: event.atMs, kind: .test(event.test)), thread: cards[event.card]?.thread)
        case .testsHandedOver(let event):
            test(event.test) { $0.stage = .handedOver(at: event.atMs) }
            touch(line: threadOf(test: event.test), at: event.atMs)
        case .testsAccepted(let event):
            test(event.test) {
                $0.stage = .accepted(at: event.atMs)
                $0.job = event.job
            }
            touch(line: threadOf(test: event.test), at: event.atMs)
        case .testsFinished(let event):
            test(event.test) { $0.stage = .finished(event.outcome, report: event.report, at: event.atMs) }
            touch(line: threadOf(test: event.test), at: event.atMs)
        case .reviewStarted(let event):
            touch(event.card) {
                $0.reviews.append(.init(reviewer: event.reviewer, task: event.task, startedAt: event.atMs))
            }
            vendors[event.reviewer] = cards[event.card]?.reviewer
            showReview(of: event.card, at: event.atMs, id)
        case .reviewFinished(let event):
            endReview(of: event.card, .finished(verdict: event.verdict, result: event.result, at: event.atMs), at: event.atMs, id)
        case .reviewNotDone(let event):
            endReview(of: event.card, .notDone(event.why, at: event.atMs), at: event.atMs, id)
        case .mergeRequested(let event):
            merge(event.merge) {
                $0.card = event.card
                $0.by = event.by
                $0.target = event.target
                $0.test = event.test
                $0.gaps = event.gaps ?? []
                $0.asksUser = event.asksUser
                $0.requestedAt = event.atMs
            }
            touch(event.card) { $0.merges.append(event.merge) }
            show(Row(id: id, at: event.atMs, kind: .merge(event.merge)), thread: cards[event.card]?.thread, dispatcher: true)
        case .mergeChecked(let event):
            step(event.merge, .checked(at: event.atMs), event.atMs, id) {
                $0.size = .init(files: event.files, insertions: event.insertions, deletions: event.deletions)
            }
        case .mergePrepared(let event):
            step(event.merge, .prepared(at: event.atMs), event.atMs, id) {
                $0.build = .init(
                    onto: event.onto, commit: event.commit, tree: event.tree, branch: event.branch,
                    worktree: event.worktree)
            }
        case .mergeRestarted(let event):
            step(event.merge, .restarted(event.why, at: event.atMs), event.atMs, id) {
                $0.restarts.append(event.why)
                $0.build = nil
            }
        case .mergeLanding(let event): step(event.merge, .landing(at: event.atMs), event.atMs, id)
        case .mergeFinished(let event): step(event.merge, .finished(at: event.atMs), event.atMs, id)
        case .mergeFailed(let event): step(event.merge, .failed(event.why, at: event.atMs), event.atMs, id)
        case .mergeCleaned(let event):
            merge(event.merge) { $0.cleaned = true }
            touch(line: threadOf(merge: event.merge), at: event.atMs)
        case .askOpened(let event):
            asks[event.ask] = Ask(
                id: event.ask, about: event.about, by: event.by, text: event.text,
                onlyDevice: event.onlyDevice ?? false, at: event.atMs)
            openAsks.insert(event.ask)
            show(
                Row(id: id, at: event.atMs, kind: .ask(event.ask)), thread: event.about.merge.flatMap(threadOf(merge:)),
                dispatcher: true)
        case .askAnswered(let event):
            if asks[event.ask] == nil {
                asks[event.ask] = Ask(
                    id: event.ask, about: .init(kind: "", merge: nil), by: "", text: "", onlyDevice: false, at: event.atMs)
            }
            asks[event.ask]?.answer = .init(word: event.answer, via: event.via, at: event.atMs)
            openAsks.remove(event.ask)
        case .deviceRegistered(let event):
            devices[event.device] = Device(
                id: event.device, kind: event.kind, name: event.name, dailyKey: event.dailyKey,
                approvalKey: event.approvalKey, at: event.atMs)
        case .intentEchoed(let event): echoed(event, id)
        case .unreadable(let type, let text):
            // It may have been anything, so it is shown where the user looks first.
            show(Row(id: id, at: log.last?.at ?? 0, kind: .unreadable(type: type, text: text)), thread: nil, dispatcher: true)
        }
    }

    // MARK: Messages

    private mutating func queued(_ wire: WireMessage, _ id: RowID) {
        let message = Message(
            id: wire.id, from: wire.from, to: wire.to, kind: wire.kind, thread: wire.thread, hop: wire.hop,
            replyTo: wire.replyTo, card: wire.task, body: wire.body, files: wire.files ?? [], at: wire.atMs,
            device: wire.device, seq: id.seq, state: .queued)
        messages[message.id] = message
        if let original = message.replyTo {
            messages[original]?.state = .answered(by: message.id)
            openQuestions.remove(original)
        }
        if message.isToUser {
            messagesToUser.append(message.id)
            if message.kind == "question" { openQuestions.insert(message.id) }
        }
        var thread = threads[message.thread] ?? Line(id: message.thread)
        threads[message.thread] = nil
        thread.messages.append(message.id)
        for member in [message.from, message.to] where !thread.members.contains(member) { thread.members.append(member) }
        thread.deepestHop = max(thread.deepestHop, message.hop)
        thread.pairs[.init(message.from, message.to), default: 0] += 1
        threads[message.thread] = thread
        show(
            Row(id: id, at: message.at, kind: .message(message.id)), thread: message.thread,
            dispatcher: message.isFromUser || message.isToUser)
    }

    private mutating func failed(_ event: Event.MessageFailed, _ id: RowID) {
        guard let message = messages[event.id] else { return }
        messages[event.id]?.state = .failed(event.why)
        openQuestions.remove(event.id)
        touch(line: message.thread, at: event.atMs)
    }

    // MARK: Cards

    private mutating func created(_ event: Event.CardCreated, _ id: RowID) {
        touch(event.card) {
            $0.title = event.title
            $0.instructions = event.instructions
            $0.acceptance = event.acceptance
            $0.conventions = event.conventions
            $0.author = event.author
            $0.vendor = event.vendor
            $0.reviewer = event.reviewer
            $0.continues = event.continues
            $0.thread = event.thread
            $0.createdAt = event.atMs
        }
        // A rework without a line of its own yet is shown where the card it reworks is.
        let thread = event.thread ?? event.continues.flatMap { cards[$0]?.thread }
        show(Row(id: id, at: event.atMs, kind: .card(event.card)), thread: thread, dispatcher: true)
    }

    private mutating func started(_ event: Event.CardStarted, _ id: RowID) {
        touch(event.card) {
            $0.work = .init(
                worker: event.worker, worktree: event.worktree, branch: event.branch, base: event.base,
                task: event.task, freshStart: event.freshStart, at: event.atMs)
            // Without a line of its own the card starts one, and its TASK is that line's first letter.
            if $0.thread == nil { $0.thread = event.task }
        }
        cardsByTask[event.task] = event.card
        vendors[event.worker] = cards[event.card]?.vendor
        show(Row(id: id, at: event.atMs, kind: .cardStarted(event.card)), thread: cards[event.card]?.thread)
    }

    private mutating func touch(_ card: String, _ change: (inout Card) -> Void) {
        if cards[card] == nil { cardOrder.append(card) }
        change(&cards[card, default: Card(id: card)])
    }

    private mutating func showReview(of card: String, at: UInt64, _ id: RowID) {
        guard let count = cards[card]?.reviews.count else { return }
        show(Row(id: id, at: at, kind: .review(card: card, index: count - 1)), thread: cards[card]?.thread)
    }

    private mutating func endReview(of card: String, _ outcome: Card.Review.Outcome, at: UInt64, _ id: RowID) {
        var isNew = false
        touch(card) {
            // The review that is still open; if there is none, its start was never seen.
            if let last = $0.reviews.indices.last, $0.reviews[last].outcome == nil {
                $0.reviews[last].outcome = outcome
            } else {
                $0.reviews.append(.init(reviewer: nil, task: nil, startedAt: nil, outcome: outcome))
                isNew = true
            }
        }
        if isNew {
            showReview(of: card, at: at, id)
        } else {
            touch(line: cards[card]?.thread, at: at)
        }
    }

    // MARK: Tests and merges

    private mutating func test(_ test: String, _ change: (inout TestRun) -> Void) {
        change(&tests[test, default: TestRun(id: test)])
    }

    private mutating func merge(_ merge: String, _ change: (inout Merge) -> Void) {
        change(&merges[merge, default: Merge(id: merge)])
    }

    private mutating func step(
        _ name: String, _ stage: Merge.Stage, _ at: UInt64, _ id: RowID, _ change: (inout Merge) -> Void = { _ in }
    ) {
        merge(name) {
            $0.stage = stage
            change(&$0)
        }
        touch(line: threadOf(merge: name), at: at)
    }

    private func threadOf(test: String) -> String? { tests[test]?.card.flatMap { cards[$0]?.thread } }
    private func threadOf(merge: String) -> String? { merges[merge]?.card.flatMap { cards[$0]?.thread } }

    // MARK: Intents

    private mutating func echoed(_ event: Event.IntentEchoed, _ id: RowID) {
        echoes[event.digest] = Echo(
            device: event.device, number: event.number, digest: event.digest, contentID: event.contentId,
            outcome: event.outcome, at: event.atMs, seq: id.seq)
        devices[event.device]?.lastIntentAt = event.atMs
        devices[event.device]?.intents += 1
        if case .refused = event.outcome { devices[event.device]?.refusals += 1 }
    }

    // MARK: Rows

    /// On the pages it belongs to.
    private mutating func show(_ row: Row, thread: String?, dispatcher: Bool = false) {
        if let thread {
            var line = threads[thread] ?? Line(id: thread)
            threads[thread] = nil  // So that appending to its rows does not copy them.
            line.rows.append(row)
            line.lastAt = max(line.lastAt, row.at)
            threads[thread] = line
        }
        if dispatcher { dispatcherRows.append(row) }
    }

    /// Something on it moved on. The row already there shows the new state.
    private mutating func touch(line thread: String?, at: UInt64) {
        if let thread, let last = threads[thread]?.lastAt { threads[thread]?.lastAt = max(last, at) }
    }

    // MARK: Waiting for the user

    /// What the user is being waited on for, longest wait first.
    public var waiting: [Waiting] {
        var items: [Waiting] = []
        items += openAsks.compactMap { asks[$0] }.map { Waiting(at: $0.at, what: .ask($0.id)) }
        items += openQuestions.compactMap { messages[$0] }.map { Waiting(at: $0.at, what: .question($0.id)) }
        items += trips.values.map { Waiting(at: $0.at, what: .trip($0.id)) }
        return items.sorted { ($0.at, $0.sortKey) < ($1.at, $1.sortKey) }
    }
}

public struct Waiting: Equatable, Hashable, Sendable {
    public let at: UInt64
    public let what: What

    public enum What: Equatable, Hashable, Sendable {
        case ask(String)
        case question(String)
        /// LMAS never clears these; the app remembers which ones the user has put away.
        case trip(TripID)
    }

    fileprivate var sortKey: String {
        switch what {
        case .ask(let id): "a\(id)"
        case .question(let id): "q\(id)"
        case .trip(let id): "t\(id.seq).\(id.index)"
        }
    }
}
