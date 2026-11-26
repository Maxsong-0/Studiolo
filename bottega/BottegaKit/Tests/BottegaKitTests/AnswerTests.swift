// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Testing
@testable import BottegaKit

/// Cards that wait for other cards, and questions nobody answers (LMAS sections 4 and 6).
struct AnswerTests {
    private func state(_ events: [Event]...) throws -> WorkshopState {
        var state = WorkshopState()
        for (index, events) in events.enumerated() {
            let seq = UInt64(index + 1)
            try state.apply(Record(seq: seq, through: seq, events: events))
        }
        return state
    }

    private func card(_ id: String, after: [String]? = nil) -> Event {
        .cardCreated(
            .init(
                card: id, author: "dispatcher", title: "t", instructions: "i", acceptance: "a", conventions: nil,
                vendor: "codex", continues: nil, reviewer: nil, thread: "m-\(id)", vendorFree: nil, after: after, atMs: 1))
    }

    private func question(_ id: String, thread: String) -> Event {
        .messageQueued(
            .init(
                message: .init(
                    id: id, from: "coder-c", to: "dispatcher", kind: "question", thread: thread, hop: 1, replyTo: nil,
                    task: nil, body: "?", files: nil, atMs: 1, device: nil)))
    }

    // MARK: As recorded

    @Test func theRecordedWaits() throws {
        let state = try Fixtures.state()
        let five = try #require(state.cards["card-5"])
        #expect(five.after == ["card-4"])
        guard case .failed(let why, _) = five.end else { Issue.record("card-5 did not fail"); return }
        #expect(why.code == "dependency_failed")
        #expect(state.cards["card-4"]?.after == [])
    }

    @Test func theRecordedReminders() throws {
        let state = try Fixtures.state()
        let three = try #require(state.cards["card-3"])
        #expect(three.reminders == [
            .init(question: "msg-16", level: 1, at: 1764000551000), .init(question: "msg-16", level: 2, at: 1764000551500),
        ])
        guard case .failed(let why, _) = three.end else { Issue.record("card-3 did not fail"); return }
        #expect(why.code == "unanswered")
        // Neither the question nor the card has a line in the recording: the dispatcher's timeline still shows them.
        #expect(three.thread == nil)
        let rows = state.dispatcherRows.compactMap { if case .answerReminded("card-3", let index) = $0.kind { index } else { nil } }
        #expect(rows == [0, 1])
        let failed = state.log.lazy.compactMap { if case .messageFailed(let e) = $0.event, e.id == "msg-16" { e.why } else { nil } }.first
        #expect(failed?.code == "unanswered")
    }

    // MARK: Made up

    @Test func aReminderShowsWhereTheQuestionWasAsked() throws {
        let reminded = Event.answerReminded(.init(card: "c", question: "q", level: 1, atMs: 2))
        let state = try state([card("c")], [question("q", thread: "elsewhere")], [reminded])
        #expect(state.threads["elsewhere"]?.rows.last?.kind == .answerReminded(card: "c", index: 0))
        #expect(state.threads["m-c"]?.rows.contains { $0.kind == .answerReminded(card: "c", index: 0) } != true)
        #expect(state.dispatcherRows.contains { $0.kind == .answerReminded(card: "c", index: 0) })
    }
}
