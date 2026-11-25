// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.

/// One thing that happened, as LMAS tells it (its `view.rs`, `UiEvent`). Words that may gain values (`kind`,
/// `role`, `code`, `limit`…) stay strings: an unknown word is shown, not refused.
public enum Event: Equatable, Sendable {
    case sessionRegistered(SessionRegistered)
    case sessionClosed(SessionClosed)
    case messageQueued(MessageQueued)
    case messageHanded(MessageHanded)
    case messageFailed(MessageFailed)
    case cardCreated(CardCreated)
    case cardStarted(CardStarted)
    case cardDone(CardDone)
    case cardFailed(CardFailed)
    case limitTripped(LimitTripped)
    case testsSubmitted(TestsSubmitted)
    case testsHandedOver(TestsHandedOver)
    case testsAccepted(TestsAccepted)
    case testsFinished(TestsFinished)
    case reviewStarted(ReviewStarted)
    case reviewFinished(ReviewFinished)
    case reviewNotDone(ReviewNotDone)
    case mergeRequested(MergeRequested)
    case mergeChecked(MergeChecked)
    case mergePrepared(MergePrepared)
    case mergeRestarted(MergeRestarted)
    case mergeLanding(MergeLanding)
    case mergeFinished(MergeFinished)
    case mergeFailed(MergeFailed)
    case mergeCleaned(MergeCleaned)
    case askOpened(AskOpened)
    case askAnswered(AskAnswered)
    case deviceRegistered(DeviceRegistered)
    case intentEchoed(IntentEchoed)
    case turnStarted(TurnStarted)
    case turnFinished(TurnFinished)
    /// A `type` this Bottega does not know, or one it knows in a shape it cannot read. The record still counts.
    case unreadable(type: String, text: String?)

    public struct SessionRegistered: Decodable, Equatable, Sendable { public var name, role: String; public var atMs: UInt64 }
    public struct SessionClosed: Decodable, Equatable, Sendable { public var name: String; public var atMs: UInt64 }
    public struct MessageQueued: Decodable, Equatable, Sendable { public var message: WireMessage }
    public struct MessageHanded: Decodable, Equatable, Sendable { public var id: String; public var atMs: UInt64 }
    public struct MessageFailed: Decodable, Equatable, Sendable { public var id: String; public var why: Why; public var atMs: UInt64 }

    public struct CardCreated: Decodable, Equatable, Sendable {
        public var card, author, title, instructions, acceptance: String
        public var conventions, vendor, continues, reviewer, thread: String?
        public var atMs: UInt64
    }
    public struct CardStarted: Decodable, Equatable, Sendable {
        public var card, worker, worktree, branch, base: String
        public var freshStart: FreshStart?
        public var task: String
        public var atMs: UInt64
    }
    /// Rework that starts over on the target branch instead of on the old card's commit.
    public struct FreshStart: Decodable, Equatable, Sendable {
        public var target: String
        public var conflicts: Int
        public var named: [String]?
    }
    public struct CardDone: Decodable, Equatable, Sendable { public var card, result: String; public var atMs: UInt64 }
    public struct CardFailed: Decodable, Equatable, Sendable { public var card: String; public var why: Why; public var atMs: UInt64 }

    public struct LimitTripped: Decodable, Equatable, Sendable {
        public var limit: String
        public var thread: String?
        public var from, to, kind: String
        public var atMs: UInt64
    }

    public struct TestRequest: Decodable, Equatable, Sendable {
        public var command, tier: String
        public var memoryGib, cpu: Int
    }
    public struct TestReport: Decodable, Equatable, Sendable {
        public var exitCode: Int?
        public var durationMs: UInt64?
        public var log: String?
        public var logBytes: UInt64
        public var truncated: Bool
        public var tail: String
    }
    public struct TestsSubmitted: Decodable, Equatable, Sendable {
        public var test, card, by: String
        public var request: TestRequest
        public var merge: String?
        public var atMs: UInt64
    }
    public struct TestsHandedOver: Decodable, Equatable, Sendable { public var test: String; public var atMs: UInt64 }
    public struct TestsAccepted: Decodable, Equatable, Sendable { public var test, job: String; public var atMs: UInt64 }
    public struct TestsFinished: Decodable, Equatable, Sendable {
        public var test: String
        public var outcome: Why
        public var report: TestReport?
        public var atMs: UInt64
    }

    public struct ReviewStarted: Decodable, Equatable, Sendable { public var card, reviewer, base, tree, task: String; public var atMs: UInt64 }
    public struct ReviewFinished: Decodable, Equatable, Sendable { public var card, verdict, result: String; public var atMs: UInt64 }
    public struct ReviewNotDone: Decodable, Equatable, Sendable { public var card: String; public var why: Why; public var atMs: UInt64 }

    public struct MergeRequested: Decodable, Equatable, Sendable {
        public var merge, card, by, target: String
        public var test: TestRequest?
        public var gaps: [Why]?
        public var asksUser: Bool
        public var atMs: UInt64
    }
    public struct MergeChecked: Decodable, Equatable, Sendable {
        public var merge, tree, base: String
        public var files: Int
        public var insertions, deletions: UInt64
        public var atMs: UInt64
    }
    public struct MergePrepared: Decodable, Equatable, Sendable {
        public var merge, onto, commit, tree, branch, worktree: String
        public var atMs: UInt64
    }
    public struct MergeRestarted: Decodable, Equatable, Sendable { public var merge: String; public var why: Why; public var atMs: UInt64 }
    public struct MergeLanding: Decodable, Equatable, Sendable { public var merge: String; public var atMs: UInt64 }
    public struct MergeFinished: Decodable, Equatable, Sendable { public var merge: String; public var atMs: UInt64 }
    public struct MergeFailed: Decodable, Equatable, Sendable { public var merge: String; public var why: Why; public var atMs: UInt64 }
    public struct MergeCleaned: Decodable, Equatable, Sendable { public var merge: String; public var atMs: UInt64 }

    public struct AskOpened: Decodable, Equatable, Sendable {
        public var ask: String
        public var about: AskAbout
        public var by, text: String
        public var onlyDevice: Bool?
        public var atMs: UInt64
    }
    /// `kind` is `merge` (with the merge's number) or `question`; later kinds are shown by their text alone.
    public struct AskAbout: Decodable, Equatable, Sendable { public var kind: String; public var merge: String? }
    public struct AskAnswered: Decodable, Equatable, Sendable {
        public var ask, answer: String
        public var via: AskVia
        public var atMs: UInt64
    }
    /// `at` is `session` (with `name`), `dialog`, `coordinator` or `device` (with `device`).
    public struct AskVia: Decodable, Equatable, Sendable { public var at: String; public var name, device: String? }

    public struct DeviceRegistered: Decodable, Equatable, Sendable {
        public var device, kind, name, dailyKey, approvalKey: String
        public var atMs: UInt64
    }

    public struct IntentEchoed: Decodable, Equatable, Sendable {
        public var device: String
        public var number: UInt64
        public var digest, contentId: String
        public var outcome: Outcome
        public var atMs: UInt64
    }

    /// An agent took up work. What goes on inside comes separately (`Live`, `TurnPage`).
    public struct TurnStarted: Decodable, Equatable, Sendable {
        public var turn, session, vendor: String
        public var model, cliVersion: String?
        /// The letters pushed into it. Those it fetched itself are not here.
        public var messages: [String]?
        public var atMs: UInt64
    }
    /// Every start has one.
    public struct TurnFinished: Decodable, Equatable, Sendable {
        public var turn, session: String
        public var outcome: Why
        public var modelTurns: Int
        /// How often each tool was used, by the vendor's own names.
        public var tools: [String: Int]?
        public var denied: Int
        /// The last it said, its beginning if that is long.
        public var text: String?
        public var atMs: UInt64
    }
}

/// A message as it travels: `kind` is `task`, `question`, `result` or `notification`; `thread` is the number of
/// the line's first message.
public struct WireMessage: Decodable, Equatable, Sendable {
    public var id, from, to, kind, thread: String
    public var hop: Int
    public var replyTo, task: String?
    public var body: String
    public var files: [TextFile]?
    public var atMs: UInt64
    /// Set when the user said it: through which device, signature already checked by LMAS.
    public var device: String?
}

/// What became of an intent.
public enum Outcome: Decodable, Equatable, Sendable {
    /// Done. `message` is the one it became, when it became one.
    case done(message: String?)
    /// This content was already done, by the device's intent `number`. Not done twice.
    case already(number: UInt64)
    case refused(Why)
    /// A result this Bottega does not know.
    case other(String)

    private enum Key: CodingKey { case result, message, number, why }

    public init(from decoder: any Decoder) throws {
        let fields = try decoder.container(keyedBy: Key.self)
        switch try fields.decode(String.self, forKey: .result) {
        case "done": self = .done(message: try fields.decodeIfPresent(String.self, forKey: .message))
        case "already": self = .already(number: try fields.decode(UInt64.self, forKey: .number))
        case "refused": self = .refused(try fields.decode(Why.self, forKey: .why))
        case let other: self = .other(other)
        }
    }
}

extension Event: Decodable {
    private enum Key: CodingKey { case type, text }

    /// Never throws: one event nobody can read must not cost the ones beside it.
    public init(from decoder: any Decoder) {
        let fields = try? decoder.container(keyedBy: Key.self)
        let type = (try? fields?.decodeIfPresent(String.self, forKey: .type)) ?? ""
        do {
            self = try Self.known(type, decoder) ?? .unreadable(type: type, text: Self.text(fields))
        } catch {
            self = .unreadable(type: type, text: Self.text(fields))
        }
    }

    private static func text(_ fields: KeyedDecodingContainer<Key>?) -> String? {
        (try? fields?.decodeIfPresent(String.self, forKey: .text)) ?? nil
    }

    private static func known(_ type: String, _ decoder: any Decoder) throws -> Event? {
        switch type {
        case "session_registered": .sessionRegistered(try .init(from: decoder))
        case "session_closed": .sessionClosed(try .init(from: decoder))
        case "message_queued": .messageQueued(try .init(from: decoder))
        case "message_handed": .messageHanded(try .init(from: decoder))
        case "message_failed": .messageFailed(try .init(from: decoder))
        case "card_created": .cardCreated(try .init(from: decoder))
        case "card_started": .cardStarted(try .init(from: decoder))
        case "card_done": .cardDone(try .init(from: decoder))
        case "card_failed": .cardFailed(try .init(from: decoder))
        case "limit_tripped": .limitTripped(try .init(from: decoder))
        case "tests_submitted": .testsSubmitted(try .init(from: decoder))
        case "tests_handed_over": .testsHandedOver(try .init(from: decoder))
        case "tests_accepted": .testsAccepted(try .init(from: decoder))
        case "tests_finished": .testsFinished(try .init(from: decoder))
        case "review_started": .reviewStarted(try .init(from: decoder))
        case "review_finished": .reviewFinished(try .init(from: decoder))
        case "review_not_done": .reviewNotDone(try .init(from: decoder))
        case "merge_requested": .mergeRequested(try .init(from: decoder))
        case "merge_checked": .mergeChecked(try .init(from: decoder))
        case "merge_prepared": .mergePrepared(try .init(from: decoder))
        case "merge_restarted": .mergeRestarted(try .init(from: decoder))
        case "merge_landing": .mergeLanding(try .init(from: decoder))
        case "merge_finished": .mergeFinished(try .init(from: decoder))
        case "merge_failed": .mergeFailed(try .init(from: decoder))
        case "merge_cleaned": .mergeCleaned(try .init(from: decoder))
        case "ask_opened": .askOpened(try .init(from: decoder))
        case "ask_answered": .askAnswered(try .init(from: decoder))
        case "device_registered": .deviceRegistered(try .init(from: decoder))
        case "intent_echoed": .intentEchoed(try .init(from: decoder))
        case "turn_started": .turnStarted(try .init(from: decoder))
        case "turn_finished": .turnFinished(try .init(from: decoder))
        default: nil
        }
    }
}

extension Event {
    /// LMAS's clock when it happened. Every known event has one.
    public var at: UInt64? {
        switch self {
        case .sessionRegistered(let event): event.atMs
        case .sessionClosed(let event): event.atMs
        case .messageQueued(let event): event.message.atMs
        case .messageHanded(let event): event.atMs
        case .messageFailed(let event): event.atMs
        case .cardCreated(let event): event.atMs
        case .cardStarted(let event): event.atMs
        case .cardDone(let event): event.atMs
        case .cardFailed(let event): event.atMs
        case .limitTripped(let event): event.atMs
        case .testsSubmitted(let event): event.atMs
        case .testsHandedOver(let event): event.atMs
        case .testsAccepted(let event): event.atMs
        case .testsFinished(let event): event.atMs
        case .reviewStarted(let event): event.atMs
        case .reviewFinished(let event): event.atMs
        case .reviewNotDone(let event): event.atMs
        case .mergeRequested(let event): event.atMs
        case .mergeChecked(let event): event.atMs
        case .mergePrepared(let event): event.atMs
        case .mergeRestarted(let event): event.atMs
        case .mergeLanding(let event): event.atMs
        case .mergeFinished(let event): event.atMs
        case .mergeFailed(let event): event.atMs
        case .mergeCleaned(let event): event.atMs
        case .askOpened(let event): event.atMs
        case .askAnswered(let event): event.atMs
        case .deviceRegistered(let event): event.atMs
        case .intentEchoed(let event): event.atMs
        case .turnStarted(let event): event.atMs
        case .turnFinished(let event): event.atMs
        case .unreadable: nil
        }
    }
}

/// Everything LMAS decided at once: all of it happened, or none of it.
public struct Record: Decodable, Equatable, Sendable {
    /// The number of its first event; one more than the previous record's `through`.
    public var seq: UInt64
    /// The number of its last event. Internal events take numbers too, so this says nothing about `events.count`.
    public var through: UInt64
    public var events: [Event]

    public init(seq: UInt64, through: UInt64, events: [Event]) {
        self.seq = seq
        self.through = through
        self.events = events
    }

    private enum Key: CodingKey { case seq, through, events }

    public init(from decoder: any Decoder) throws {
        let fields = try decoder.container(keyedBy: Key.self)
        seq = try fields.decodeNumber(from: 1, forKey: .seq)
        through = try fields.decodeNumber(from: 1, forKey: .through)
        events = try fields.decode([Event].self, forKey: .events)
    }
}
