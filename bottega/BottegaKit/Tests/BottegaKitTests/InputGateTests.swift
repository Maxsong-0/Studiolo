// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Testing

@testable import BottegaKit

struct InputGateTests {
    private func press(_ timestamp: Double = 10, _ change: (inout InputGate.Input) -> Void = { _ in }) -> InputGate.Input {
        var input = InputGate.Input(isPress: true, sourceProcess: 0, timestamp: timestamp, isInKeyWindow: true)
        change(&input)
        return input
    }

    @Test func aRealPressCountsOnce() throws {
        var gate = InputGate()
        try gate.admit(press(), isInFront: true, now: 10.1)
        #expect(throws: InputGate.Refusal.usedAlready) { try gate.admit(press(), isInFront: true, now: 10.2) }
        try gate.admit(press(10.3), isInFront: true, now: 10.3)
        // Going back to an older press is using it again.
        #expect(throws: InputGate.Refusal.usedAlready) { try gate.admit(press(10.2), isInFront: true, now: 10.4) }
    }

    @Test func nothingElseCounts() {
        func refusal(_ input: InputGate.Input?, isInFront: Bool = true, now: Double = 10.1) -> InputGate.Refusal? {
            var gate = InputGate()
            do throws(InputGate.Refusal) {
                try gate.admit(input, isInFront: isInFront, now: now)
                return nil
            } catch {
                return error
            }
        }
        #expect(refusal(press()) == nil)
        #expect(refusal(nil) == .noEvent)
        #expect(refusal(press(), isInFront: false) == .notInFront)
        #expect(refusal(press { $0.isPress = false }) == .notAPress)
        #expect(refusal(press { $0.sourceProcess = 501 }) == .synthetic)
        #expect(refusal(press { $0.sourceProcess = nil }) == .synthetic)
        #expect(refusal(press { $0.isInKeyWindow = false }) == .otherWindow)
        #expect(refusal(press(), now: 11.5) == .stale)
        #expect(refusal(press(20)) == .stale)
    }

    /// A refused press is not used up, and using one up does not let the next stale one by.
    @Test func refusalsChangeNothing() throws {
        var gate = InputGate()
        #expect(throws: InputGate.Refusal.notInFront) { try gate.admit(press(), isInFront: false, now: 10.1) }
        try gate.admit(press(), isInFront: true, now: 10.1)
        #expect(throws: InputGate.Refusal.noEvent) { try gate.admit(nil, isInFront: true, now: 10.1) }
    }
}
