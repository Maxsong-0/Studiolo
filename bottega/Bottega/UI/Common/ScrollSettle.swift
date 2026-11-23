// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import QuartzCore

/// When a scroll counts as over: the timeline, the sidebar's list and the Settings pane all
/// hold their rows away from the pointer while the content moves under it (no hover, no
/// clicks, no hover animations), and give them back once no frame has moved for this long.
/// Wheel ticks report no phase, so the quiet itself is what ends a scroll. Shorter hands
/// the pointer back sooner after a flick; longer spares more hover work in a slow, stepped
/// scroll. One value for the three, so they settle alike.
enum ScrollSettle {
    /// The quiet that ends a scroll, in seconds, for a check against `CACurrentMediaTime()`.
    static let seconds: CFTimeInterval = 0.15
    /// The same quiet, for a sleep that is re-armed on every movement.
    static var delay: Duration { .milliseconds(Int((seconds * 1000).rounded())) }
    /// How often a watch that is armed once per scroll looks for the quiet (the timeline's).
    static let poll: Duration = .milliseconds(80)
}
