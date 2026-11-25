// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.

/// Whether something about to be signed is being asked for at the keyboard or mouse right now.
///
/// The event that is "current" is not enough: it stays around after it has been handled, so a press through the
/// accessibility interface, which comes with no event, would pass on the strength of the last real click. So the
/// event has to be on its way through the app at this moment, be fresh, be in the front window, and be used once.
public struct InputGate: Sendable {
    /// What is known about the event being handled.
    public struct Input: Equatable, Sendable {
        /// A key going down or a mouse button.
        public var isPress: Bool
        /// The process that posted it: 0 for the keyboard and mouse themselves. nil when AppKit made it up.
        public var sourceProcess: Int64?
        /// Seconds since startup, as events are stamped.
        public var timestamp: Double
        /// Whether it went to the window that has the keyboard.
        public var isInKeyWindow: Bool

        public init(isPress: Bool, sourceProcess: Int64?, timestamp: Double, isInKeyWindow: Bool) {
            self.isPress = isPress
            self.sourceProcess = sourceProcess
            self.timestamp = timestamp
            self.isInKeyWindow = isInKeyWindow
        }
    }

    public enum Refusal: String, Error, Sendable {
        case notInFront
        case noEvent
        case notAPress
        case synthetic
        case stale
        case otherWindow
        case usedAlready
    }

    /// How long after it happened a press still counts.
    public static let patience = 1.0

    private var lastUsed = -Double.infinity

    public init() {}

    /// `input` is nil when no event is being handled. A press that gets through is used up.
    public mutating func admit(_ input: Input?, isInFront: Bool, now: Double) throws(Refusal) {
        guard isInFront else { throw .notInFront }
        guard let input else { throw .noEvent }
        guard input.isPress else { throw .notAPress }
        guard input.sourceProcess == 0 else { throw .synthetic }
        guard now - input.timestamp <= Self.patience, input.timestamp <= now + Self.patience else { throw .stale }
        guard input.isInKeyWindow else { throw .otherWindow }
        guard input.timestamp > lastUsed else { throw .usedAlready }
        lastUsed = input.timestamp
    }
}
