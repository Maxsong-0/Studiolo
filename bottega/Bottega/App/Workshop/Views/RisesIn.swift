// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import SwiftUI

/// Shown at the start hidden, then rising into place after `delay` (`CardsMotion`); with less motion only fading in.
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
