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
    /// Times in a row it ran into the quota; 0 when LMAS left it out. Never past what the wire counts to
    /// (`Wire.farthest`), so it can be shown and counted on without overflowing.
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
        streak = min(event.streak ?? 0, Wire.farthest)
        at = event.atMs
    }
}

extension WorkshopState {
    /// The gates of this vendor that are not open: its own first, then its models' by name.
    public func gatesNotOpen(of vendor: String) -> [Gate] {
        gates.values.filter { $0.vendor == vendor && !$0.isOpen }
            .sorted { ($0.model == nil ? 0 : 1, $0.model ?? "") < ($1.model == nil ? 0 : 1, $1.model ?? "") }
    }

    /// The gate that holds one of a vendor's models back, as the model picker marks it: the vendor's own when that is
    /// not open (it holds every model back), otherwise the model's own when that is not. Marked only: whether a turn
    /// runs on it is LMAS's call (the user, 2026-11-27).
    public func gate(holdingBack model: String, of vendor: String) -> Gate? {
        if let own = gates[Gate.Key(vendor: vendor, model: nil)], !own.isOpen { return own }
        guard let gate = gates[Gate.Key(vendor: vendor, model: model)], !gate.isOpen else { return nil }
        return gate
    }
}
