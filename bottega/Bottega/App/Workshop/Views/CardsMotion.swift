// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import SwiftUI

/// How the cards page moves between Board and List (vault Bottega-5 §7, canvas version 10): the one switched to
/// comes in a card or a row at a time, rising a little out of focus (the canvas's `rise2`). With less motion, the
/// page and its rows only fade.
enum CardsMotion {
    /// The page swap itself.
    static func switching(reduceMotion: Bool) -> Animation {
        reduceMotion ? .easeOut(duration: 0.2) : .smooth(duration: 0.25)
    }

    /// The list: 28 ms a row, and none waits longer than 280 ms, so a long list is not still coming in a second later.
    static func listDelay(row: Int) -> Double { min(Double(row) * 0.028, 0.28) }

    /// The board: a column 60 ms after the one to its left, a card 45 ms after the one above it. The canvas sets no
    /// end; half a second is one here, for a long column.
    static func boardDelay(column: Int, card: Int) -> Double { min(Double(column) * 0.06 + Double(card) * 0.045, 0.5) }
}

/// Shown at the start hidden, then rising into place after `delay`; with less motion only fading in.
struct RisesIn: ViewModifier {
    let delay: Double
    let hasRisen: Bool
    let reduceMotion: Bool

    func body(content: Content) -> some View {
        content
            .opacity(hasRisen ? 1 : 0)
            .offset(y: hasRisen || reduceMotion ? 0 : 8)
            .blur(radius: hasRisen || reduceMotion ? 0 : 3)
            .animation(reduceMotion ? .easeOut(duration: 0.2) : .easeOut(duration: 0.34).delay(delay), value: hasRisen)
    }
}
