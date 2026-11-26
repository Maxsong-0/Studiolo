// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import CryptoKit
import Foundation
import Testing
@testable import BottegaKit

/// What the recorded events add up to.
struct RecordedStateTests {
    @Test func sessions() throws {
        let state = try Fixtures.state()
        #expect(state.through == 128)
        #expect(state.sessions["dispatcher"]?.role == "dispatcher")
        #expect(state.sessions["dispatcher"]?.isOpen == true)
        #expect(state.sessions["coder-card-1"]?.closedAt == 1764000014000)
    }

    @Test func messagesKeepTheirState() throws {
        let state = try Fixtures.state()
        #expect(state.messages["msg-1"]?.state == .handed(at: 1764000004000))
        #expect(state.messages["msg-2"]?.state == .answered(by: "msg-3"))
        #expect(state.messages["msg-4"]?.state == .answered(by: "msg-5"))
        #expect(state.messages["msg-3"]?.state == .queued)
        #expect(state.messages["msg-9"]?.device == "device-1")
        #expect(state.messages["msg-8"]?.files.first?.name == "草图.md")
        // msg-6 was never queued as far as this log says; its failure is still on record.
        #expect(state.messages["msg-6"] == nil)
        #expect(state.log.contains { if case .messageFailed(let failed) = $0.event { failed.id == "msg-6" } else { false } })
    }

    @Test func linesAndTheirMembers() throws {
        let state = try Fixtures.state()
        let first = try #require(state.threads["msg-1"])
        #expect(first.messages == ["msg-1", "msg-2", "msg-3", "msg-8"])
        #expect(first.members == ["planner", "dispatcher", "coder-card-1", "coder-card-2"])
        #expect(first.deepestHop == 2)
        #expect(first.busiestPair?.pair == .init("dispatcher", "coder-card-1"))
        #expect(first.busiestPair?.count == 2)
        #expect(state.kind(of: first) == .planner)
        #expect(state.title(of: first) == "给设置页加一个「清空缓存」按钮。")
        #expect(try state.kind(of: #require(state.threads["msg-4"])) == .agents)
        #expect(try state.title(of: #require(state.threads["msg-4"])) == "Review of card-1")
        #expect(try state.kind(of: #require(state.threads["msg-9"])) == .dispatcher)
        // LMAS's reminders to the dispatcher sit with it.
        #expect(try state.kind(of: #require(state.threads["msg-17"])) == .dispatcher)
        #expect(try state.kind(of: #require(state.threads["msg-18"])) == .dispatcher)
        #expect(state.threads(of: .dispatcher).map(\.id) == ["msg-9", "msg-18", "msg-17"])
    }

    @Test func cards() throws {
        let state = try Fixtures.state()
        let one = try #require(state.cards["card-1"])
        #expect(one.title == "清空缓存按钮")
        #expect(one.work?.branch == "lmas/card-1")
        #expect(one.end == .done(result: "msg-3", at: 1764000013000))
        #expect(one.tests == ["test-1", "test-2"])
        #expect(one.merges == ["merge-1"])
        // The review that went back to wait was never seen starting: it stands on its own, after the one that finished.
        #expect(one.reviews.map(\.reviewer) == ["reviewer-card-1", nil])
        #expect(one.reviews.first?.outcome == .finished(verdict: "approve", result: "msg-5", at: 1764000016000))
        guard case .requeued(let action, let requeueWhy, _) = one.reviews.last?.outcome else { Issue.record("requeued review"); return }
        #expect(action == "retry" && requeueWhy.code == "process_ended")
        #expect(state.column(of: one) == .done)
        let two = try #require(state.cards["card-2"])
        #expect(two.continues == "card-1")
        #expect(two.work?.freshStart?.named == ["src/settings.rs"])
        #expect(two.work?.gitDir == "/Users/max/app/.git/worktrees/card-2" && one.work?.gitDir == nil)
        #expect(two.thread == "msg-6")
        guard case .failed(let why, _) = two.end else { Issue.record("card-2"); return }
        #expect(why.code == "worker_stopped")
        guard case .notDone(let why, _) = two.reviews.first?.outcome else { Issue.record("review"); return }
        #expect(why.code == "no_verdict")
        #expect(state.column(of: two) == .failed)
        #expect(state.vendors["coder-card-1"] == "codex")
        #expect(state.vendors["reviewer-card-1"] == "claude")
        let three = try #require(state.cards["card-3"])
        #expect(three.requeues.map(\.action) == ["retry", "hold"])
        #expect(three.requeues.first?.why.code == "service_down")
        guard case .failed(let threeWhy, _) = three.end else { Issue.record("card-3"); return }
        // It failed twice: the AI stopped, then (after a rework) its question went unanswered.
        #expect(threeWhy.code == "unanswered")
        #expect(state.column(of: three) == .failed)
        let tenWhy = state.log.lazy.compactMap { if case .messageFailed(let e) = $0.event, e.id == "msg-10" { e.why } else { nil } }.first
        #expect(tenWhy?.code == "card_requeued")
    }

    @Test func testsAndMerges() throws {
        let state = try Fixtures.state()
        #expect(state.tests["test-1"]?.job == "20251124-010101-ab12")
        guard case .finished(let outcome, let report, _) = state.tests["test-1"]?.stage else { Issue.record("test-1"); return }
        #expect(outcome.code == "failed" && report?.tail == "1 failed\n")
        #expect(state.tests["test-2"]?.stage == .submitted)
        #expect(state.tests["test-2"]?.merge == "merge-1")
        let merge = try #require(state.merges["merge-1"])
        #expect(merge.stage == .finished(at: 1764000023000))
        #expect(merge.cleaned)
        #expect(merge.size == .init(files: 2, insertions: 30, deletions: 4))
        #expect(merge.restarts.map(\.code) == ["target_moved"])
        #expect(merge.gaps.map(\.code) == ["tests_skipped"])
        // These two were never requested as far as this log says.
        guard case .failed(let why, _) = state.merges["merge-2"]?.stage else { Issue.record("merge-2"); return }
        #expect(why.code == "conflict")
        #expect(state.merges["merge-3"]?.isOver == true)
    }

    @Test func asksDevicesAndEchoes() throws {
        let state = try Fixtures.state()
        #expect(state.asks["ask-1"]?.answer?.word == "approved")
        #expect(state.asks["ask-1"]?.text.contains("+30 −4") == true)
        #expect(state.asks["ask-2"]?.answer?.word == "withdrawn")
        let byDevice = try #require(state.asks["ask-3"])
        #expect(byDevice.onlyDevice)
        #expect(byDevice.answer?.word == "approved")
        #expect(byDevice.answer?.via == .init(at: "device", name: nil, device: "device-1"))
        // What was signed is what this code shows.
        #expect(try Fixtures.vectors().intents.last { $0.intent.kind == "approve" }?.intent.body.contains(byDevice.textDigest) == true)
        let device = try #require(state.devices["device-1"])
        #expect(device.name == "书房的 Mac Studio")
        #expect(device.intents == 17 && device.refusals == 3)
        let vectors = try Fixtures.vectors()
        for vector in vectors.intents { #expect(state.echoes[vector.digest]?.number == vector.intent.number) }
        // All three kinds have been answered but the limit, which LMAS never clears.
        #expect(state.waiting.map(\.what) == [.trip(RowID(seq: 42, index: 0))])
    }

    @Test func everyEventIsInTheLogOnceAndRowsAreUnique() throws {
        let state = try Fixtures.state()
        let events = try Fixtures.records().reduce(0) { $0 + $1.events.count }
        #expect(state.log.count == events)
        #expect(Set(state.log.map(\.id)).count == events)
        #expect(state.log.map(\.id) == state.log.map(\.id).sorted())
        for thread in state.threads.values { #expect(Set(thread.rows.map(\.id)).count == thread.rows.count) }
    }

    @Test func lanes() throws {
        let state = try Fixtures.state()
        let lanes = state.lanes(of: try #require(state.threads["msg-1"]))
        #expect(lanes.members.count == 4)
        #expect(lanes.letters.map(\.id) == ["msg-1", "msg-2", "msg-3", "msg-8"])
        #expect(lanes.letters.map(\.isReply) == [false, false, true, false])
        // msg-1 is still being worked on; msg-2 was, until msg-3 answered it.
        #expect(lanes.working == [.init(lane: 1, top: 0.5, bottom: nil), .init(lane: 2, top: 1.5, bottom: 2.5)])
    }
}

struct StateTests {
    private func record(_ seq: UInt64, through: UInt64? = nil, _ events: Event...) -> Record {
        Record(seq: seq, through: through ?? seq + UInt64(max(events.count, 1)) - 1, events: events)
    }

    private func message(
        _ id: String, from: String, to: String, kind: String = "task", thread: String? = nil, replyTo: String? = nil,
        at: UInt64 = 1, device: String? = nil
    ) -> Event {
        .messageQueued(
            .init(
                message: .init(
                    id: id, from: from, to: to, kind: kind, thread: thread ?? id, hop: 0, replyTo: replyTo, task: nil,
                    body: id, files: nil, atMs: at, device: device)))
    }

    // MARK: Numbers

    @Test func recordsMustFollowOn() throws {
        var state = WorkshopState()
        try state.apply(record(1, through: 3))
        #expect(state.through == 3)
        #expect(throws: SequenceBreak(expected: 4, got: 5)) { try state.apply(record(5)) }
        #expect(throws: SequenceBreak(expected: 4, got: 3)) { try state.apply(record(3)) }
        // A refused record leaves the numbers where they were.
        #expect(state.through == 3)
        try state.apply(record(4))
        #expect(state.through == 4)
    }

    /// An empty record still moves the numbers on, and how many events there are says nothing about them.
    @Test func numbersAreNotCountedFromEvents() throws {
        var state = WorkshopState()
        try state.apply(record(1, through: 7))
        try state.apply(record(8, through: 12, message("m", from: "a", to: "b")))
        try state.apply(record(13, through: 13))
        #expect(state.through == 13)
    }

    @Test func historyMayStartLate() throws {
        var state = WorkshopState(startingAt: 40)
        #expect(state.through == 39)
        // LMAS gives the whole record the number falls in.
        try state.apply(record(38, through: 41))
        #expect(state.through == 41)
        #expect(throws: SequenceBreak.self) { try state.apply(record(40, through: 45)) }
        var wrong = WorkshopState(startingAt: 40)
        #expect(throws: SequenceBreak.self) { try wrong.apply(record(41)) }
        var early = WorkshopState(startingAt: 40)
        #expect(throws: SequenceBreak.self) { try early.apply(record(30, through: 39)) }
    }

    @Test func aRecordThatEndsBeforeItBeginsIsRefused() {
        var state = WorkshopState()
        #expect(throws: SequenceBreak.self) { try state.apply(Record(seq: 1, through: 0, events: [])) }
    }

    // MARK: Waiting

    @Test func waitingIsLongestFirstAndClears() throws {
        var state = WorkshopState()
        try state.apply(record(1,
            .askOpened(.init(ask: "ask-1", about: .init(kind: "question", merge: nil), by: "dispatcher", text: "行？", onlyDevice: nil, atMs: 30)),
            message("q", from: "coder", to: "user", kind: "question", at: 10),
            .limitTripped(.init(limit: "pair", thread: nil, from: "a", to: "b", kind: "task", atMs: 20)),
            message("n", from: "coder", to: "user", kind: "notification", at: 5)))
        #expect(state.waiting.map(\.what) == [.question("q"), .trip(RowID(seq: 1, index: 2)), .ask("ask-1")])
        try state.apply(record(5,
            .askAnswered(.init(ask: "ask-1", answer: "declined", via: .init(at: "dialog", name: nil, device: nil), atMs: 40)),
            message("r", from: "user", to: "coder", kind: "result", thread: "q", replyTo: "q", at: 41, device: "device-1")))
        #expect(state.waiting.map(\.what) == [.trip(RowID(seq: 1, index: 2))])
        #expect(state.messages["q"]?.state == .answered(by: "r"))
        #expect(state.messagesToUser == ["q", "n"])
    }

    @Test func aQuestionThatFailedIsNotWaitedOn() throws {
        var state = WorkshopState()
        try state.apply(record(1, message("q", from: "coder", to: "user", kind: "question")))
        try state.apply(record(2, .messageFailed(.init(id: "q", why: .init(code: "recipient_closed", text: "x"), atMs: 2))))
        #expect(state.waiting.isEmpty)
    }

    /// Handed over after it was answered: the answer stands.
    @Test func statesDoNotGoBackwards() throws {
        var state = WorkshopState()
        try state.apply(record(1, message("t", from: "a", to: "b"), message("r", from: "b", to: "a", kind: "result", thread: "t", replyTo: "t")))
        try state.apply(record(3, .messageHanded(.init(id: "t", atMs: 9))))
        #expect(state.messages["t"]?.state == .answered(by: "r"))
    }

    @Test func cardColumns() throws {
        var state = WorkshopState()
        func column() -> WorkshopState.Column? { state.cards["card-1"].map(state.column(of:)) }
        try state.apply(record(1, .cardCreated(.init(
            card: "card-1", author: "dispatcher", title: "t", instructions: "i", acceptance: "a", conventions: nil, vendor: "codex",
            continues: nil, reviewer: nil, thread: nil, atMs: 1))))
        #expect(column() == .queued)
        try state.apply(record(2, .cardStarted(.init(
            card: "card-1", worker: "w", worktree: "/w", branch: "b", base: "c", freshStart: nil, task: "m1", atMs: 2))))
        #expect(column() == .working)
        #expect(state.cards["card-1"]?.thread == "m1")
        try state.apply(record(3, message("q", from: "w", to: "user", kind: "question", thread: "m1")))
        #expect(column() == .needsYou)
        try state.apply(record(4, message("r", from: "user", to: "w", kind: "result", thread: "m1", replyTo: "q")))
        #expect(column() == .working)
        try state.apply(record(5, .cardDone(.init(card: "card-1", result: "m2", atMs: 5))))
        #expect(column() == .done)
        try state.apply(record(6, .mergeRequested(.init(
            merge: "merge-1", card: "card-1", by: "dispatcher", target: "main", test: nil, gaps: nil, asksUser: true, atMs: 6))))
        try state.apply(record(7, .askOpened(.init(
            ask: "ask-1", about: .init(kind: "merge", merge: "merge-1"), by: "dispatcher", text: "合？", onlyDevice: nil, atMs: 7))))
        #expect(column() == .needsYou)
        try state.apply(record(8, .askAnswered(.init(
            ask: "ask-1", answer: "approved", via: .init(at: "device", name: nil, device: "device-1"), atMs: 8))))
        #expect(column() == .done)
        #expect(state.cards(in: .done).map(\.id) == ["card-1"])
    }

    /// A card whose AI stopped halfway waits again, and is worked again when a fresh one starts it.
    @Test func aRequeuedCardWaitsThenRunsAgain() throws {
        var state = WorkshopState()
        func column() -> WorkshopState.Column? { state.cards["card-1"].map(state.column(of:)) }
        try state.apply(record(1, .cardCreated(.init(
            card: "card-1", author: "dispatcher", title: "t", instructions: "i", acceptance: "a", conventions: nil, vendor: "codex",
            continues: nil, reviewer: nil, thread: nil, atMs: 1))))
        try state.apply(record(2, .cardStarted(.init(
            card: "card-1", worker: "w", worktree: "/w", branch: "b", base: "c", freshStart: nil, task: "m1", atMs: 2))))
        #expect(column() == .working)
        try state.apply(record(3, .cardRequeued(.init(card: "card-1", action: "retry", why: .init(code: "rate_limited", text: "限速"), atMs: 3))))
        #expect(column() == .queued)
        #expect(state.cards["card-1"]?.requeues.count == 1)
        #expect(state.cards["card-1"]?.thread == "m1")
        #expect(state.dispatcherRows.contains { $0.kind == .cardRequeued(card: "card-1", index: 0) })
        try state.apply(record(4, .cardStarted(.init(
            card: "card-1", worker: "w2", worktree: "/w", branch: "b", base: "c", freshStart: nil, task: "m2", atMs: 4))))
        #expect(column() == .working)
        #expect(state.cards["card-1"]?.work?.worker == "w2")
        #expect(state.cards["card-1"]?.requeues.count == 1)
    }

    /// A review that went back to wait closes the review it was, and the next `review_started` is a new one.
    @Test func aRequeuedReviewStartsAgain() throws {
        var state = WorkshopState()
        try state.apply(record(1, .reviewStarted(.init(card: "card-1", reviewer: "r1", base: "b", tree: "t", task: "t1", atMs: 1))))
        try state.apply(record(2, .reviewRequeued(.init(card: "card-1", action: "retry", why: .init(code: "process_ended", text: "x"), atMs: 2))))
        try state.apply(record(3, .reviewStarted(.init(card: "card-1", reviewer: "r2", base: "b", tree: "t", task: "t2", atMs: 3))))
        let reviews = try #require(state.cards["card-1"]?.reviews)
        #expect(reviews.map(\.reviewer) == ["r1", "r2"])
        guard case .requeued(let action, _, let at) = reviews.first?.outcome else { Issue.record("first"); return }
        #expect(action == "retry" && at == 2)
        #expect(reviews.last?.outcome == nil)
    }

    /// Things the log never introduced: shown from what is said about them, not dropped and not a crash.
    @Test func strangersAreTakenIn() throws {
        var state = WorkshopState()
        try state.apply(record(1,
            .cardDone(.init(card: "card-9", result: "m", atMs: 1)),
            .testsFinished(.init(test: "test-9", outcome: .init(code: "passed", text: "过了"), report: nil, atMs: 1)),
            .testsHandedOver(.init(test: "test-8", atMs: 1)),
            .mergeLanding(.init(merge: "merge-9", atMs: 1)),
            .mergeCleaned(.init(merge: "merge-8", atMs: 1)),
            .askAnswered(.init(ask: "ask-9", answer: "approved", via: .init(at: "dialog", name: nil, device: nil), atMs: 1)),
            .messageHanded(.init(id: "msg-9", atMs: 1)),
            .sessionClosed(.init(name: "ghost", atMs: 1)),
            .reviewFinished(.init(card: "card-8", verdict: "changes", result: "m", atMs: 1)),
            .reviewNotDone(.init(card: "card-8", why: .init(code: "x", text: "x"), atMs: 1)),
            .intentEchoed(.init(device: "device-9", number: 1, digest: "d", contentId: "c", outcome: .done(message: nil), atMs: 1))))
        #expect(state.log.count == 11)
        #expect(state.cardOrder == ["card-9", "card-8"])
        #expect(state.cards["card-8"]?.reviews.count == 2)
        #expect(state.sessions["ghost"]?.isOpen == false)
        #expect(state.asks["ask-9"]?.answer != nil)
        #expect(state.waiting.isEmpty)
    }

    @Test func aSessionCanComeBack() throws {
        var state = WorkshopState()
        try state.apply(record(1, .sessionRegistered(.init(name: "x", role: "coder", atMs: 1)), .sessionClosed(.init(name: "x", atMs: 2))))
        #expect(state.sessions["x"]?.isOpen == false)
        try state.apply(record(3, .sessionRegistered(.init(name: "x", role: "reviewer", atMs: 3))))
        #expect(state.sessions["x"]?.isOpen == true)
        #expect(state.sessions["x"]?.role == "reviewer")
    }

    @Test func whoTheUserCanTalkTo() throws {
        var state = WorkshopState()
        try state.apply(record(1,
            .sessionRegistered(.init(name: "coder", role: "coder", atMs: 1)),
            .sessionRegistered(.init(name: "boss", role: "dispatcher", atMs: 1)),
            .sessionRegistered(.init(name: "user", role: "user", atMs: 1)),
            .sessionRegistered(.init(name: "gone", role: "coder", atMs: 1)),
            .sessionClosed(.init(name: "gone", atMs: 1)),
            message("m", from: "user", to: "coder"), message("n", from: "coder", to: "gone", thread: "m")))
        #expect(state.recipients.map(\.name) == ["boss", "coder"])
        #expect(state.recipients(on: try #require(state.threads["m"])).map(\.name) == ["coder"])
        #expect(state.kind(of: try #require(state.threads["m"])) == .direct)
    }

    /// A letter LMAS writes starts a line of its own; it sits with whom it went to, and a line an agent starts stays
    /// between agents even when it is the dispatcher's.
    @Test func lmasLettersSitWithWhomTheyWentTo() throws {
        var state = WorkshopState()
        try state.apply(record(1,
            .sessionRegistered(.init(name: "dispatcher", role: "dispatcher", atMs: 1)),
            .sessionRegistered(.init(name: "planner", role: "planner", atMs: 1)),
            .sessionRegistered(.init(name: "coder-c", role: "coder", atMs: 1)),
            message("a", from: Session.coordinator, to: "dispatcher", kind: "notification"),
            message("b", from: Session.coordinator, to: "planner", kind: "notification"),
            message("c", from: Session.coordinator, to: "coder-c", kind: "notification"),
            message("d", from: "dispatcher", to: "coder-c")))
        func kind(_ id: String) throws -> WorkshopState.LineKind { state.kind(of: try #require(state.threads[id])) }
        #expect(try kind("a") == .dispatcher)
        #expect(try kind("b") == .planner)
        #expect(try kind("c") == .agents)
        #expect(try kind("d") == .agents)
    }

    /// LMAS compares bytes: nothing trimmed, no line ending or accent written another way.
    @Test func anAskIsItsBytes() throws {
        func digest(_ json: String) throws -> String {
            let line = #"{"record":{"seq":1,"through":1,"events":[{"type":"ask_opened","ask":"ask-1","about":{"kind":"question"},"by":"x","text":"\#(json)","at_ms":1}]}}"#
            guard case .record(let record) = try ServerLine.read(Data(line.utf8)) else { throw ServerLine.Malformed() }
            var state = WorkshopState()
            try state.apply(record)
            return try #require(state.asks["ask-1"]).textDigest
        }
        func sha(_ bytes: [UInt8]) -> String { Hex.encode(SHA256.hash(data: bytes)) }
        #expect(try digest(#" a\r\n\t "#) == sha([0x20, 0x61, 0x0d, 0x0a, 0x09, 0x20]))
        #expect(try digest("é") == sha([0xc3, 0xa9]))
        #expect(try digest("e\u{301}") == sha([0x65, 0xcc, 0x81]))
        #expect(try digest(#"e\u0301"#) == sha([0x65, 0xcc, 0x81]))
        #expect(try digest(#"\ufeffa\u0000b"#) == sha([0xef, 0xbb, 0xbf, 0x61, 0x00, 0x62]))
    }

    /// A letter to the user counts as delivered only once it has been on screen.
    @Test func seenStopsBeforeTheFirstUnshownLetter() throws {
        var state = WorkshopState()
        try state.apply(record(1, through: 2, message("a", from: "x", to: "user")))
        try state.apply(record(3, message("b", from: "x", to: "y")))
        try state.apply(record(4, through: 6, message("c", from: "x", to: "user")))
        try state.apply(record(7, through: 7))
        #expect(state.seenThrough(shown: []) == nil)
        #expect(state.seenThrough(shown: ["a"]) == 3)
        #expect(state.seenThrough(shown: ["c"]) == nil)
        #expect(state.seenThrough(shown: ["a", "c"]) == 7)
    }

    /// After a restart nothing has been on screen yet: what was delivered before does not hold up what came since.
    @Test func whatIsDeliveredAlreadyDoesNotHoldUpTheRest() throws {
        var state = WorkshopState()
        try state.apply(record(1, message("a", from: "x", to: "user")))
        try state.apply(record(2, .messageHanded(.init(id: "a", atMs: 1))))
        try state.apply(record(3, message("b", from: "x", to: "user")))
        #expect(state.seenThrough(shown: []) == nil)
        #expect(state.seenThrough(shown: ["b"]) == 3)
        try state.apply(record(4, .messageHanded(.init(id: "b", atMs: 1))))
        // Nothing waits: there is nothing to say.
        #expect(state.seenThrough(shown: ["a", "b"]) == nil)
    }

    /// Big logs stay quick: each event costs about the same however many came before.
    @Test(.timeLimit(.minutes(1))) func manyEvents() throws {
        var state = WorkshopState()
        for seq in 1...50_000 as ClosedRange<UInt64> {
            try state.apply(record(seq, message("m\(seq)", from: "a", to: seq % 2 == 0 ? "user" : "b", thread: "m\(seq % 50 + 1)")))
        }
        #expect(state.log.count == 50_000)
        #expect(state.threads.count == 50)
    }
}

struct UntrustedTextTests {
    @Test func hiddenCharactersGo() {
        #expect(UntrustedText.clean("a\u{0}b\u{7}c\u{7F}d\u{9B}e") == "abcde")
        #expect(UntrustedText.clean("合\u{202E}niam\u{202C} 进") == "合niam 进")
        #expect(UntrustedText.clean("a\u{200B}b\u{FEFF}c\u{2066}d\u{E0041}e\u{3164}f") == "abcdef")
        #expect(UntrustedText.clean("a\tb\nc") == "a\tb\nc")
        #expect(UntrustedText.clean("a\r\nb\rc\u{2028}d\u{85}e") == "a\nb\nc\nd\ne")
    }

    @Test func terminalEscapesGoWhole() {
        #expect(UntrustedText.clean("\u{1B}[1;31mred\u{1B}[0m") == "red")
        #expect(UntrustedText.clean("\u{1B}]8;;https://evil\u{7}link\u{1B}]8;;\u{1B}\\!") == "link!")
        // Unfinished ones take nothing with them.
        #expect(UntrustedText.clean("a\u{1B}[") == "a[")
        #expect(UntrustedText.clean("a\u{1B}") == "a")
        #expect(UntrustedText.clean("a\u{1B}]never ends\nb") == "a]never ends\nb")
        #expect(UntrustedText.clean("a\u{1B}[12\nb") == "a[12\nb")
    }

    @Test func ordinaryTextIsLeftAlone() {
        for text in ["给设置页加一个「清空缓存」按钮。", "[link](https://x) **bold** <b>", "👨‍👩‍👧 ❤️ 🇨🇳 👍🏽 1️⃣ ❤️‍🔥 🏳️‍🌈 ☺︎", "e\u{301}", "naïve — “quotes” +30 −4"] {
            #expect(UntrustedText.clean(text) == text)
            #expect(UntrustedText.revealing(text).text == text)
            #expect(UntrustedText.revealing(text).hidden == 0)
        }
    }

    /// Nothing is dropped from a text that will be signed for: it is spelled out.
    @Test func revealingSpellsOutWhatIsHidden() {
        let shown = UntrustedText.revealing("推 main\u{202E}?\u{0}\r\n好\u{200D}")
        #expect(shown.text == "推 main⟨U+202E⟩?⟨U+0000⟩⟨U+000D⟩\n好⟨U+200D⟩")
        #expect(shown.hidden == 4)
        #expect(UntrustedText.revealing("\u{E0041}").text == "⟨U+E0041⟩")
        #expect(UntrustedText.revealing("把 card-1 合进分支 main？\n改动：2 个文件，+30 −4").hidden == 0)
    }

    /// Variation selectors can carry bytes nobody sees, and a digit counts as an emoji to Unicode.
    @Test(arguments: [
        "a\u{FE0F}", "1\u{FE0F}2", "进\u{E0100}", "❤\u{FE0F}\u{FE0F}", "❤\u{FE01}", "\u{FE0F}", "1\u{200D}2", "#\u{200D}*",
        "©\u{200D}®", "1\u{FE0F}\u{200D}🔥", "❤\u{200D}", "😀\u{200D}\u{200D}😀",
    ])
    func selectorsAndJoinersHideNothing(text: String) {
        let shown = UntrustedText.revealing(text)
        #expect(shown.hidden > 0)
        #expect(shown.hidden == shown.text.split(separator: "⟨U+", omittingEmptySubsequences: false).count - 1)
        #expect(UntrustedText.clean(text).unicodeScalars.count < text.unicodeScalars.count)
    }

    @Test func oneLine() {
        #expect(UntrustedText.line("  a\n\n b\tc ") == "a b c")
        #expect(UntrustedText.line(String(repeating: "x", count: 300), limit: 10) == "xxxxxxxxxx…")
    }
}
