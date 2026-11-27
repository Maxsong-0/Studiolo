// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Testing
@testable import BottegaKit

/// The cards' list view: which card is on top, and what its tests-and-merge cell says.
struct CardListTests {
    // MARK: Recorded

    /// From LMAS's events: card-5 failed last (…553000), card-3 just before (…552000), card-1's review went to another
    /// vendor at …453000, card-4 was reassigned at …452000, and card-2's review was given up at …28000.
    @Test func theRecordedCardsByWhatLastHappened() throws {
        let state = try Fixtures.state()
        #expect(state.cardsByRecentActivity.map(\.id) == ["card-5", "card-3", "card-1", "card-4", "card-2"])
        #expect(try state.lastActivity(of: #require(state.cards["card-1"])) == 1764000453000)
        #expect(try state.lastActivity(of: #require(state.cards["card-2"])) == 1764000028000)
    }

    @Test func theRecordedCardsProgress() throws {
        let state = try Fixtures.state()
        func progress(_ id: String) throws -> CardProgress { state.progress(of: try #require(state.cards[id])) }
        // merge-1 landed at …23000; test-2, run for it, was submitted after that and never finished.
        #expect(try progress("card-1") == .testsRunning)
        #expect(try progress("card-2") == .failed(Why(code: "worker_stopped", text: "Worker 没回 RESULT 就停了：一轮超时")))
        #expect(try progress("card-3") == .failed(Why(code: "unanswered", text: "Worker 问写卡人的 msg-16 到了时限还没人答")))
        #expect(try progress("card-4") == .none)
        #expect(try progress("card-5") == .failed(Why(code: "dependency_failed", text: "等的 card-4 那一串停住了（失败了，或者审查要改），到了时限没人接着返工，这张卡没开始过")))
        #expect(try state.size(of: #require(state.cards["card-1"])) == Merge.Size(files: try sizeFiles(state), insertions: 30, deletions: 4))
        #expect(try state.size(of: #require(state.cards["card-2"])) == nil)
    }

    private func sizeFiles(_ state: WorkshopState) throws -> Int { try #require(state.merges["merge-1"]?.size?.files) }

    // MARK: Each case of the cell

    @Test func aMergeThatAsksTheUserComesFirst() throws {
        var state = WorkshopState()
        try state.apply(record(1, created("c", at: 1), started("c", at: 2), mergeRequested("m", card: "c", at: 3)))
        #expect(state.progress(of: try card(state, "c")) == .merging(.requested))
        try state.apply(record(4, .askOpened(.init(ask: "a", about: .init(kind: "merge", merge: "m"), by: "lmas", text: "merge?", atMs: 4))))
        #expect(state.progress(of: try card(state, "c")) == .mergeWaitsForYou)
        #expect(state.progress(of: try card(state, "c")).mark == .waitsForYou)
        try state.apply(record(5, .askAnswered(.init(ask: "a", answer: "approved", via: .init(at: "device", name: "Mac", device: "d"), atMs: 5))))
        #expect(state.progress(of: try card(state, "c")) == .merging(.requested))
    }

    /// The size shown is that of the last merge that was checked: a card merged again shows its new change, once
    /// LMAS has checked it.
    @Test func theSizeIsTheLastCheckedMerges() throws {
        var state = WorkshopState()
        try state.apply(record(1, created("c", at: 1), started("c", at: 2), mergeRequested("m1", card: "c", at: 3)))
        #expect(state.size(of: try card(state, "c")) == nil)
        try state.apply(record(4, .mergeChecked(.init(merge: "m1", tree: "t", base: "b", files: 1, insertions: 2, deletions: 3, atMs: 4))))
        try state.apply(record(5, mergeRequested("m2", card: "c", at: 5)))
        #expect(state.size(of: try card(state, "c")) == Merge.Size(files: 1, insertions: 2, deletions: 3))
        try state.apply(record(6, .mergeChecked(.init(merge: "m2", tree: "t", base: "b", files: 4, insertions: 5, deletions: 6, atMs: 6))))
        #expect(state.size(of: try card(state, "c")) == Merge.Size(files: 4, insertions: 5, deletions: 6))
    }

    /// Asking the user about its merge, and the answer, are things that happened to the card.
    @Test func theUsersSayOnAMergeCountsAsActivity() throws {
        var state = WorkshopState()
        try state.apply(record(1, created("c", at: 1), mergeRequested("m", card: "c", at: 3), created("d", at: 4)))
        #expect(state.cardsByRecentActivity.map(\.id) == ["d", "c"])
        try state.apply(record(4, .askOpened(.init(ask: "a", about: .init(kind: "merge", merge: "m"), by: "lmas", text: "merge?", atMs: 5))))
        #expect(try state.lastActivity(of: card(state, "c")) == 5)
        #expect(state.cardsByRecentActivity.map(\.id) == ["c", "d"])
        try state.apply(record(5, created("e", at: 6), .askAnswered(.init(ask: "a", answer: "declined", via: .init(at: "device", name: "Mac", device: "d"), atMs: 7))))
        #expect(try state.lastActivity(of: card(state, "c")) == 7)
        // An ask about another merge is not this card's.
        try state.apply(record(7, .askOpened(.init(ask: "b", about: .init(kind: "merge", merge: "other"), by: "lmas", text: "?", atMs: 9))))
        #expect(try state.lastActivity(of: card(state, "c")) == 7)
    }

    @Test func aFailedCardSaysWhy() throws {
        let why = Why(code: "worker_stopped", text: "stopped")
        var state = WorkshopState()
        try state.apply(record(1, created("c", at: 1), testsSubmitted("t", card: "c", at: 2), .cardFailed(.init(card: "c", why: why, atMs: 3))))
        #expect(state.progress(of: try card(state, "c")) == .failed(why))
    }

    @Test func theLaterOfTheLastTestAndTheLastMergeSpeaks() throws {
        let failed = Why(code: "failed", text: "2 failed")
        var state = WorkshopState()
        try state.apply(record(1, created("c", at: 1), started("c", at: 2), testsSubmitted("t1", card: "c", at: 3)))
        #expect(state.progress(of: try card(state, "c")) == .testsRunning)
        #expect(state.progress(of: try card(state, "c")).mark == .spinner)
        try state.apply(record(4, .testsAccepted(.init(test: "t1", job: "j", atMs: 4))))
        #expect(state.progress(of: try card(state, "c")) == .testsRunning)
        try state.apply(record(5, .testsFinished(.init(test: "t1", outcome: failed, report: nil, atMs: 5))))
        #expect(state.progress(of: try card(state, "c")) == .testsFailed(failed))
        #expect(state.progress(of: try card(state, "c")).mark == .cross)
        try state.apply(record(6, testsSubmitted("t2", card: "c", at: 6), .testsFinished(.init(test: "t2", outcome: Why(code: "passed", text: "ok"), report: nil, atMs: 7))))
        #expect(state.progress(of: try card(state, "c")) == .testsPassed)
        // The merge moved after the tests did.
        try state.apply(record(8, mergeRequested("m", card: "c", at: 8)))
        #expect(state.progress(of: try card(state, "c")) == .merging(.requested))
        try state.apply(record(9, .mergeChecked(.init(merge: "m", tree: "t", base: "b", files: 2, insertions: 5, deletions: 1, atMs: 9))))
        #expect(state.progress(of: try card(state, "c")) == .merging(.checked(at: 9)))
        #expect(state.size(of: try card(state, "c")) == Merge.Size(files: 2, insertions: 5, deletions: 1))
        // The merge's own tests move after its last step.
        try state.apply(record(10, testsSubmitted("t3", card: "c", at: 10, merge: "m")))
        #expect(state.progress(of: try card(state, "c")) == .testsRunning)
        try state.apply(record(11, .mergeFinished(.init(merge: "m", atMs: 11))))
        #expect(state.progress(of: try card(state, "c")) == .merged)
        #expect(state.progress(of: try card(state, "c")).mark == .none)
    }

    @Test func aTieGoesToTheMerge() throws {
        let why = Why(code: "conflict", text: "conflict")
        var state = WorkshopState()
        try state.apply(record(1, created("c", at: 1), testsSubmitted("t", card: "c", at: 5), mergeRequested("m", card: "c", at: 5)))
        #expect(state.progress(of: try card(state, "c")) == .merging(.requested))
        try state.apply(record(4, .mergeFailed(.init(merge: "m", why: why, atMs: 6)), .testsAccepted(.init(test: "t", job: "j", atMs: 6))))
        #expect(state.progress(of: try card(state, "c")) == .notMerged(why))
    }

    @Test func aQueuedCardSaysWhatItWaitsFor() throws {
        var state = WorkshopState()
        try state.apply(record(1, created("c", at: 1, after: ["a", "b"])))
        #expect(state.progress(of: try card(state, "c")) == .waitsFor(["a", "b"]))
        // Once started, what it waited for no longer holds it.
        try state.apply(record(2, started("c", at: 2)))
        #expect(state.progress(of: try card(state, "c")) == .none)
    }

    @Test func cardsThatMovedAtOnceKeepTheLaterHeardOfFirst() throws {
        var state = WorkshopState()
        try state.apply(record(1, created("a", at: 5), created("b", at: 5), created("c", at: 3)))
        #expect(state.cardsByRecentActivity.map(\.id) == ["b", "a", "c"])
        try state.apply(record(4, testsSubmitted("t", card: "c", at: 9)))
        #expect(state.cardsByRecentActivity.map(\.id) == ["c", "b", "a"])
        try state.apply(record(5, mergeRequested("m", card: "a", at: 10), .mergeLanding(.init(merge: "m", atMs: 12))))
        #expect(state.cardsByRecentActivity.map(\.id) == ["a", "c", "b"])
        #expect(try state.lastActivity(of: card(state, "a")) == 12)
    }

    // MARK: Words

    @Test func theProjectTagIsTheRepositorysFolder() {
        func label(_ repo: String) -> String? {
            Overview.Cards(repo: repo, vendors: [], reviewers: nil, tests: true, mergeInto: nil).projectLabel
        }
        #expect(label("/Users/max/app") == "app")
        #expect(label("/Users/max/app/") == "app")
        #expect(label("studiolo") == "studiolo")
        #expect(label("") == nil)
        #expect(label("/") == nil)
        #expect(label("/x/a\u{202E}b") == UntrustedText.line("a\u{202E}b", limit: 40))
    }

    @Test func aCardNothingTimesHasNoAge() throws {
        let minute: UInt64 = 60_000
        var state = WorkshopState()
        try state.apply(record(1, created("timed", at: minute), created("untimed", at: 0)))
        #expect(try state.age(of: card(state, "timed"), now: 3 * minute) == "2m")
        #expect(try state.age(of: card(state, "untimed"), now: 3 * minute) == "")
    }

    @Test func howLongAgo() {
        let minute: UInt64 = 60_000
        #expect(Elapsed.short(from: 0, to: 59_999) == "now")
        #expect(Elapsed.short(from: 0, to: minute) == "1m")
        #expect(Elapsed.short(from: 0, to: 59 * minute + 59_999) == "59m")
        #expect(Elapsed.short(from: 0, to: 60 * minute) == "1h")
        #expect(Elapsed.short(from: 0, to: 24 * 60 * minute - 1) == "23h")
        #expect(Elapsed.short(from: 0, to: 24 * 60 * minute) == "1d")
        #expect(Elapsed.short(from: 0, to: 100 * 24 * 60 * minute) == "100d")
        // A clock that went back says now, not a number below zero.
        #expect(Elapsed.short(from: 10 * minute, to: minute) == "now")
    }

    // MARK: Making events

    private func card(_ state: WorkshopState, _ id: String) throws -> Card { try #require(state.cards[id]) }

    private func record(_ seq: UInt64, _ events: Event...) -> Record {
        Record(seq: seq, through: seq + UInt64(max(events.count, 1)) - 1, events: events)
    }

    private func created(_ id: String, at: UInt64, after: [String]? = nil) -> Event {
        .cardCreated(
            .init(
                card: id, author: "dispatcher", title: id, instructions: "", acceptance: "", conventions: nil, vendor: "codex",
                continues: nil, reviewer: nil, thread: nil, vendorFree: nil, after: after, atMs: at))
    }

    private func started(_ id: String, at: UInt64) -> Event {
        .cardStarted(.init(card: id, worker: "coder-\(id)", worktree: "/w", branch: "b", base: "main", gitDir: nil, freshStart: nil, task: "task-\(id)", atMs: at))
    }

    private func testsSubmitted(_ id: String, card: String, at: UInt64, merge: String? = nil) -> Event {
        .testsSubmitted(.init(test: id, card: card, by: "coder", request: .init(command: "t", tier: "targeted", memoryGib: 1, cpu: 1), merge: merge, atMs: at))
    }

    private func mergeRequested(_ id: String, card: String, at: UInt64) -> Event {
        .mergeRequested(.init(merge: id, card: card, by: "coder", target: "main", test: nil, gaps: nil, asksUser: true, atMs: at))
    }
}
