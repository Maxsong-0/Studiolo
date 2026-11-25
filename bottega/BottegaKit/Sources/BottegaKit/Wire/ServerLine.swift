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
}

/// An answer to a request. What is inside `ok` depends on the request, so it is read when the asker says what it is.
public struct Reply: Equatable, Sendable {
    public var id: UInt64
    private var line: Data

    init(id: UInt64, line: Data) {
        self.id = id
        self.line = line
    }

    public func read<Body: Decodable>(_ type: Body.Type = Body.self) throws -> Body {
        try Wire.decoder.decode(Envelope<Body>.self, from: line).ok
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
    public var cards: Cards?

    public struct Cards: Decodable, Equatable, Sendable {
        public var repo: String
        public var vendors: [String]
        public var reviewers: [String]?
        public var tests: Bool
        public var mergeInto: String?
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
