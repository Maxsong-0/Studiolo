// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation
import Testing
@testable import BottegaKit

struct OutboxTests {
    private let say = IntentDraft.say(to: "dispatcher", thread: nil, text: "hi", files: [], wantsReply: true)
    private let approve = IntentDraft.approve(ask: "ask-1", textDigest: "d")

    private func signed(_ draft: IntentDraft, number: UInt64 = 1) -> SignedIntent {
        SignedIntent(device: "device-1", number: number, createdAt: 1, kind: draft.kind, body: "{}", signature: "\(number)")
    }

    private func outbox(_ draft: IntentDraft, number: UInt64 = 1) -> Outbox {
        var outbox = Outbox()
        outbox.add(draft, id: "a", at: 1)
        outbox.sign(signed(draft, number: number))
        return outbox
    }

    private func result(_ step: Outbox.Step?) -> Outbox.Settled.Result? {
        if case .settled(let settled)? = step { settled.result } else { nil }
    }

    @Test func settling() {
        var box = outbox(say)
        #expect(result(box.take(.done(message: "msg-7"), at: 1, now: 1)) == .done(message: "msg-7"))
        #expect(box.isEmpty)
        box = outbox(.openLine(.init(project: "p", vendor: "codex", role: "coder", text: "看看")))
        #expect(box.head?.draft.typed?.text == "看看")
        #expect(result(box.take(.done(message: nil, line: "line-1"), at: 1, now: 1)) == .done(message: nil, line: "line-1"))
        box = outbox(say)
        #expect(result(box.take(.already(number: 4), at: 1, now: 1)) == .already(number: 4))
        box = outbox(say)
        let why = Why(code: "not_on_thread", text: "不在这条线上")
        guard case .settled(let settled)? = box.take(.refused(why), at: 1, now: 1) else { Issue.record(); return }
        #expect(settled.result == .refused(why) && settled.isUndone && settled.item.draft.typed?.text == "hi")
        box = outbox(say)
        guard case .refused(let other)? = result(box.take(.other("postponed"), at: 1, now: 1)) else { Issue.record(); return }
        #expect(other.code == "postponed")
    }

    @Test func aGapTakesAnotherNumberAndKeepsItsContent() {
        var box = outbox(say, number: 9)
        #expect(box.take(WireError(code: "gap", text: "x", next: 2)) == .signAgain(number: 2, clockOffset: 0))
        #expect(box.head?.id == "a")
        #expect(box.head?.signed == nil)
    }

    @Test func aConflictIsLookedUpBeforeAnythingIsSentAgain() {
        var box = outbox(say)
        #expect(box.take(WireError(code: "conflict", text: "x", next: 5)) == .catchUp(next: 5))
        // Still the same bytes until the events have been looked through.
        #expect(box.head?.signed == signed(say))
        var done = box
        #expect(result(done.resolveConflict(next: 5, echo: .done(message: "m"))) == .done(message: "m"))
        #expect(box.resolveConflict(next: 5, echo: nil) == .signAgain(number: 5, clockOffset: 0))
        #expect(box.head?.id == "a" && box.head?.signed == nil)
    }

    @Test func renumberingGivesUp() {
        var box = outbox(say)
        for number in 2...4 as ClosedRange<UInt64> {
            #expect(box.take(WireError(code: "gap", text: "x", next: number)) == .signAgain(number: number, clockOffset: 0))
            box.sign(signed(say, number: number))
        }
        guard case .notSent? = result(box.take(WireError(code: "gap", text: "x", next: 5))) else { Issue.record(); return }
        #expect(box.isEmpty)
    }

    /// Signing an approval again would put up a Touch ID prompt nobody asked for.
    @Test func approvalsAreNeverSignedAgainByThemselves() {
        var box = outbox(approve)
        guard case .notSent? = result(box.take(WireError(code: "gap", text: "x", next: 2))) else { Issue.record(); return }
        box = outbox(approve)
        guard case .notSent? = result(box.resolveConflict(next: 2, echo: nil)) else { Issue.record(); return }
        box = outbox(approve)
        guard case .refused? = result(box.take(.refused(.init(code: "in_future", text: "x")), at: 1, now: 5)) else { Issue.record(); return }
    }

    @Test func aClockThatRunsAheadIsSetBackOnce() {
        var box = outbox(say, number: 3)
        let why = Why(code: "in_future", text: "x")
        #expect(box.take(.refused(why), at: 1_000, now: 601_000) == .signAgain(number: 4, clockOffset: -600_000))
        box.sign(signed(say, number: 4))
        #expect(result(box.take(.refused(why), at: 1_000, now: 601_000)) == .refused(why))
    }

    @Test(arguments: ["dropped", "number_too_old", "never_heard_of"])
    func otherErrorsGiveTheWordsBack(code: String) {
        var box = outbox(say)
        guard case .settled(let settled)? = box.take(WireError(code: code, text: "为什么", next: 4)) else { Issue.record(); return }
        #expect(settled.result == .notSent(.init(code: code, text: "为什么")))
        #expect(settled.isUndone)
        #expect(box.isEmpty)
    }

    @Test func onlyWhatWasNeverSignedCanBeTakenBack() {
        var box = outbox(say)
        box.add(say, id: "b", at: 2)
        #expect(box.withdraw("a") == nil)
        #expect(box.notSigned("a") == nil)
        #expect(box.withdraw("b")?.id == "b")
        #expect(box.items.map(\.id) == ["a"])
    }

    @Test func unsignedDecisionsAreNotRemembered() throws {
        var box = Outbox()
        box.add(approve, id: "a", at: 1)
        box.add(say, id: "b", at: 1)
        box.add(.decline(ask: "ask-2"), id: "c", at: 1)
        #expect(box.dropUnsignedDecisions().map(\.item.id) == ["a", "c"])
        #expect(box.items.map(\.id) == ["b"])
        // Nor what was decided about the moment; what means the same later is.
        box = Outbox()
        for (index, draft) in [
            IntentDraft.stopTurn("turn-1"), .setModel(line: "l", session: "s", model: "m", effort: nil),
            .addProject(name: "p", path: "/p"), .removeProject(name: "p"),
            .arrange(line: "l", title: "t", pinned: nil, archived: nil), .markRead(line: "l", through: 3),
            .openLine(.init(project: "p", vendor: "codex", role: "coder")),
        ].enumerated() { box.add(draft, id: "\(index)", at: 1) }
        #expect(box.dropUnsignedDecisions().map(\.item.id) == ["0", "1", "2", "3"])
        #expect(box.items.map(\.id) == ["4", "5", "6"])
        // Signed ones are: they may have been handed in.
        box = outbox(approve)
        #expect(box.dropUnsignedDecisions().isEmpty)
    }

    /// What is on disk after a restart is the very bytes that may have been sent before it.
    @Test func survivesTheDisk() throws {
        var box = outbox(.answer(message: "msg-3", text: "x", files: [.init(name: "a", content: "b")]), number: 7)
        box.add(say, id: "b", at: 2)
        box.add(.arrange(line: "l", title: "t", pinned: nil, archived: true), id: "c", at: 2)
        box.add(.openLine(.init(project: "p", vendor: "codex", role: "coder", text: "x")), id: "d", at: 2)
        let back = try JSONDecoder().decode(Outbox.self, from: JSONEncoder().encode(box))
        #expect(back == box)
        #expect(back.head?.signed?.signature == "7")
    }
}
