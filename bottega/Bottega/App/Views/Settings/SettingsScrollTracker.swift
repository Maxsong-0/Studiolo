// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import SwiftUI

/// Whether the reader is scrolling a settings pane right now, reported to
/// `ScrollActivity` under one fixed pane id so every effect that has no business
/// running under a moving finger rests while the pane scrolls, and declared to the
/// rows so they stop hovering (and animating) as they slide under the pointer.
@MainActor
@Observable
final class SettingsScrollTracker: ScrollActivityReporter {
    private static let pane = UUID()
    private(set) var isScrolling = false
    /// Whether the pane's rows are held away from the pointer: true through the drag and
    /// the wheel's movement, false again once the flick coasts or the scroll settles, so a
    /// click that stops a flick always reaches its target (the chat timeline's rule).
    private(set) var isFrozen = false
    @ObservationIgnored private var isCoasting = false
    @ObservationIgnored private var settle: Task<Void, Never>?

    var isScrollingNow: Bool { isScrolling }

    /// A frame of movement: scrolling from now, and until a beat (`ScrollSettle`) has passed
    /// with none.
    func note() {
        if !isScrolling {
            isScrolling = true
            ScrollActivity.shared.setScrolling(true, timeline: Self.pane, reporter: self)
        }
        if !isCoasting, !isFrozen { isFrozen = true }
        settle?.cancel()
        settle = Task { @MainActor [weak self] in
            try? await Task.sleep(for: ScrollSettle.delay)
            guard let self, !Task.isCancelled else { return }
            isScrolling = false
            isCoasting = false
            if isFrozen { isFrozen = false }
            ScrollActivity.shared.setScrolling(false, timeline: Self.pane)
        }
    }

    /// The drag is over and the flick is coasting: the rows take the pointer again, while
    /// everything that rests on a scroll still rests.
    func coast() {
        isCoasting = true
        if isFrozen { isFrozen = false }
    }

    /// The pane is going away: nothing of it is reported any more.
    func end() {
        settle?.cancel()
        settle = nil
        if isFrozen { isFrozen = false }
        isCoasting = false
        guard isScrolling else { return }
        isScrolling = false
        ScrollActivity.shared.setScrolling(false, timeline: Self.pane)
    }
}
