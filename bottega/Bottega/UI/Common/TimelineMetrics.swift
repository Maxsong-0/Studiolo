// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import CoreGraphics

/// The timeline's shared geometry. One vertical rhythm governs every pair of rows —
/// reply to reply, reply to work line, work line to row — so the distance between
/// text and the steps never varies. A message's hover controls sit inside that gap
/// rather than widening it.
///
/// It sits in UI rather than beside the rows because the markdown renderer draws one of
/// the timeline's pills itself: the lead's delegation card (`CrewDelegationBlock`).
enum TimelineMetrics {
    /// The gap between any two consecutive rows in the timeline.
    static let rowSpacing: CGFloat = 16
    /// The room a message keeps for its hover line (copy, revert). Reserved inside
    /// the row so showing the controls never moves text, then subtracted from the
    /// row's own height so they fill the gap below instead of adding to it.
    static let hoverLineHeight: CGFloat = 18
    /// The icon column every transcript row starts with. Rows carry no leading inset,
    /// so the column sits on the reply text's own x and all labels line up after it.
    static let iconWidth: CGFloat = 16
    static let iconSpacing: CGFloat = 8
    /// One pill for everything that sits in the conversation as a badge or a card: the
    /// hand pills, the question badge, the merge pill, a notice, the lead's delegation
    /// card, the working line. Same corner, same insets, so they line up in the column
    /// and read as one family. Trailing is wider than leading because most end in a
    /// chevron or a time, which sits lighter than the glyph that opens them.
    static let pillLeading: CGFloat = 12
    static let pillTrailing: CGFloat = 14
    static let pillVertical: CGFloat = 8
    static let pillRadius: CGFloat = 18
    /// The room a user message leaves at the trailing edge and a reply at its own: the
    /// two insets that keep the column from reading as one wall of text.
    static let userInset: CGFloat = 64
    static let replyInset: CGFloat = 48
}
