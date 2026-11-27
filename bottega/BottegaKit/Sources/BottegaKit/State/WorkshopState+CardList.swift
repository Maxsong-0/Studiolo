// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.

// What the cards' list view (vault Bottega-5, section 7) asks of the state. Nothing here changes it.

/// Where a card's tests and merge stand, as one cell of the list says it.
public enum CardProgress: Equatable, Sendable {
    /// Nothing worth a word.
    case none
    /// Not started, and its writer said it waits for these first.
    case waitsFor([String])
    /// On the testbox now: submitted, handed over or running.
    case testsRunning
    case testsPassed
    case testsFailed(Why)
    /// LMAS asks the user before it merges.
    case mergeWaitsForYou
    case merging(Merge.Stage)
    case merged
    case notMerged(Why)
    /// The card itself failed.
    case failed(Why)

    /// The one mark the cell draws besides its words.
    public enum Mark: Equatable, Sendable {
        case none
        /// A process is running (grey, not the amber of an AI at work).
        case spinner
        case cross
        /// Red: it waits for the user.
        case waitsForYou
    }

    public var mark: Mark {
        switch self {
        case .testsRunning: .spinner
        case .testsFailed: .cross
        case .mergeWaitsForYou: .waitsForYou
        case .none, .waitsFor, .testsPassed, .merging, .merged, .notMerged, .failed: .none
        }
    }
}

extension WorkshopState {
    /// When anything last happened to a card: to it, its reviews, its tests, its merges or the user's say on them.
    /// 0 when nothing says.
    public func lastActivity(of card: Card) -> UInt64 {
        var times: [UInt64?] = [card.createdAt, card.work?.at, card.end?.at]
        times += card.requeues.map(\.at)
        times += card.reassignments.map(\.at)
        times += card.reminders.map(\.at)
        times += card.reviews.flatMap { [$0.startedAt, $0.outcome?.at] }
        times += card.tests.map { tests[$0]?.lastAt }
        times += card.merges.map { merges[$0]?.lastAt }
        times += asks.values.filter { $0.about.merge.map(card.merges.contains) ?? false }.flatMap { [$0.at, $0.answer?.at] }
        return times.compactMap(\.self).max() ?? 0
    }

    /// Every card, the one something last happened to first. Of two that moved at the same moment, the one heard of
    /// later comes first, so the order does not flicker between redraws.
    public var cardsByRecentActivity: [Card] {
        cardOrder.enumerated().compactMap { index, id in cards[id].map { (card: $0, at: lastActivity(of: $0), index: index) } }
            .sorted { ($0.at, $0.index) > ($1.at, $1.index) }
            .map(\.card)
    }

    /// Files changed and lines added and taken away, known once a merge of it has been checked: the last one that was.
    public func size(of card: Card) -> Merge.Size? {
        card.merges.lazy.reversed().compactMap { merges[$0]?.size }.first
    }

    /// What the list's tests-and-merge cell says. What waits for the user comes first, then how the card failed,
    /// then whichever of its last test run and its last merge moved last (a merge runs its own tests, and those
    /// move after the merge's own last step), then what a card in the queue waits for.
    public func progress(of card: Card) -> CardProgress {
        if card.merges.contains(where: { merge in asks.values.contains { $0.about.merge == merge && $0.answer == nil } }) {
            return .mergeWaitsForYou
        }
        if case .failed(let why, _) = card.end { return .failed(why) }
        let test = card.tests.last.flatMap { tests[$0] }
        let merge = card.merges.last.flatMap { merges[$0] }
        // A tie goes to the merge: it is the later step.
        if let merge, (merge.lastAt ?? 0) >= (test?.lastAt ?? 0) { return progress(of: merge) }
        if let test { return progress(of: test) }
        if card.work == nil, card.end == nil, !card.after.isEmpty { return .waitsFor(card.after) }
        return .none
    }

    private func progress(of merge: Merge) -> CardProgress {
        switch merge.stage {
        case .finished: .merged
        case .failed(let why, _): .notMerged(why)
        case .requested, .checked, .prepared, .restarted, .landing: .merging(merge.stage)
        }
    }

    private func progress(of test: TestRun) -> CardProgress {
        switch test.stage {
        case .submitted, .handedOver, .accepted: .testsRunning
        case .finished(let outcome, _, _): outcome.code == "passed" ? .testsPassed : .testsFailed(outcome)
        }
    }
}

extension Overview.Cards {
    /// What a card's project tag says: the repository's folder name. LMAS has one repository for all its cards for
    /// now (vault Bottega-8, A2), so every card gets the same one.
    public var projectLabel: String? {
        let name = repo.split(separator: "/").last.map(String.init) ?? ""
        let label = UntrustedText.line(name, limit: 40)
        return label.isEmpty ? nil : label
    }
}

/// How long ago, as short as the list's last column has room for.
public enum Elapsed {
    public static func short(from then: UInt64, to now: UInt64) -> String {
        let minutes = (now > then ? now - then : 0) / 60_000
        switch minutes {
        case ..<1: return "now"
        case ..<60: return "\(minutes)m"
        case ..<(24 * 60): return "\(minutes / 60)h"
        default: return "\(minutes / (24 * 60))d"
        }
    }
}

extension TestRun {
    /// When it last moved.
    var lastAt: UInt64? {
        switch stage {
        case .submitted: submittedAt
        case .handedOver(let at), .accepted(let at), .finished(_, _, let at): at
        }
    }
}

extension Merge {
    /// When it last moved.
    var lastAt: UInt64? {
        switch stage {
        case .requested: requestedAt
        case .checked(let at), .prepared(let at), .restarted(_, let at), .landing(let at), .finished(let at), .failed(_, let at): at
        }
    }
}

extension Card.End {
    var at: UInt64 {
        switch self {
        case .done(_, let at), .failed(_, let at): at
        }
    }
}

extension Card.Review.Outcome {
    var at: UInt64 {
        switch self {
        case .finished(_, _, let at), .notDone(_, let at), .requeued(_, _, let at): at
        }
    }
}
