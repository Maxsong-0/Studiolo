// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import SwiftUI
import Testing

/// How the cards page's Board and List come in (Bottega-5 §7, canvas version 10).
struct CardsMotionTests {
    @Test func theListRises28msARowUpTo280() {
        #expect(CardsMotion.listDelay(row: 0) == 0)
        #expect(abs(CardsMotion.listDelay(row: 1) - 0.028) < 1e-9)
        #expect(abs(CardsMotion.listDelay(row: 9) - 0.252) < 1e-9)
        #expect(abs(CardsMotion.listDelay(row: 11) - 0.28) < 1e-9)
        #expect(abs(CardsMotion.listDelay(row: 500) - 0.28) < 1e-9)
    }

    @Test func theBoardRises60msAColumnAnd45msACardUpToHalfASecond() {
        #expect(CardsMotion.boardDelay(column: 0, card: 0) == 0)
        #expect(abs(CardsMotion.boardDelay(column: 1, card: 0) - 0.06) < 1e-9)
        #expect(abs(CardsMotion.boardDelay(column: 0, card: 1) - 0.045) < 1e-9)
        #expect(abs(CardsMotion.boardDelay(column: 2, card: 3) - 0.255) < 1e-9)
        #expect(abs(CardsMotion.boardDelay(column: 4, card: 10) - 0.5) < 1e-9)
    }

    @Test func withLessMotionTheSwapOnlyFades() {
        #expect(CardsMotion.switching(reduceMotion: true) == .easeOut(duration: 0.2))
        #expect(CardsMotion.switching(reduceMotion: false) == .smooth(duration: 0.25))
    }

    @Test func theBoardRisesOnceAndOnlyForASwitch() {
        var cue = CardsMotion.Cue()
        // Opened the way it was left: there at once.
        #expect(!cue.isDue)
        cue.layoutSwitched()
        #expect(cue.isDue)
        // Shown again later, after the link came back, say: at once.
        cue.boardRose()
        #expect(!cue.isDue)
        // Switched, then gone to another page before the board showed: back on the cards page, at once.
        cue.layoutSwitched()
        cue.pageChanged()
        #expect(!cue.isDue)
    }
}
