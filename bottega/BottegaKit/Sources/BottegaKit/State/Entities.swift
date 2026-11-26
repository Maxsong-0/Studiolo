// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.

/// A member of the bus. The same name can open again after closing.
public struct Session: Equatable, Sendable, Identifiable {
    public let name: String
    public internal(set) var role: String
    public internal(set) var openedAt: UInt64
    public internal(set) var closedAt: UInt64?

    public var id: String { name }
    public var isOpen: Bool { closedAt == nil }

    /// The user, as a member of the bus.
    public static let user = "user"
    /// LMAS itself, when it writes a letter (reminders, a line's history for the next agent).
    public static let coordinator = "coordinator"
}

public struct Message: Equatable, Sendable, Identifiable {
    public let id: String
    public let from: String
    public let to: String
    public let kind: String
    /// The line it is on.
    public let thread: String
    public let hop: Int
    public let replyTo: String?
    /// The card the sender says it is about. Unchecked by LMAS.
    public let card: String?
    /// Written by whoever sent it: show it as data.
    public let body: String
    public let files: [TextFile]
    public let at: UInt64
    /// The device the user said it through, signature checked by LMAS.
    public let device: String?
    /// The number of the record it came in.
    public let seq: UInt64
    public internal(set) var state: State
    /// Kept when it has been answered since.
    public internal(set) var handedAt: UInt64?

    public enum State: Equatable, Sendable {
        case queued
        case handed(at: UInt64)
        case answered(by: String)
        case failed(Why)
    }

    public var isOpenQuestion: Bool {
        guard kind == "question" else { return false }
        return switch state {
        case .queued, .handed: true
        case .answered, .failed: false
        }
    }

    public var isFromUser: Bool { device != nil || from == Session.user }
    public var isToUser: Bool { to == Session.user }
}

/// An agent's turn: from taking up work to stopping. What went on inside is kept apart (`TurnDetails`).
public struct Turn: Equatable, Sendable, Identifiable {
    public let id: String
    public let session: String
    public internal(set) var vendor: String?
    public internal(set) var model: String?
    public internal(set) var effort: String?
    public internal(set) var cliVersion: String?
    /// The letters pushed into it.
    public internal(set) var messages: [String] = []
    /// nil when its start was never seen.
    public internal(set) var startedAt: UInt64?
    public internal(set) var end: End?
    public internal(set) var stop: Stop?

    /// The user wanted it stopped. What came of it shows in `end`, unless it went on.
    public enum Stop: Equatable, Sendable {
        case requested(at: UInt64)
        case failed(at: UInt64)
    }

    public struct End: Equatable, Sendable {
        /// `completed`, `failed`, `stopped`, `stopped_by_user`, `process_exited`, `timed_out`, `restarted`…
        public let outcome: Why
        public let modelTurns: Int
        public let tools: [String: Int]
        public let denied: Int
        public let text: String?
        public let at: UInt64
    }

    public var isRunning: Bool { end == nil }

    init(id: String, session: String) {
        self.id = id
        self.session = session
    }

    func wasRunning(at moment: UInt64) -> Bool {
        guard let startedAt else { return false }
        return moment >= startedAt && moment <= end?.at ?? .max
    }
}

/// A line: one task chain, named by its first message.
public struct Line: Equatable, Sendable, Identifiable {
    public let id: String
    public internal(set) var messages: [String] = []
    /// Everyone who sent or got something on it, in the order they turned up.
    public internal(set) var members: [String] = []
    public internal(set) var deepestHop = 0
    public internal(set) var lastAt: UInt64 = 0
    /// Rows of its timeline.
    public internal(set) var rows: [Row] = []
    /// Letters between each pair, whichever way they went.
    public internal(set) var pairs: [Pair: Int] = [:]

    /// Two members, in a fixed order, so both directions count together.
    public struct Pair: Hashable, Sendable {
        public let one, other: String
        public init(_ a: String, _ b: String) {
            (one, other) = a < b ? (a, b) : (b, a)
        }
    }

    public var busiestPair: (pair: Pair, count: Int)? {
        // Ties go to the pair that sorts first, so the answer does not flicker between redraws.
        pairs.max { ($0.value, $1.key.one, $1.key.other) < ($1.value, $0.key.one, $0.key.other) }
            .map { ($0.key, $0.value) }
    }
}

/// One row of a timeline. Rows point at what they show; what they show keeps changing underneath.
public struct Row: Equatable, Sendable, Identifiable {
    public let id: RowID
    public let at: UInt64
    public let kind: Kind

    public enum Kind: Equatable, Sendable {
        case message(String)
        /// The card, live.
        case card(String)
        case cardStarted(String)
        case cardEnded(String)
        /// Back in the queue after its AI stopped halfway. Index into the card's requeues: a card can go back more than
        /// once, and each row tells its own time.
        case cardRequeued(card: String, index: Int)
        /// The card, or its review, went to another vendor. Index into the card's reassignments.
        case reassigned(card: String, index: Int)
        /// A quota gate changed: the key into `gateChanges`.
        case gate(RowID)
        /// LMAS reminded someone of a question the card's AI asked. Index into the card's reminders.
        case answerReminded(card: String, index: Int)
        case test(String)
        /// Index into the card's reviews.
        case review(card: String, index: Int)
        case merge(String)
        case ask(String)
        case trip(TripID)
        case unreadable(type: String, text: String?)
    }
}

/// One event of the log.
public struct Happening: Equatable, Sendable, Identifiable {
    public let id: RowID
    public let at: UInt64
    public let event: Event
}

/// Where an event stands in the log: its record's number, and which of the record's events it is.
public struct RowID: Hashable, Comparable, Sendable {
    public let seq: UInt64
    public let index: Int

    public init(seq: UInt64, index: Int) {
        self.seq = seq
        self.index = index
    }

    public static func < (one: RowID, other: RowID) -> Bool { (one.seq, one.index) < (other.seq, other.index) }
}

public struct Card: Equatable, Sendable, Identifiable {
    public let id: String
    public internal(set) var title: String?
    public internal(set) var instructions: String?
    public internal(set) var acceptance: String?
    public internal(set) var conventions: String?
    public internal(set) var author: String?
    public internal(set) var vendor: String?
    public internal(set) var reviewer: String?
    /// The card this one reworks.
    public internal(set) var continues: String?
    public internal(set) var thread: String?
    public internal(set) var createdAt: UInt64?
    public internal(set) var work: Work?
    public internal(set) var end: End?
    public internal(set) var tests: [String] = []
    public internal(set) var reviews: [Review] = []
    public internal(set) var merges: [String] = []
    /// Each time its AI stopped halfway and the card went back to wait, oldest first.
    public internal(set) var requeues: [Requeue] = []
    /// Its writer named no vendor: while a gate is not open, it may go to another one.
    public internal(set) var isVendorFree = false
    /// Each time it, or its review, went to another vendor before starting, oldest first.
    public internal(set) var reassignments: [Reassignment] = []
    /// The cards its writer said it waits for before it first starts. Whether they have landed is LMAS's call.
    public internal(set) var after: [String] = []
    /// Each reminder LMAS sent about a question its AI asked, oldest first.
    public internal(set) var reminders: [Reminder] = []

    public struct Work: Equatable, Sendable {
        public let worker, worktree, branch, base, task: String
        public let gitDir: String?
        public let freshStart: Event.FreshStart?
        public let at: UInt64
    }

    public struct Requeue: Equatable, Sendable {
        public let action: String
        public let why: Why
        public let at: UInt64
    }

    public struct Reminder: Equatable, Sendable {
        /// The letter that asked.
        public let question: String
        public let level: UInt64
        public let at: UInt64

        public init(question: String, level: UInt64, at: UInt64) {
            self.question = question
            self.level = level
            self.at = at
        }
    }

    public struct Reassignment: Equatable, Sendable {
        public enum What: Equatable, Sendable { case card, review }
        public let what: What
        /// The vendor it had, if known.
        public let from: String?
        public let to: String
        /// For a card: the reviewer picked again with it.
        public let reviewer: String?
        public let at: UInt64

        public init(what: What, from: String?, to: String, reviewer: String?, at: UInt64) {
            self.what = what
            self.from = from
            self.to = to
            self.reviewer = reviewer
            self.at = at
        }
    }

    public enum End: Equatable, Sendable {
        case done(result: String, at: UInt64)
        case failed(Why, at: UInt64)
    }

    public struct Review: Equatable, Sendable {
        public let reviewer: String?
        public let task: String?
        public let startedAt: UInt64?
        public internal(set) var outcome: Outcome?

        public enum Outcome: Equatable, Sendable {
            case finished(verdict: String, result: String, at: UInt64)
            case notDone(Why, at: UInt64)
            /// The reviewer stopped halfway; a fresh one reviews from the start (another `review_started`).
            case requeued(action: String, Why, at: UInt64)
        }
    }

    init(id: String) { self.id = id }
}

/// A run of tests on the testbox.
public struct TestRun: Equatable, Sendable, Identifiable {
    public let id: String
    public internal(set) var card: String?
    public internal(set) var by: String?
    public internal(set) var request: Event.TestRequest?
    public internal(set) var merge: String?
    public internal(set) var submittedAt: UInt64?
    public internal(set) var job: String?
    public internal(set) var stage: Stage = .submitted

    public enum Stage: Equatable, Sendable {
        case submitted
        case handedOver(at: UInt64)
        case accepted(at: UInt64)
        case finished(Why, report: Event.TestReport?, at: UInt64)
    }

    init(id: String) { self.id = id }
}

public struct Merge: Equatable, Sendable, Identifiable {
    public let id: String
    public internal(set) var card: String?
    public internal(set) var by: String?
    public internal(set) var target: String?
    public internal(set) var test: Event.TestRequest?
    public internal(set) var gaps: [Why] = []
    public internal(set) var asksUser = false
    public internal(set) var requestedAt: UInt64?
    public internal(set) var size: Size?
    public internal(set) var build: Build?
    public internal(set) var restarts: [Why] = []
    public internal(set) var stage: Stage = .requested
    public internal(set) var cleaned = false

    public struct Size: Equatable, Sendable {
        public let files: Int
        public let insertions, deletions: UInt64
    }

    public struct Build: Equatable, Sendable {
        public let onto, commit, tree, branch, worktree: String
    }

    public enum Stage: Equatable, Sendable {
        case requested
        case checked(at: UInt64)
        case prepared(at: UInt64)
        /// Put back to the start: the target moved, or LMAS stopped halfway.
        case restarted(Why, at: UInt64)
        case landing(at: UInt64)
        case finished(at: UInt64)
        case failed(Why, at: UInt64)
    }

    public var isOver: Bool {
        switch stage {
        case .finished, .failed: true
        case .requested, .checked, .prepared, .restarted, .landing: false
        }
    }

    init(id: String) { self.id = id }
}

/// A question to the user that LMAS itself waits on.
public struct Ask: Equatable, Sendable, Identifiable {
    public let id: String
    public let about: Event.AskAbout
    public let by: String
    /// Shown exactly as it is. What an approval signs for is this text's digest.
    public let text: String
    /// Only a device's approval key counts; not the dialog, not the Planner.
    public let onlyDevice: Bool
    public let at: UInt64
    public internal(set) var answer: Answer?

    /// Of the bytes LMAS sent. Also how to tell whether two texts are the same: `==` on strings calls texts equal
    /// that Unicode merely writes two ways, and LMAS does not.
    public var textDigest: String { Digest.sha256(text) }

    public struct Answer: Equatable, Sendable {
        /// `approved`, `declined`, `withdrawn`.
        public let word: String
        public let via: Event.AskVia
        public let at: UInt64
    }
}

/// Which limit trip: they have no number of their own.
public typealias TripID = RowID

/// A letter held back because it ran into a limit. It was not sent.
public struct Trip: Equatable, Sendable, Identifiable {
    public let id: TripID
    /// `hop`, `budget`, `pair`.
    public let limit: String
    public let thread: String?
    public let from, to, kind: String
    public let at: UInt64
}

public struct Device: Equatable, Sendable, Identifiable {
    public let id: String
    public let kind, name: String
    public let dailyKey, approvalKey: String
    public let at: UInt64
    /// The last intent of its that LMAS handled.
    public internal(set) var lastIntentAt: UInt64?
    public internal(set) var intents = 0
    public internal(set) var refusals = 0
}

/// What became of an intent, found by its digest.
public struct Echo: Equatable, Sendable {
    public let device: String
    public let number: UInt64
    public let digest, contentID: String
    public let outcome: Outcome
    public let at: UInt64
    /// The number of the record it is in.
    public let seq: UInt64
}
