// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.

// Quota gates, and cards and reviews moved to another vendor because of one (LMAS `docs/bottega-protocol.md`,
// section 8).

extension Event {
    /// A vendor's quota gate (or one of its models' gates) changed.
    public struct GateChanged: Decodable, Equatable, Sendable {
        public var vendor: String
        /// Only for a gate that closes one model.
        public var model: String?
        /// `open`, `started_only` (no new cards; what started carries on) or `closed` (all stops). Kept as a word:
        /// an unknown one is shown.
        public var level: String
        /// Why it is not open. An open gate has none.
        public var why: Why?
        /// When LMAS looks again: the window resets, it asks again, or it lets one through to try.
        public var untilMs: UInt64?
        /// How many times in a row it ran into the quota with no turn done in between. Left out when 0.
        public var streak: UInt64?
        public var atMs: UInt64
    }

    /// A card not started yet goes to another vendor, because the gate of the one it had is not open.
    public struct CardReassigned: Decodable, Equatable, Sendable {
        public var card, vendor: String
        /// The reviewer picked again with it.
        public var reviewer: String?
        public var atMs: UInt64
    }

    /// A review not started yet goes to another vendor, because its gate closed.
    public struct ReviewReassigned: Decodable, Equatable, Sendable { public var card, vendor: String; public var atMs: UInt64 }
}
