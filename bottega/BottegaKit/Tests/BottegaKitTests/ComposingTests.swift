// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Testing

@testable import BottegaKit

struct ComposingTests {
    /// A dispatcher, an agent, and a line where the agent has asked the user something.
    private func state(asking: Bool = true, agentLeft: Bool = false) throws -> WorkshopState {
        var state = WorkshopState()
        var events: [Event] = [
            .sessionRegistered(.init(name: "coder", role: "coder", atMs: 1)),
            .sessionRegistered(.init(name: "dispatcher", role: "dispatcher", atMs: 2)),
            .sessionRegistered(.init(name: "user", role: "user", atMs: 2)),
            .messageQueued(.init(message: letter("m1", "user", "dispatcher", "task"))),
            .messageQueued(.init(message: letter("m2", "dispatcher", "coder", "task"))),
        ]
        if asking {
            events.append(.messageQueued(.init(message: letter("m3", "coder", "user", "question"))))
            events.append(.messageQueued(.init(message: letter("m4", "dispatcher", "user", "question"))))
        }
        if agentLeft { events.append(.sessionClosed(.init(name: "coder", atMs: 9))) }
        try state.apply(Record(seq: 1, through: UInt64(events.count), events: events))
        return state
    }

    private func letter(_ id: String, _ from: String, _ to: String, _ kind: String, replyTo: String? = nil) -> WireMessage {
        WireMessage(id: id, from: from, to: to, kind: kind, thread: "m1", hop: 0, replyTo: replyTo, task: nil, body: id, files: nil, atMs: 3, device: nil)
    }

    private func drafts(_ key: DraftKey, _ change: (inout UnsentDraft) -> Void = { _ in }) -> Drafts {
        var drafts = Drafts()
        drafts[key].text = "hi"
        change(&drafts[key])
        return drafts
    }

    @Test func theDispatcherIsFirst() throws {
        let state = try state()
        #expect(state.recipients(for: .dispatcher).map(\.name) == ["dispatcher", "coder"])
        #expect(drafts(.dispatcher).intent(for: .dispatcher, in: state)
            == .say(to: "dispatcher", thread: nil, text: "hi", files: [], wantsReply: true))
        // Whatever was ticked elsewhere: a new line always wants a reply.
        let straight = drafts(.dispatcher) { $0.recipient = "coder"; $0.wantsReply = false }
        #expect(straight.intent(for: .dispatcher, in: state) == .say(to: "coder", thread: nil, text: "hi", files: [], wantsReply: true))
    }

    @Test func aLineAnswersTheOldestQuestionUnlessToldToInterject() throws {
        let state = try state()
        #expect(drafts(.line("m1")).intent(for: .line("m1"), in: state) == .answer(message: "m3", text: "hi", files: []))
        let interjecting = drafts(.line("m1")) { $0.interjects = true; $0.recipient = "coder"; $0.wantsReply = false }
        #expect(interjecting.intent(for: .line("m1"), in: state) == .say(to: "coder", thread: "m1", text: "hi", files: [], wantsReply: false))
        #expect(drafts(.question("m4")).intent(for: .question("m4"), in: state) == .answer(message: "m4", text: "hi", files: []))
    }

    @Test func withoutAQuestionALineInterjects() throws {
        let state = try state(asking: false)
        guard case .say(_, "m1", "hi", [], true) = drafts(.line("m1")).intent(for: .line("m1"), in: state) else {
            Issue.record("not an interjection")
            return
        }
        #expect(drafts(.question("m3")).intent(for: .question("m3"), in: state) == nil)
    }

    /// Words meant for one agent never go to another because the first one left.
    @Test func someoneWhoLeftIsNotSwapped() throws {
        let state = try state(asking: false, agentLeft: true)
        let picked = drafts(.line("m1")) { $0.recipient = "coder" }
        #expect(picked.target(of: .line("m1"), in: state) == nil)
        #expect(picked.intent(for: .line("m1"), in: state) == nil)
        #expect(drafts(.line("m1")).target(of: .line("m1"), in: state) == .interject("dispatcher"))
    }

    @Test func anAnsweredQuestionIsNotAnsweredAgain() throws {
        var state = try state()
        try state.apply(Record(seq: 8, through: 8, events: [
            .messageQueued(.init(message: letter("m5", "user", "coder", "result", replyTo: "m3")))
        ]))
        #expect(drafts(.question("m3")).intent(for: .question("m3"), in: state) == nil)
        #expect(drafts(.line("m1")).intent(for: .line("m1"), in: state) == .answer(message: "m4", text: "hi", files: []))
    }

    @Test func whatComesBackIsInFrontOfWhatWasTypedSince() {
        var drafts = drafts(.line("m1")) { $0.recipient = "coder"; $0.interjects = true; $0.wantsReply = false }
        drafts.cleared(.line("m1"))
        #expect(drafts[.line("m1")].isBlank)
        #expect(drafts[.line("m1")].recipient == "coder")
        #expect(drafts[.line("m1")].interjects)
        #expect(!drafts[.line("m1")].wantsReply)
        drafts[.line("m1")].text = "next"
        let file = TextFile(name: "a", content: "b")
        drafts.putBack("hi", [file], .init(code: "not_on_thread", text: "no"), in: .line("m1"))
        #expect(drafts[.line("m1")].text == "hi\n\nnext")
        #expect(drafts[.line("m1")].files == [file])
        #expect(drafts[.line("m1")].problem?.code == "not_on_thread")
        #expect(drafts[.line("m1")].recipient == "coder")
        #expect(drafts[.dispatcher] == UnsentDraft())
    }

    @Test func leftoversGoHome() throws {
        #expect(IntentDraft.say(to: "a", thread: "m1", text: "", files: [], wantsReply: true).home == .line("m1"))
        #expect(IntentDraft.say(to: "a", thread: nil, text: "", files: [], wantsReply: true).home == .dispatcher)
        #expect(IntentDraft.answer(message: "m3", text: "", files: []).home == .question("m3"))
        let state = try state()
        #expect(state.reachable(.question("m3")) == .question("m3"))
        #expect(state.reachable(.question("m2")) == .line("m1"))
        #expect(state.reachable(.question("gone")) == .dispatcher)
        #expect(state.reachable(.line("m1")) == .line("m1"))
        #expect(state.reachable(.line("gone")) == .dispatcher)
        let answer = IntentDraft.answer(message: "m3", text: "", files: [])
        #expect(state.belongs(answer, above: .line("m1")))
        #expect(state.belongs(answer, above: .question("m3")))
        #expect(!state.belongs(answer, above: .dispatcher))
        #expect(!state.belongs(.decline(ask: "a"), above: .dispatcher))
        #expect(state.linesWaiting(state.waiting) == ["m1"])
    }
}
