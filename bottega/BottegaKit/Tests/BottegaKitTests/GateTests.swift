// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Testing
@testable import BottegaKit

/// Quota gates, and cards and reviews moved to another vendor because of one (LMAS section 8).
struct GateTests {
    private func state(_ events: [Event]...) throws -> WorkshopState {
        var state = WorkshopState()
        for (index, events) in events.enumerated() {
            let seq = UInt64(index + 1)
            try state.apply(Record(seq: seq, through: seq, events: events))
        }
        return state
    }

    private func gate(_ vendor: String, model: String? = nil, _ level: String, at: UInt64 = 1) -> Event {
        .gateChanged(
            .init(
                vendor: vendor, model: model, level: level,
                why: level == "open" ? nil : Why(code: "line", text: "过线了"), untilMs: nil, streak: nil, atMs: at))
    }

    /// LMAS counts in 64 bits and the app goes on counting with what it is told: no more is kept than a number can be.
    @Test func aStreakThatCannotBeIsCutDown() throws {
        func streak(_ told: UInt64?) throws -> UInt64? {
            let closed = Event.gateChanged(
                .init(vendor: "grok", model: nil, level: "closed", why: Why(code: "quota_hit", text: "x"), untilMs: nil, streak: told, atMs: 1))
            return try state([closed]).gatesNotOpen(of: "grok").first?.streak
        }
        #expect(try streak(nil) == 0)
        #expect(try streak(2) == 2)
        #expect(try streak(Wire.farthest) == Wire.farthest)
        #expect(try streak(Wire.farthest + 1) == Wire.farthest)
        #expect(try streak(.max) == Wire.farthest)
        // What the app does with it: a count of this size still goes into an Int.
        #expect(Int(exactly: Wire.farthest) != nil)
    }

    // MARK: As recorded

    @Test func theRecordedGates() throws {
        let state = try Fixtures.state()
        // claude's own gate closed, then opened again; its Fable model's stays half shut.
        #expect(state.gates[.init(vendor: "claude", model: nil)]?.isOpen == true)
        let claude = state.gatesNotOpen(of: "claude")
        #expect(claude.map(\.model) == ["Fable"])
        #expect(claude.first?.level == "started_only")
        #expect(claude.first?.until == 1764086850000)
        let grok = try #require(state.gatesNotOpen(of: "grok").first)
        #expect((grok.level, grok.why?.code, grok.streak, grok.until) == ("closed", "quota_hit", 2, 1764007650000))
        #expect(state.gatesNotOpen(of: "codex").isEmpty)
        // Every change is a row on the dispatcher's timeline, showing that change and not the latest.
        let rows = state.dispatcherRows.compactMap { if case .gate(let id) = $0.kind { id } else { nil } }
        #expect(rows.count == 4)
        #expect(rows.first.flatMap { state.gateChanges[$0] }?.level == "closed")
        #expect(rows.first.flatMap { state.gateChanges[$0] }?.model == nil)
    }

    @Test func theRecordedReassignments() throws {
        let state = try Fixtures.state()
        let four = try #require(state.cards["card-4"])
        #expect(four.isVendorFree)
        #expect((four.vendor, four.reviewer) == ("codex", "grok"))
        #expect(four.reassignments == [.init(what: .card, from: "claude", to: "codex", reviewer: "grok", at: 1764000452000)])
        let one = try #require(state.cards["card-1"])
        #expect(!one.isVendorFree)
        #expect(one.reviewer == "grok")
        #expect(one.reassignments == [.init(what: .review, from: "claude", to: "grok", reviewer: nil, at: 1764000453000)])
        #expect(state.dispatcherRows.contains { $0.kind == .reassigned(card: "card-4", index: 0) })
        #expect(state.dispatcherRows.contains { $0.kind == .reassigned(card: "card-1", index: 0) })
        // Waiting for its vendor to be back does not count as a try.
        #expect(state.cards["card-3"]?.requeues.last?.action == "hold")
        #expect(state.cards["card-3"]?.work == nil)
        // Each time it went back has its own row, not the latest twice.
        let requeues = state.dispatcherRows.compactMap { if case .cardRequeued("card-3", let index) = $0.kind { index } else { nil } }
        #expect(requeues == [0, 1])
    }

    // MARK: Made up

    @Test func aModelsGateDoesNotStandForItsVendors() throws {
        let state = try state([gate("codex", model: "big", "closed")], [gate("codex", "started_only")], [gate("codex", "open", at: 2)])
        #expect(state.gatesNotOpen(of: "codex").map(\.model) == ["big"])
        #expect(state.gates[.init(vendor: "codex", model: nil)]?.at == 2)
    }

    @Test func anUnknownLevelIsKeptAndNotTakenForOpen() throws {
        let state = try state([gate("agy", "half_closed")])
        #expect(state.gatesNotOpen(of: "agy").first?.level == "half_closed")
    }

    @Test func aReassignedCardKeepsItsReviewerWhenNoneIsPickedAgain() throws {
        let created = Event.cardCreated(
            .init(
                card: "c", author: "dispatcher", title: "t", instructions: "i", acceptance: "a", conventions: nil,
                vendor: "claude", continues: nil, reviewer: "codex", thread: "m", vendorFree: true, atMs: 1))
        let moved = Event.cardReassigned(.init(card: "c", vendor: "agy", reviewer: nil, atMs: 2))
        let state = try state([created], [moved])
        #expect((state.cards["c"]?.vendor, state.cards["c"]?.reviewer) == ("agy", "codex"))
        #expect(state.threads["m"]?.rows.last?.kind == .reassigned(card: "c", index: 0))
    }

    @Test func aReviewMovedBeforeItsCardWasSeenIsStillKept() throws {
        let state = try state([.reviewReassigned(.init(card: "c", vendor: "grok", atMs: 1))])
        #expect(state.cards["c"]?.reassignments == [.init(what: .review, from: nil, to: "grok", reviewer: nil, at: 1)])
        #expect(state.cards["c"]?.reviewer == "grok")
    }
}
