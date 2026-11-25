// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.

/// What the user asked for and LMAS has not settled yet, in order. One intent is in the air at a time: the next one
/// takes its number only after the one before it is settled, so this device never leaves a gap.
///
/// Pure bookkeeping. What each answer means is decided here; sending, signing and keeping it on disk is the link's
/// job. The rules are section 7 of the protocol.
public struct Outbox: Codable, Equatable, Sendable {
    public struct Item: Codable, Equatable, Identifiable, Sendable {
        /// The content id. Stays the same however often the item is numbered again.
        public let id: String
        public let draft: IntentDraft
        /// The user's clock, for showing.
        public let addedAt: UInt64
        /// Once signed, resent byte for byte.
        public fileprivate(set) var signed: SignedIntent?
        fileprivate var renumberings = 0
        fileprivate var retimed = false
    }

    /// How an item ended.
    public struct Settled: Equatable, Sendable {
        public let item: Item
        public let result: Result

        public enum Result: Equatable, Sendable {
            case done(message: String?)
            /// Done before, by this device's intent of this number.
            case already(number: UInt64)
            /// LMAS handled it and did not do it.
            case refused(Why)
            /// It never became an intent LMAS handled.
            case notSent(Why)
        }

        /// The words go back into the composer: nothing was done with them.
        public var isUndone: Bool {
            switch result {
            case .done, .already: false
            case .refused, .notSent: true
            }
        }

        public init(item: Item, result: Result) {
            self.item = item
            self.result = result
        }
    }

    /// What to do after an answer.
    public enum Step: Equatable, Sendable {
        case settled(Settled)
        /// Sign the same content again, under this number, with this much added to the clock.
        case signAgain(number: UInt64, clockOffset: Int64)
        /// Fetch events up to the newest, then ask `resolveConflict`.
        case catchUp(next: UInt64)
    }

    public private(set) var items: [Item] = []

    /// After this many new numbers something is wrong that another number will not mend.
    static let maxRenumberings = 3

    public init() {}

    public var head: Item? { items.first }
    public var isEmpty: Bool { items.isEmpty }

    public mutating func add(_ draft: IntentDraft, id: String, at: UInt64) {
        items.append(Item(id: id, draft: draft, addedAt: at))
    }

    /// The head has been signed. Keep it on disk before sending it.
    public mutating func sign(_ intent: SignedIntent) {
        guard !items.isEmpty else { return }
        items[0].signed = intent
    }

    /// The head's signature never left this Mac and will not.
    public mutating func unsign() {
        guard !items.isEmpty else { return }
        items[0].signed = nil
    }

    /// An item LMAS cannot have: taken out, nothing happens to it.
    public mutating func withdraw(_ id: String) -> Item? {
        guard let index = items.firstIndex(where: { $0.id == id }), items[index].signed == nil else { return nil }
        return items.remove(at: index)
    }

    /// Without a connection, or after a restart. A decision that was never signed is not remembered: a Touch ID
    /// prompt must come from a click, not from LMAS coming back.
    public mutating func dropUnsignedDecisions() -> [Settled] {
        var dropped: [Settled] = []
        items.removeAll { item in
            guard item.signed == nil, item.draft.typed == nil else { return false }
            dropped.append(Settled(item: item, result: .notSent(.notSigned)))
            return true
        }
        return dropped
    }

    /// An item was not signed: the user cancelled, or the key would not. nil when it is no longer waiting.
    public mutating func notSigned(_ id: String, _ why: Why = .notSigned) -> Settled? {
        withdraw(id).map { Settled(item: $0, result: .notSent(why)) }
    }

    /// LMAS handled the head.
    public mutating func take(_ outcome: Outcome, at: UInt64, now: UInt64) -> Step? {
        guard let item = items.first, let number = item.signed?.number else { return nil }
        // This Mac's clock runs ahead of LMAS's. Once, set it back by what the echo shows and sign anew; the number
        // was used up.
        if case .refused(let why) = outcome, why.code == "in_future", !item.retimed, item.draft.key == .daily {
            items[0].retimed = true
            items[0].signed = nil
            return .signAgain(number: number + 1, clockOffset: Int64(clamping: at) - Int64(clamping: now))
        }
        return settle(result(of: outcome))
    }

    /// LMAS did not get as far as handling the head; it used up no number.
    public mutating func take(_ error: WireError) -> Step? {
        guard !items.isEmpty else { return nil }
        switch error.code {
        case "gap":
            guard let next = error.next else { return settle(.notSent(.init(code: error.code, text: error.text))) }
            return renumber(next, because: error)
        case "conflict":
            guard let next = error.next else { return settle(.notSent(.init(code: error.code, text: error.text))) }
            return .catchUp(next: next)
        default:
            // `number_too_old`: the books no longer go back that far, so nobody can say whether it was done. The
            // user gets the words back and decides. Everything else (`dropped`, and codes to come): not done.
            return settle(.notSent(.init(code: error.code, text: error.text)))
        }
    }

    /// The events are in. If one of them echoes exactly what was signed, it was done; if not, another number.
    public mutating func resolveConflict(next: UInt64, echo: Outcome?) -> Step? {
        if let echo { return settle(result(of: echo)) }
        return renumber(next, because: WireError(code: "conflict", text: "Another intent already used this number."))
    }

    private func result(of outcome: Outcome) -> Settled.Result {
        switch outcome {
        case .done(let message): .done(message: message)
        case .already(let number): .already(number: number)
        case .refused(let why): .refused(why)
        case .other(let word): .refused(.init(code: word, text: "LMAS answered “\(word)”, which this Bottega does not know."))
        }
    }

    private mutating func renumber(_ next: UInt64, because error: WireError) -> Step? {
        // Signing an approval again means Touch ID again, which only the user starts.
        if items[0].draft.key == .approval || items[0].renumberings >= Self.maxRenumberings {
            return settle(.notSent(.init(code: error.code, text: error.text)))
        }
        items[0].renumberings += 1
        items[0].signed = nil
        return .signAgain(number: next, clockOffset: 0)
    }

    private mutating func settle(_ result: Settled.Result) -> Step {
        .settled(Settled(item: items.removeFirst(), result: result))
    }
}

extension Why {
    public static let notSigned = Why(code: "not_signed", text: "Not signed, so not sent.")
}
