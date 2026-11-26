// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import BottegaKit
import BottegaLink
import Foundation
import Testing

/// What goes on inside turns, over the connection.
@MainActor
@Suite(.timeLimit(.minutes(1)))
struct TurnLinkTests {
    private static let began = #""began":{"turn":"turn-7","session":"coder","vendor":"codex","started_at_ms":1}"#
    private static func item(_ n: Int, _ status: String, _ text: String) -> String {
        #""item":{"n":\#(n),"kind":"text","status":"\#(status)","started_at_ms":1,"text":{"text":"\#(text)"}}"#
    }
    private static func page(_ items: ClosedRange<Int>, of last: Int, through: Int? = nil) -> String {
        let list = items.map { #"{"n":\#($0),"kind":"text","status":"done","started_at_ms":1,"text":{"text":"\#($0)"}}"# }
        return #"{"turn":"turn-7","session":"coder","vendor":"codex","started_at_ms":1,"state":"whole","items":[\#(list.joined(separator: ","))],"through":\#(through ?? items.upperBound),"last":\#(last)}"#
    }

    private func rig(_ change: (inout FakeLMAS.Knobs) -> Void) async throws -> Rig {
        let lmas = FakeLMAS()
        lmas.set(change)
        let rig = Rig(lmas: lmas)
        try await rig.registered()
        return rig
    }

    @Test func whatIsRunningComesFirstAndThenGrows() async throws {
        let rig = try await rig {
            $0.turns = [:]
            $0.running = [("turn-7", [Self.began, Self.item(1, "running", "a")])]
        }
        defer { rig.stop() }
        try await rig.until("pushed") { rig.turns["turn-7"]?.items.count == 1 }
        #expect(rig.watching == [["turn-7"]])
        rig.lmas.push("turn-7", #""append":{"n":1,"at":1,"text":"bc"}"#)
        rig.lmas.push("turn-7", #""item":{"broken":true}"#)
        rig.lmas.push("turn-7", #""later":{}"#)
        try await rig.until("grown") { rig.turns["turn-7"]?.items.first?.text.pieces.map(\.text) == ["abc"] }
        rig.lmas.push("turn-7", #""ended":{"whole":true}"#)
        try await rig.until("ended") { rig.turns["turn-7"]?.completeness == .whole }
        #expect(rig.isLive)
    }

    /// LMAS before 4.1: everything else works.
    @Test func anLMASThatDoesNotTellIsFollowedAllTheSame() async throws {
        let rig = try await rig { $0.turns = nil }
        defer { rig.stop() }
        rig.lmas.append(#"{"type":"session_closed","name":"x","at_ms":2}"#)
        try await rig.until("the event") { rig.state.log.count == 2 }
        #expect(rig.watching.isEmpty)
        #expect(rig.isLive)
        #expect(rig.statuses.allSatisfy { if case .retrying = $0 { false } else { true } })
        await #expect(throws: WireError(code: "unknown_request", text: "unknown_request")) { try await rig.client.turn("turn-1") }
    }

    /// An answer to `watch` that cannot be read does not stop the events, and LMAS watches all the same: what it
    /// pushes is taken, and what was growing falls behind when the connection goes.
    @Test func followsAllTheSameWhenWatchCannotBeRead() async throws {
        let rig = try await rig {
            $0.turns = [:]
            $0.garblesWatch = true
            $0.running = [("turn-7", [Self.began])]
        }
        defer { rig.stop() }
        // Pushed right behind the answer: once it is here, the answer has been read.
        try await rig.until("pushed behind the answer") { rig.turns["turn-7"]?.completeness == .growing }
        #expect(rig.isLive)
        #expect(rig.watching.isEmpty)
        rig.lmas.append(#"{"type":"session_registered","name":"a","role":"coder","at_ms":1}"#)
        try await rig.until("a") { rig.state.sessions["a"] != nil }
        // The turn ends while nobody is connected, and the next answer cannot be read either: nothing says it is
        // over, and it does not stay growing for ever.
        rig.lmas.set { $0.running = [] }
        rig.lmas.dropAll()
        try await rig.until("behind") { rig.turns["turn-7"]?.completeness == .behind }
        #expect(rig.watching == [nil])
        // With an answer that can be read, it says what is running.
        rig.lmas.set { $0.garblesWatch = false }
        rig.lmas.dropAll()
        try await rig.until("watching") { rig.watching.contains { $0 != nil } }
    }

    @Test func aNewConnectionWatchesAgainAndWhatEndedMeanwhileIsBehind() async throws {
        let rig = try await rig {
            $0.turns = [:]
            $0.running = [("turn-7", [Self.began, Self.item(1, "running", "a")])]
        }
        defer { rig.stop() }
        try await rig.until("pushed") { rig.turns["turn-7"]?.items.count == 1 }
        rig.lmas.set { $0.running = [("turn-8", [])] }
        rig.lmas.dropAll()
        try await rig.until("watching again") { rig.watching == [["turn-7"], nil, ["turn-8"]] }
        #expect(rig.turns["turn-7"]?.completeness == .behind)
    }

    @Test func aTurnIsFetchedPageByPage() async throws {
        let rig = try await rig { $0.turns = ["turn-7": [Self.page(1...2, of: 3), Self.page(3...3, of: 3)], "turn-1": []] }
        defer { rig.stop() }
        var turn = TurnDetails()
        turn.take(try await rig.client.turn("turn-7"))
        #expect(turn.items.map(\.id) == [1, 2, 3])
        #expect(turn.completeness == .whole)
        await #expect(throws: WireError(code: "no_such_turn", text: "no_such_turn")) { try await rig.client.turn("turn-9") }
        await #expect(throws: WireError(code: "no_details", text: "no_details")) { try await rig.client.turn("turn-1") }
    }

    /// A page that goes nowhere is not asked for again for ever.
    @Test func aPageThatDoesNotGetAnywhereEndsIt() async throws {
        let rig = try await rig { $0.turns = ["turn-7": [Self.page(1...2, of: 9, through: 0)]] }
        defer { rig.stop() }
        await #expect(throws: ServerLine.Malformed.self) { try await rig.client.turn("turn-7") }
    }
}
