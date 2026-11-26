// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.

/// A vendor's quota gate, or one of its models' gates, as LMAS last said it (`gate_changed`).
public struct Gate: Equatable, Sendable {
    public struct Key: Hashable, Sendable {
        public let vendor: String
        /// None for the vendor's own gate.
        public let model: String?

        public init(vendor: String, model: String?) {
            self.vendor = vendor
            self.model = model
        }
    }

    public let vendor: String
    public let model: String?
    /// `open`, `started_only`, `closed`; an unknown word is kept and shown.
    public let level: String
    public let why: Why?
    /// When LMAS looks again.
    public let until: UInt64?
    /// Times in a row it ran into the quota; 0 when LMAS left it out.
    public let streak: UInt64
    public let at: UInt64

    public var key: Key { Key(vendor: vendor, model: model) }
    public var isOpen: Bool { level == "open" }

    public init(_ event: Event.GateChanged) {
        vendor = event.vendor
        model = event.model
        level = event.level
        why = event.why
        until = event.untilMs
        streak = event.streak ?? 0
        at = event.atMs
    }
}

extension WorkshopState {
    /// The gates of this vendor that are not open: its own first, then its models' by name.
    public func gatesNotOpen(of vendor: String) -> [Gate] {
        gates.values.filter { $0.vendor == vendor && !$0.isOpen }
            .sorted { ($0.model == nil ? 0 : 1, $0.model ?? "") < ($1.model == nil ? 0 : 1, $1.model ?? "") }
    }
}
