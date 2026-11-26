// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation
import Testing
@testable import BottegaKit

/// `turns.jsonl`, LMAS's recording of one connection's `watch` and `turn`.
struct RecordedTurnTests {
    private struct Said: Decodable { var from: String; var line: JSON }

    private func said(by who: String) throws -> [JSON] {
        try Fixtures.lines("turns.jsonl").map { try JSONDecoder().decode(Said.self, from: $0) }.filter { $0.from == who }.map(\.line)
    }

    private func read(_ line: JSON) throws -> ServerLine { try ServerLine.read(try JSONEncoder().encode(line)) }

    private func pushed() throws -> [Live] {
        try said(by: "lmas").compactMap { if case .live(let live) = try read($0) { live } else { nil } }
    }

    private func pages() throws -> [TurnPage] {
        try said(by: "lmas").compactMap { line in
            guard line["ok"]?["items"] != nil, case .reply(let reply) = try read(line) else { return nil }
            return try reply.read()
        }
    }

    @Test func everyLineIsRead() throws {
        for line in try said(by: "lmas") {
            let read = try read(line)
            #expect(read != .unknown)
            if case .live(let live) = read { #expect(live.what != .unknown && !live.turn.isEmpty) }
        }
        #expect(try pushed().count == 20)
        #expect(try pages().map(\.through) == [6, 11])
        let errors = try said(by: "lmas").compactMap { if case .error(_, let error) = try read($0) { error.code } else { nil } }
        #expect(errors == ["no_such_turn", "no_details", "already_watching"])
    }

    @Test func whatBottegaSaysIsWhatThisCodeWrites() throws {
        let mine: [(UInt64, Request)] = [
            (1, .watch), (2, .turn("turn-1", from: 1)), (3, .turn("turn-1", from: 7)), (4, .turn("turn-99", from: 1)),
            (5, .turn("turn-2", from: 1)), (6, .watch),
        ]
        let written = try mine.map { try JSONDecoder().decode(JSON.self, from: ClientLine.request(id: $0.0, $0.1)) }
        #expect(try written == said(by: "bottega"))
    }

    /// Put together from what was pushed, it is what `turn` gives.
    @Test func pushedAndFetchedAreTheSame() throws {
        var details: [String: TurnDetails] = [:]
        for live in try pushed() { details[live.turn, default: TurnDetails()].take(live.what) }
        var fetched = TurnDetails()
        fetched.take(try pages())
        #expect(details["turn-1"] == fetched)
        #expect(fetched.items.map(\.id) == Array(1...11))
        // The dispatcher's question to its model: a title, and no thoughts.
        let asked = fetched.items[10]
        #expect(asked.item.kind == "reasoning" && asked.item.title == "第 2 次问模型" && asked.text.pieces.isEmpty)
        #expect(fetched.completeness == .whole)
        #expect(fetched.head == TurnHead(turn: "turn-1", session: "coder-card-1", vendor: "codex", model: "gpt-5.2-codex", effort: "high", startedAtMs: 1764000050000))
        #expect(details["turn-2"]?.completeness == .behind)
    }

    /// The preview of each, right before it ends, agrees with how it ends.
    @Test func thePreviewAgreesWithTheEnd() throws {
        var details = TurnDetails()
        var compared = 0
        for live in try pushed() where live.turn == "turn-1" {
            if case .item(let item) = live.what, item.isOver, let before = details.items.first(where: { $0.id == item.n }) {
                #expect(before.text == item.text.map(GrownText.init))
                compared += 1
            }
            details.take(live.what)
        }
        #expect(compared == 2)
    }

    @Test func fieldsComeThrough() throws {
        let items = try pages().flatMap(\.items)
        #expect(items.map(\.kind) == ["reasoning", "read", "search", "command", "edit", "tool", "notice", "plan", "text", "coordinator", "reasoning"])
        #expect(Set(items.map(\.status)) == ["done", "failed", "denied", "interrupted"])
        #expect(items[1].paths == ["/Users/max/lmas-worktrees/card-1/src/settings.rs"])
        #expect(items[2].title == "找 cache")
        #expect(items[3].exitCode == 0)
        #expect(items[3].input?.text == "cargo test")
        #expect(items[3].text?.cut == .init(at: 16, bytes: 300_000))
        #expect(items[4].files.map(\.change) == ["modify", "move"])
        #expect(items[4].files[0].diff?.text.hasPrefix("@@") == true)
        #expect(items[4].files[1].to?.hasSuffix("new.rs") == true)
        #expect(items[6].code == "denied")
        #expect(items[7].steps.map(\.status) == ["done", "running", "pending"])
        #expect(items[9].tool == "send_message")
        #expect(items[9].endedAt == 1764000051040)
    }

    @Test func turnsAreInTheState() throws {
        let state = try Fixtures.state()
        let turn = try #require(state.turns["turn-1"])
        #expect(turn.session == "coder-card-1" && turn.model == "gpt-5.2-codex" && turn.cliVersion == "0.171.0")
        #expect(turn.messages == ["msg-9"])
        // The tools' names are the vendor's: `send_message` is not a Swift name to be turned into `sendMessage`.
        #expect(turn.end?.tools == ["exec_command": 2, "send_message": 1])
        #expect(turn.end?.outcome.code == "completed" && turn.end?.denied == 1 && turn.end?.modelTurns == 4)
        #expect(state.turns["turn-2"]?.end?.outcome.code == "restarted")
        #expect(state.vendors["coder-card-1"] == "codex")
    }
}

struct GrownTextTests {
    private func pieces(_ text: GrownText) -> [String] { text.pieces.map { "\($0.at):\($0.text)" } }

    @Test func growsInBytes() {
        var text = GrownText(CutText(text: "先看"))
        text.append("设置", at: 6)
        #expect(pieces(text) == ["0:先看设置"] && text.end == 12 && text.isWhole)
    }

    @Test func whatIsMissingIsCounted() {
        var text = GrownText(CutText(text: "ab"))
        text.append("ef", at: 4)
        text.append("gh", at: 6)
        text.append("z", at: 100)
        #expect(pieces(text) == ["0:ab", "4:efgh", "100:z"])
        #expect(text.pieces.indices.map(text.missing(before:)) == [0, 2, 92])
        #expect(!text.isWhole)
        var late = GrownText()
        late.append("x", at: 5)
        #expect(late.missing(before: 0) == 5 && !late.isWhole)
    }

    @Test func whatIsThereIsNotTakenTwice() {
        var text = GrownText(CutText(text: "abcd"))
        text.append("cdef", at: 2)
        #expect(pieces(text) == ["0:abcdef"])
        text.append("bc", at: 1)
        text.append("", at: 50)
        #expect(pieces(text) == ["0:abcdef"])
        // In the middle of a character: nothing to make of it.
        var wide = GrownText(CutText(text: "先"))
        wide.append("看看", at: 1)
        #expect(pieces(wide) == ["0:先"])
    }

    @Test func aCutOneStartsInTwo() {
        #expect(pieces(GrownText(CutText(text: "先end", cut: .init(at: 3, bytes: 10)))) == ["0:先", "13:end"])
        #expect(pieces(GrownText(CutText(text: "end", cut: .init(at: 0, bytes: 10)))) == ["10:end"])
        // A cut that cannot be: all of it is shown all the same.
        for at: UInt64 in [1, 99, .max] {
            #expect(pieces(GrownText(CutText(text: "先end", cut: .init(at: at, bytes: 10)))) == ["0:先end"])
        }
        #expect(pieces(GrownText(CutText(text: "ab", cut: .init(at: 1, bytes: .max)))) == ["0:ab"])
        var far = GrownText(CutText(text: "ab"))
        far.append("c", at: .max)
        #expect(pieces(far) == ["0:ab"])
    }

    @Test func onlyItsStartAndItsEndAreKept() {
        var text = GrownText()
        var sent = ""
        for line in 0..<60_000 {
            let next = "第 \(line) 行\n"
            text.append(next, at: UInt64(sent.utf8.count))
            sent += next
        }
        let kept = text.pieces.reduce(0) { $0 + $1.text.utf8.count }
        #expect(kept >= GrownText.keptHead + GrownText.keptTail - 8 && kept <= (GrownText.keptHead + GrownText.keptTail) * 3 / 2)
        #expect(text.pieces.count == 2)
        #expect(sent.hasPrefix(text.pieces[0].text) && sent.hasSuffix(text.pieces[1].text))
        #expect(text.end == UInt64(sent.utf8.count))
        #expect(Int(text.missing(before: 1)) + kept == sent.utf8.count)
    }

    @Test func theTailStartsWithALine() {
        let text = GrownText(CutText(text: "one\ntwo\nthree\n"))
        #expect(text.tail(100) == "one\ntwo\nthree\n")
        #expect(text.tail(9) == "three\n")
        #expect(text.tail(5) == "hree\n")
        #expect(GrownText(CutText(text: "先看看")).tail(4) == "看")
        #expect(GrownText().tail(4) == "")
    }
}

struct TurnDetailsTests {
    private let head = TurnHead(turn: "turn-1", session: "s", vendor: "codex", startedAtMs: 1)

    private func page(_ state: String, _ items: String, through: Int, last: Int) throws -> TurnPage {
        try Wire.decoder.decode(
            TurnPage.self,
            from: Data(#"{"turn":"turn-1","session":"s","vendor":"codex","started_at_ms":1,"state":"\#(state)","items":[\#(items)],"through":\#(through),"last":\#(last)}"#.utf8))
    }

    @Test(arguments: [
        ("/bin/zsh -lc 'cat README.md && ls'", "cat README.md && ls"), (#"/bin/bash -c "ls""#, "ls"),
        ("/bin/sh -c ls", "ls"), ("cargo test", "cargo test"), ("/bin/zsh -lc ''", "''"), ("/bin/zsh -lc ", "/bin/zsh -lc "),
        ("/bin/zsh script", "/bin/zsh script"), ("/bin/zsh -lc 'ls\"", "'ls\""),
    ])
    func aCommandInShortHasNoShellAroundIt(command: String, short: String) {
        var item = TurnItem(n: 1, kind: "command", status: "running")
        #expect(item.commandInShort == nil)
        item.input = CutText(text: command)
        #expect(item.commandInShort == short)
    }

    /// Anything that is no place from 1 on is not taken, whatever it says about itself.
    @Test(arguments: ["-9223372036854775808", "9223372036854775807", "-1", "0", "18446744073709551616", "1.5"])
    func aPlaceThatCannotBeIsNoPlace(n: String) throws {
        func live(_ what: String) throws -> Live {
            guard case .live(let live) = try ServerLine.read(Data(#"{"live":{"turn":"turn-1",\#(what)}}"#.utf8)) else { throw ServerLine.Malformed() }
            return live
        }
        #expect(try live(#""item":{"n":\#(n),"kind":"text","status":"running"}"#).what == .unknown)
        #expect(try live(#""append":{"n":\#(n),"at":0,"text":"x"}"#).what == .unknown)
        #expect(try page("whole", #"{"n":\#(n),"kind":"text","status":"done"}"#, through: 0, last: 0).items == [])
        var turn = TurnDetails()
        turn.take(.began(head))
        turn.take(.item(TurnItem(n: 1, kind: "text", status: "running")))
        turn.take(.item(TurnItem(n: .min, kind: "text", status: "done")))
        turn.take(.append(n: .min, at: 0, text: "x"))
        turn.take(.append(n: .max, at: 0, text: "x"))
        #expect(turn.items.map(\.id) == [.min, 1])
    }

    @Test func anItemReplacesTheOneBefore() {
        var turn = TurnDetails()
        turn.take(.began(head))
        turn.take(.item(TurnItem(n: 1, kind: "command", status: "running")))
        turn.take(.append(n: 1, at: 0, text: "ab"))
        turn.take(.append(n: 9, at: 0, text: "nobody's"))
        #expect(turn.current?.id == 1)
        turn.take(.item(TurnItem(n: 1, kind: "command", status: "done", text: CutText(text: "xyz"))))
        #expect(turn.items.map { $0.text.pieces.map(\.text) } == [["xyz"]])
        #expect(turn.current == nil)
        turn.take(.append(n: 1, at: 3, text: "late"))
        #expect(turn.items[0].text.end == 3)
    }

    @Test func theyStayInOrderWhateverOrderTheyComeIn() {
        var turn = TurnDetails()
        for n in [3, 1, 2, 5, 2] { turn.take(.item(TurnItem(n: n, kind: "text", status: "done"))) }
        #expect(turn.items.map(\.id) == [1, 2, 3, 5])
        turn.take(.append(n: 5, at: 0, text: "x"))
        turn.take(.item(TurnItem(n: 5, kind: "text", status: "running")))
        turn.take(.append(n: 5, at: 0, text: "x"))
        #expect(turn.items.last?.text.end == 1)
    }

    @Test func completenessFollowsWhatIsKnown() throws {
        var turn = TurnDetails()
        #expect(turn.completeness == .behind)
        turn.take(.began(head))
        #expect(turn.completeness == .growing)
        // Fetched while it is being pushed: what is pushed is further along.
        turn.take(.item(TurnItem(n: 1, kind: "text", status: "running")))
        turn.take([try page("running", "", through: 0, last: 0)])
        #expect(turn.items.count == 1 && turn.completeness == .growing)
        turn.fellBehind()
        #expect(turn.completeness == .behind)
        turn.take([try page("running", #"{"n":1,"kind":"text","status":"running","started_at_ms":1}"#, through: 1, last: 1)])
        #expect(turn.completeness == .behind)
        turn.take([try page("partial", #"{"n":1,"kind":"text","status":"done","started_at_ms":1},{"kind":"lost"},3,{"n":4,"kind":["?"],"status":7,"started_at_ms":-1,"files":[{}],"text":{"text":"x"}}"#, through: 4, last: 4)])
        #expect(turn.items.map(\.id) == [1, 4] && turn.completeness == .partial)
        #expect(turn.items[1].item.kind == "" && turn.items[1].text.end == 1)
        turn.fellBehind()
        #expect(turn.completeness == .partial)
        turn.take(.ended(whole: true))
        #expect(turn.completeness == .whole)
        turn.take([])
        #expect(turn.completeness == .whole)
    }
}

/// The bars in the lanes.
struct TurnLaneTests {
    private func state(_ events: Event...) throws -> WorkshopState {
        var state = WorkshopState()
        for (index, event) in events.enumerated() {
            try state.apply(Record(seq: UInt64(index + 1), through: UInt64(index + 1), events: [event]))
        }
        return state
    }

    private func letter(_ id: String, _ from: String, _ to: String, _ kind: String, at: UInt64, replyTo: String? = nil) -> Event {
        .messageQueued(.init(message: WireMessage(
            id: id, from: from, to: to, kind: kind, thread: "m1", hop: 0, replyTo: replyTo, task: nil, body: "", files: nil,
            atMs: at, device: nil)))
    }

    private func start(_ turn: String, _ session: String, at: UInt64, _ messages: [String]? = nil) -> Event {
        .turnStarted(.init(turn: turn, session: session, vendor: "codex", model: nil, cliVersion: nil, messages: messages, atMs: at))
    }

    private func end(_ turn: String, _ session: String, at: UInt64) -> Event {
        .turnFinished(.init(
            turn: turn, session: session, outcome: Why(code: "completed", text: ""), modelTurns: 1, tools: nil, denied: 0, text: nil,
            atMs: at))
    }

    private func bars(_ state: WorkshopState) -> [String] {
        let lanes = state.lanes(of: state.threads["m1"]!)
        return lanes.working.map { "\(lanes.members[$0.lane]) \($0.top)–\($0.bottom.map { "\($0)" } ?? "")" }
    }

    @Test func aPushedTurnRunsFromItsLetterToItsEnd() throws {
        // Two turns for the TASK: with a question in between, and after its answer.
        let state = try state(
            letter("m1", "d", "c", "task", at: 10), start("t1", "c", at: 11, ["m1"]),
            letter("m2", "c", "d", "question", at: 12), end("t1", "c", at: 13),
            letter("m3", "d", "c", "result", at: 20, replyTo: "m2"), start("t2", "c", at: 21, ["m3"]),
            letter("m4", "c", "d", "result", at: 22, replyTo: "m1"))
        #expect(bars(state) == ["c 0.5–2.0", "c 2.5–"])
        #expect(state.runningTurn(of: "c")?.id == "t2")
        #expect(state.turns(with: state.messages["m1"]!).map(\.id) == ["t1"])
    }

    /// LMAS counts in milliseconds: the last letter and the end of the turn it came from can be in the same one.
    @Test func aLetterSentAsTheTurnEndsIsInIt() throws {
        let state = try state(
            letter("m1", "d", "c", "task", at: 10), start("t1", "c", at: 11, ["m1"]),
            letter("m2", "c", "d", "result", at: 13, replyTo: "m1"), end("t1", "c", at: 13))
        #expect(bars(state) == ["c 0.5–2.0"])
    }

    @Test func aTurnThatFetchedItsLettersIsFoundByThem() throws {
        let state = try state(
            letter("m1", "u", "d", "task", at: 10), start("t1", "d", at: 15),
            .messageHanded(.init(id: "m1", atMs: 16)), letter("m2", "d", "c", "task", at: 17), end("t1", "d", at: 18),
            // Something else it did later.
            start("t2", "d", at: 30), end("t2", "d", at: 31))
        #expect(bars(state) == ["d 1.0–2.0"])
        #expect(state.turns(with: state.messages["m1"]!).map(\.id) == ["t1"])
        #expect(state.turns(of: "d").map(\.id) == ["t1", "t2"])
    }

    /// The whole turn between two letters still shows.
    @Test func aShortTurnIsNotNothing() throws {
        let state = try state(
            letter("m1", "d", "c", "task", at: 10), start("t1", "c", at: 11, ["m1"]), end("t1", "c", at: 12),
            letter("m2", "d", "c", "task", at: 20))
        #expect(bars(state) == ["c 0.5–1.0"])
    }

    /// No turns known of it: from the TASK to what answers it.
    @Test func withoutTurnsTheLettersTell() throws {
        let state = try state(
            letter("m1", "d", "p", "task", at: 10), .messageHanded(.init(id: "m1", atMs: 11)),
            letter("m2", "p", "d", "result", at: 12, replyTo: "m1"), start("t1", "d", at: 1), end("t1", "d", at: 2))
        #expect(bars(state) == ["p 0.5–1.5"])
    }

    /// LMAS only began to tell turns: what came before keeps its bars, and gets no second one.
    @Test func whatCameBeforeTheFirstTurnKeepsItsBar() throws {
        let state = try state(
            letter("m1", "d", "c", "task", at: 10), .messageHanded(.init(id: "m1", atMs: 11)),
            letter("m2", "c", "d", "result", at: 12, replyTo: "m1"),
            letter("m3", "d", "c", "task", at: 20), .messageHanded(.init(id: "m3", atMs: 20)), start("t1", "c", at: 21, ["m3"]),
            letter("m4", "d", "c", "task", at: 22), letter("m5", "c", "d", "result", at: 23, replyTo: "m3"), end("t1", "c", at: 24))
        #expect(bars(state) == ["c 0.5–1.5", "c 2.5–5.0"])
    }

    @Test func anEndWithoutAStartIsTaken() throws {
        let state = try state(letter("m1", "d", "c", "task", at: 10), end("t1", "c", at: 12))
        #expect(state.turns["t1"]?.startedAt == nil && state.runningTurn(of: "c") == nil)
        #expect(bars(state) == [])
    }
}
