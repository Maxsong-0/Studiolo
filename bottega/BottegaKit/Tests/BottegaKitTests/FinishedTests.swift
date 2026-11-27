// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Testing
@testable import BottegaKit

/// What the "work finished" notification counts.
struct FinishedTests {
    private func record(_ seq: UInt64, _ events: Event...) -> Record {
        Record(seq: seq, through: seq + UInt64(max(events.count, 1)) - 1, events: events)
    }

    private func created(_ id: String) -> Event {
        .cardCreated(
            .init(
                card: id, author: "dispatcher", title: id, instructions: "", acceptance: "", conventions: nil, vendor: "codex",
                continues: nil, reviewer: nil, thread: nil, vendorFree: nil, after: nil, atMs: 1))
    }

    private func started(_ id: String) -> Event {
        .cardStarted(.init(card: id, worker: "coder-\(id)", worktree: "/w", branch: "b", base: "main", gitDir: nil, freshStart: nil, task: "task-\(id)", atMs: 2))
    }

    private func letter(_ id: String, from: String, to: String) -> Event {
        .messageQueued(
            .init(
                message: .init(
                    id: id, from: from, to: to, kind: "task", thread: id, hop: 0, replyTo: nil, task: nil, body: id, files: nil,
                    atMs: 3, device: from == "user" ? "device-1" : nil)))
    }

    private func turnStarted(_ id: String, _ session: String, pushed: [String]? = nil, at: UInt64 = 4) -> Event {
        .turnStarted(.init(turn: id, session: session, vendor: "local", model: nil, effort: nil, cliVersion: nil, messages: pushed, atMs: at))
    }

    private func turnFinished(_ id: String, _ session: String, _ outcome: String = "completed", at: UInt64 = 9) -> Event {
        .turnFinished(.init(turn: id, session: session, outcome: Why(code: outcome, text: outcome), modelTurns: 1, tools: nil, denied: 0, text: nil, atMs: at))
    }

    private func counted(_ before: [Record], then after: [Record]) throws -> Int {
        var state = WorkshopState()
        for record in before { try state.apply(record) }
        let earlier = state
        for record in after { try state.apply(record) }
        return state.finished(since: earlier)
    }

    @Test func cardsDoneOrFailed() throws {
        let begun = [record(1, created("a"), created("b"), created("c"), started("a"), started("b"))]
        #expect(try counted(begun, then: [record(6, .cardDone(.init(card: "a", result: "ok", atMs: 5)))]) == 1)
        #expect(try counted(begun, then: [record(6, .cardFailed(.init(card: "b", why: Why(code: "x", text: "x"), atMs: 5)))]) == 1)
        let both = record(6, .cardDone(.init(card: "a", result: "ok", atMs: 5)), .cardFailed(.init(card: "b", why: Why(code: "x", text: "x"), atMs: 5)))
        #expect(try counted(begun, then: [both]) == 2)
        // Over before: nothing new.
        #expect(try counted(begun + [both], then: [record(8, created("d"))]) == 0)
    }

    @Test func aTurnThatTookWhatTheUserSaid() throws {
        let asked = [record(1, letter("m1", from: "user", to: "dispatcher"), turnStarted("t1", "dispatcher", pushed: ["m1"]))]
        #expect(try counted(asked, then: [record(3, turnFinished("t1", "dispatcher"))]) == 1)
        // However it ended, short of the user stopping it.
        #expect(try counted(asked, then: [record(3, turnFinished("t1", "dispatcher", "failed"))]) == 1)
        #expect(try counted(asked, then: [record(3, turnFinished("t1", "dispatcher", "stopped_by_user"))]) == 0)
        let askedToStop = record(3, .turnStopRequested(.init(turn: "t1", session: "dispatcher", atMs: 5)))
        #expect(try counted(asked + [askedToStop], then: [record(4, turnFinished("t1", "dispatcher"))]) == 0)
        // Still running.
        #expect(try counted(asked, then: [record(3, letter("m2", from: "dispatcher", to: "user"))]) == 0)
        // Over before: told already.
        let over = asked + [record(3, turnFinished("t1", "dispatcher"))]
        #expect(try counted(over, then: [record(4, letter("m2", from: "dispatcher", to: "user"))]) == 0)
    }

    /// A letter handed over while the turn ran counts as well as one pushed into it.
    @Test func aLetterHandedOverWhileItRan() throws {
        let running = [record(1, turnStarted("t1", "line-1", at: 4), letter("m1", from: "user", to: "line-1"), .messageHanded(.init(id: "m1", atMs: 6)))]
        #expect(try counted(running, then: [record(4, turnFinished("t1", "line-1", at: 9))]) == 1)
        // Handed over after it had ended: for another turn.
        let late = [record(1, turnStarted("t1", "line-1", at: 4), letter("m1", from: "user", to: "line-1"))]
        let after = [record(3, turnFinished("t1", "line-1", at: 9), .messageHanded(.init(id: "m1", atMs: 10)))]
        #expect(try counted(late, then: after) == 0)
    }

    @Test func turnsTheUserHadNoPartIn() throws {
        // Agents talking among themselves.
        let between = [record(1, letter("m1", from: "dispatcher", to: "coder"), turnStarted("t1", "coder", pushed: ["m1"]))]
        #expect(try counted(between, then: [record(3, turnFinished("t1", "coder"))]) == 0)
        // What the user said to someone else.
        let elsewhere = [record(1, letter("m1", from: "user", to: "dispatcher"), turnStarted("t1", "coder", pushed: ["m1"]))]
        #expect(try counted(elsewhere, then: [record(3, turnFinished("t1", "coder"))]) == 0)
        // A card's agent: its card says when it is over.
        let carded = [record(1, created("a"), started("a"), letter("m1", from: "user", to: "coder-a"), turnStarted("t1", "coder-a", pushed: ["m1"]))]
        #expect(try counted(carded, then: [record(5, turnFinished("t1", "coder-a"))]) == 0)
    }

    /// From LMAS's events: card-1 done; card-2, card-3 and card-5 failed. turn-1 is card-1's coder's, turn-2 was asked
    /// to stop and turn-3 stopped by the user.
    @Test func theRecordedOnes() throws {
        #expect(try Fixtures.state().finished(since: WorkshopState()) == 4)
    }
}
