// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.

// A card's AI asked something and nobody has answered yet (LMAS `docs/bottega-protocol.md`, section 4).

extension Event {
    /// LMAS reminded whoever the question went to. The same record has the reminder letter (a coordinator
    /// `notification`), unless the question went to the user. At 24 hours the question fails `unanswered`.
    public struct AnswerReminded: Decodable, Equatable, Sendable {
        public var card: String
        /// The number of the letter that asked.
        public var question: String
        /// 1 after an hour, 2 after twelve (LMAS's defaults). Kept as a number: a third is shown as it comes.
        public var level: UInt64
        public var atMs: UInt64
    }
}
