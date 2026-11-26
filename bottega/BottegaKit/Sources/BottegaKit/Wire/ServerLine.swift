// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
public import Foundation

/// The first line of every connection.
public struct Greeting: Equatable, Sendable {
    public var version: Int
    public var connection: UInt64
    public var nonce: Data
    public var registered: Bool
}

public struct Welcome: Decodable, Equatable, Sendable {
    public var device, name: String
    /// The newest event's number; 0 when there is none.
    public var latest: UInt64
    /// The number this device's next intent should carry.
    public var nextIntent: UInt64
    public var nowMs: UInt64

    private enum Key: CodingKey { case device, name, latest, nextIntent, nowMs }

    public init(from decoder: any Decoder) throws {
        let fields = try decoder.container(keyedBy: Key.self)
        device = try fields.decode(String.self, forKey: .device)
        name = try fields.decode(String.self, forKey: .name)
        latest = try fields.decode(UInt64.self, forKey: .latest)
        nextIntent = try fields.decodeNumber(from: 1, forKey: .nextIntent)
        nowMs = try fields.decode(UInt64.self, forKey: .nowMs)
    }
}

/// Why LMAS did not do what a request asked. Switch on `code`; `text` is for people.
public struct WireError: Decodable, Equatable, Error, Sendable {
    public var code, text: String
    /// With `gap`, `conflict`, `number_too_old`: the number to use now.
    public var next: UInt64?
    /// With `ahead`.
    public var latest: UInt64?
    /// With `gone`.
    public var oldest: UInt64?

    public init(code: String, text: String, next: UInt64? = nil, latest: UInt64? = nil, oldest: UInt64? = nil) {
        self.code = code
        self.text = text
        self.next = next
        self.latest = latest
        self.oldest = oldest
    }

    private enum Key: CodingKey { case code, text, next, latest, oldest }

    /// With a number that cannot be, it is the same error without it.
    public init(from decoder: any Decoder) throws {
        let fields = try decoder.container(keyedBy: Key.self)
        code = try fields.decode(String.self, forKey: .code)
        text = try fields.decode(String.self, forKey: .text)
        next = try? fields.decodeNumber(from: 1, forKey: .next)
        latest = try? fields.decodeNumber(forKey: .latest)
        oldest = try? fields.decodeNumber(from: 1, forKey: .oldest)
    }
}

/// An answer to a request. What is inside `ok` depends on the request, so it is read when the asker says what it is.
public struct Reply: Equatable, Sendable {
    public var id: UInt64
    private var line: Data

    init(id: UInt64, line: Data) {
        self.id = id
        self.line = line
    }

    /// An answer that is not what was asked for is a line that cannot be read, like any other.
    public func read<Body: Decodable>(_ type: Body.Type = Body.self) throws(ServerLine.Malformed) -> Body {
        do {
            return try Wire.decoder.decode(Envelope<Body>.self, from: line).ok
        } catch {
            throw ServerLine.Malformed()
        }
    }

    private struct Envelope<Body: Decodable>: Decodable { var ok: Body }
}

/// A line LMAS wrote.
public enum ServerLine: Equatable, Sendable {
    case greeting(Greeting)
    case welcome(Welcome)
    /// Unregistered handshake accepted: hand in the registration.
    case registerNow
    case reply(Reply)
    /// Errors of the handshake carry no `id`.
    case error(id: UInt64?, WireError)
    case record(Record)
    /// Everything up to here has been sent.
    case caughtUp(through: UInt64)
    /// Something about a turn. Comes apart from the records, in no order with them.
    case live(Live)
    /// A line of a kind this Bottega does not know. Ignored.
    case unknown

    public struct Malformed: Error, Equatable { public init() {} }

    public static func read(_ line: Data) throws -> ServerLine {
        let fields: Fields
        do {
            fields = try Wire.decoder.decode(Fields.self, from: line)
        } catch {
            throw Malformed()
        }
        if let lmas = fields.lmas {
            guard
                lmas == "ui", let version = fields.version, let connection = fields.connection,
                let nonce = fields.nonce.flatMap({ Hex.decode($0, count: Preimage.nonceBytes) })
            else { throw Malformed() }
            return .greeting(
                Greeting(version: version, connection: connection, nonce: nonce, registered: fields.registered ?? false))
        }
        if let error = fields.error { return .error(id: fields.id, error) }
        if fields.ok != nil, let id = fields.id { return .reply(Reply(id: id, line: line)) }
        if let record = fields.record { return .record(record) }
        if let live = fields.live { return .live(live) }
        if let caughtUp = fields.caughtUp { return .caughtUp(through: caughtUp.through) }
        if let welcome = fields.welcome { return .welcome(welcome) }
        if fields.register != nil { return .registerNow }
        return .unknown
    }

    private struct Fields: Decodable {
        var lmas: String?
        var version: Int?
        var connection: UInt64?
        var nonce: String?
        var registered: Bool?
        var id: UInt64?
        var ok: Present?
        var error: WireError?
        var record: Record?
        var caughtUp: CaughtUp?
        var live: Live?
        var welcome: Welcome?
        var register: Present?
    }

    private struct CaughtUp: Decodable { var through: UInt64 }

    /// The key is there; what it holds is someone else's business.
    private struct Present: Decodable { init(from decoder: any Decoder) {} }
}

/// `overview`: what this LMAS has switched on.
public struct Overview: Decodable, Equatable, Sendable {
    public var nowMs, latest, oldest: UInt64
    public var lmas: String
    /// Intents that came under a device's number and were not that device's, or not readable, since LMAS started.
    public var droppedIntents: UInt64?
    public var cards: Cards?
    /// What the bus allows now. LMAS before it told them has none.
    public var limits: Limits?
    /// Set when lines straight to one agent can be opened.
    public var lines: Lines?

    public struct Cards: Decodable, Equatable, Sendable {
        public var repo: String
        public var vendors: [String]
        public var reviewers: [String]?
        public var tests: Bool
        public var mergeInto: String?
    }

    /// What the user says counts towards none of these.
    public struct Limits: Decodable, Equatable, Sendable {
        /// How far a letter may be passed on.
        public var maxHop: Int
        /// Letters on a line.
        public var threadBudget: Int
        /// Letters between two members, in a while.
        public var pairLimit: Int
        public var pairWindowMs: UInt64
        /// Letters one member may send, in a while.
        public var senderRate: Int
        public var senderWindowMs: UInt64
    }

    public struct Lines: Decodable, Equatable, Sendable {
        /// How many of their agents may be up at once.
        public var max: Int
    }
}

/// A page of `events`.
public struct Page: Decodable, Equatable, Sendable {
    public var records: [Record]
    public var through, latest: UInt64
}

/// `follow` accepted.
public struct Following: Decodable, Equatable, Sendable { public var latest: UInt64 }

/// `register` accepted.
public struct Registered: Decodable, Equatable, Sendable { public var device: String }

/// `seen` accepted.
public struct Seen: Decodable, Equatable, Sendable { public var through: UInt64 }

/// `intent` reached step 3. `repeated` when LMAS had already answered this very intent before.
public struct IntentReply: Decodable, Equatable, Sendable {
    /// The number of the record the echo is in.
    public var seq: UInt64
    public var event: Event
    public var repeated: Bool?

    public var echo: Event.IntentEchoed? {
        if case .intentEchoed(let echo) = event { echo } else { nil }
    }
}
