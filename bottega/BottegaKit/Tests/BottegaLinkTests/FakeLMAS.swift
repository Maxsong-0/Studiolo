// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import BottegaKit
import BottegaLink
import Foundation
import Synchronization

/// LMAS's side of the protocol document, in memory, enough to hold a client to it: it verifies every signature with
/// the registered keys, keeps the intent books and refuses what the document says is refused. Not LMAS: what it
/// proves is that the client follows the document, not that the two fit.
final class FakeLMAS: Connector, Sendable {
    struct Knobs {
        /// Cut the connection instead of answering the next intent: before doing it, or after.
        var dropBeforeIntent = 0
        var dropAfterIntent = 0
        /// Answer the next greeting with this error.
        var refuseNext: WireError?
        var isRunning = true
        /// LMAS's clock, as an offset to the real one.
        var clock: Int64 = 0
        var oldest: UInt64 = 1
        /// What it keeps of turns: JSON of the pages, by turn. nil: an LMAS that does not know `watch` and `turn`.
        var turns: [String: [String]]?
        /// Pushed right behind the answer to `watch`: JSON of what is inside `live`.
        var running: [(turn: String, lines: [String])] = []
    }

    private struct Books {
        var knobs = Knobs()
        var connection: UInt64 = 6
        var registration: Registration?
        var log: [String] = []  // JSON of each record
        var through: UInt64 = 0
        var next: UInt64 = 1
        var ledger: [UInt64: (digest: String, seq: UInt64, event: String)] = [:]
        var contents: [String: UInt64] = [:]
        var executed: [(kind: String, body: String)] = []
        var handedIn: [SignedIntent] = []
        var seen: [UInt64] = []
        var dropped = 0
        var followedFrom: [UInt64] = []
        var open: [FakeChannel] = []
        var refuseOutcome: [String: String] = [:]  // kind → why JSON
    }

    private let books = Mutex(Books())
    static let device = "device-1"

    /// Registered, but with keys nobody has: every intent is dropped.
    func forgetKeys() {
        let other = SoftwareKeys()
        books.withLock { books in
            books.registration = books.registration.map { old in
                Registration(
                    name: old.name, dailyKey: other.daily.hex, approvalKey: other.approval.hex,
                    dailySignature: old.dailySignature, approvalSignature: old.approvalSignature)
            }
        }
    }
    func set(_ change: (inout Knobs) -> Void) { books.withLock { change(&$0.knobs) } }
    func refuse(_ kind: String, code: String, text: String) {
        books.withLock { $0.refuseOutcome[kind] = #"{"code":"\#(code)","text":"\#(text)"}"# }
    }
    var executed: [(kind: String, body: String)] { books.withLock { $0.executed } }
    var handedIn: [SignedIntent] { books.withLock { $0.handedIn } }
    var followedFrom: [UInt64] { books.withLock { $0.followedFrom } }
    var seen: [UInt64] { books.withLock { $0.seen } }
    var through: UInt64 { books.withLock { $0.through } }
    var isRegistered: Bool { books.withLock { $0.registration != nil } }

    /// Another program using up this device's next number, which must not happen and does.
    func useUpANumber() {
        books.withLock { books in
            books.ledger[books.next] = (String(repeating: "0", count: 64), 0, "")
            books.next += 1
        }
    }

    /// A log restored to nothing, registration and all.
    func wipe() {
        books.withLock { books in
            books.registration = nil
            books.log = []
            books.through = 0
            books.next = 1
            books.ledger = [:]
            books.contents = [:]
        }
        dropAll()
    }

    /// Appends a record; `events` is JSON inside the brackets.
    func append(_ events: String, spans: UInt64 = 1) {
        let channels = books.withLock { books -> [(FakeChannel, String)] in
            let line = Self.append(events, spans, &books)
            return books.open.filter(\.isFollowing).map { ($0, line) }
        }
        for (channel, line) in channels { channel.yield(line) }
    }

    /// A record whose number does not follow on.
    func sendBroken() {
        let channels = books.withLock { $0.open }
        for channel in channels { channel.yield(#"{"record":{"seq":999,"through":999,"events":[]}}"#) }
    }

    func dropAll() {
        let channels = books.withLock { books in
            defer { books.open = [] }
            return books.open
        }
        for channel in channels { channel.close() }
    }

    private static func append(_ events: String, _ spans: UInt64, _ books: inout Books) -> String {
        let seq = books.through + 1
        books.through += spans
        let line = #"{"record":{"seq":\#(seq),"through":\#(books.through),"events":[\#(events)]}}"#
        books.log.append(line)
        return line
    }

    func connect() async throws -> any LineChannel {
        let channel = try books.withLock { books -> FakeChannel in
            guard books.knobs.isRunning else { throw POSIXError(.ECONNREFUSED) }
            books.connection += 1
            let channel = FakeChannel(lmas: self, connection: books.connection)
            books.open.append(channel)
            return channel
        }
        let refusal = books.withLock { books in
            defer { books.knobs.refuseNext = nil }
            return books.knobs.refuseNext
        }
        if let refusal {
            channel.yield(#"{"error":{"code":"\#(refusal.code)","text":"\#(refusal.text)"}}"#)
            channel.close()
        } else {
            channel.yield(
                #"{"lmas":"ui","version":1,"connection":\#(channel.connection),"nonce":"\#(Hex.encode(channel.nonce))","registered":\#(isRegistered)}"#)
        }
        return channel
    }

    // MARK: Requests

    fileprivate func take(_ line: Data, on channel: FakeChannel) {
        guard let json = try? JSONSerialization.jsonObject(with: line) as? [String: Any] else {
            return channel.fail(nil, "malformed")
        }
        if let hello = json["hello"] as? String { return self.hello(hello, json, channel) }
        let id = json["id"] as? UInt64 ?? 0
        switch json["do"] as? String {
        case "register": register(json, id, channel)
        case "overview":
            let (latest, oldest, dropped) = books.withLock { ($0.through, $0.knobs.oldest, $0.dropped) }
            channel.ok(id, #"{"now_ms":1,"latest":\#(latest),"oldest":\#(oldest),"lmas":"fake","dropped_intents":\#(dropped)}"#)
        case "follow": follow(json["from"] as? UInt64 ?? 0, id, channel)
        case "seen":
            let through = json["through"] as? UInt64 ?? 0
            let latest = books.withLock { books in
                if through <= books.through { books.seen.append(through) }
                return books.through
            }
            guard through <= latest else { return channel.fail(id, "ahead", #","latest":\#(latest)"#) }
            channel.ok(id, #"{"through":\#(through)}"#)
        case "intent": intent(json["intent"] as? [String: Any] ?? [:], id, channel)
        case "watch": watch(id, channel)
        case "turn": turn(json["turn"] as? String ?? "", json["from"] as? Int ?? 0, id, channel)
        case "ping": channel.ok(id, #"{"now_ms":1}"#)
        default: channel.fail(id, "unknown_request")
        }
    }

    private func watch(_ id: UInt64, _ channel: FakeChannel) {
        let knobs = books.withLock { $0.knobs }
        guard knobs.turns != nil else { return channel.fail(id, "unknown_request") }
        guard !channel.isWatching else { return channel.fail(id, "already_watching") }
        channel.isWatching = true
        channel.ok(id, #"{"turns":[\#(knobs.running.map { #""\#($0.turn)""# }.joined(separator: ","))]}"#)
        for running in knobs.running { for line in running.lines { channel.yield(#"{"live":{"turn":"\#(running.turn)",\#(line)}}"#) } }
    }

    private func turn(_ turn: String, _ from: Int, _ id: UInt64, _ channel: FakeChannel) {
        guard let turns = books.withLock({ $0.knobs.turns }) else { return channel.fail(id, "unknown_request") }
        guard let pages = turns[turn] else { return channel.fail(id, "no_such_turn") }
        guard !pages.isEmpty else { return channel.fail(id, "no_details") }
        // Every page in the tests has two items.
        channel.ok(id, pages[min((from - 1) / 2, pages.count - 1)])
    }

    /// Pushes to everyone watching; `line` is JSON inside `live`, behind the turn.
    func push(_ turn: String, _ line: String) {
        for channel in books.withLock({ $0.open }) where channel.isWatching {
            channel.yield(#"{"live":{"turn":"\#(turn)",\#(line)}}"#)
        }
    }

    private func hello(_ hello: String, _ json: [String: Any], _ channel: FakeChannel) {
        guard hello == "bottega" else { return channel.yield(#"{"register":{"now_ms":1}}"#) }
        let (registration, latest, next) = books.withLock { ($0.registration, $0.through, $0.next) }
        guard
            let registration, json["device"] as? String == Self.device, let key = PublicKey(hex: registration.dailyKey),
            key.verifies(
                .handshake(nonce: channel.nonce, connection: channel.connection),
                signature: json["signature"] as? String ?? "")
        else {
            channel.fail(nil, "handshake")
            return channel.close()
        }
        channel.yield(
            #"{"welcome":{"device":"\#(Self.device)","name":"\#(registration.name)","latest":\#(latest),"next_intent":\#(next),"now_ms":1}}"#)
    }

    private func register(_ json: [String: Any], _ id: UInt64, _ channel: FakeChannel) {
        guard
            let data = try? JSONSerialization.data(withJSONObject: json["registration"] ?? [:]),
            let registration = try? JSONDecoder().decode(Registration.self, from: data),
            registration.isSigned(nonce: channel.nonce, connection: channel.connection)
        else { return channel.fail(id, "bad_registration") }
        let isNew = books.withLock { books -> Bool? in
            if let old = books.registration {
                let same = (old.name, old.dailyKey, old.approvalKey) == (registration.name, registration.dailyKey, registration.approvalKey)
                return same ? false : nil
            }
            books.registration = registration
            return true
        }
        guard let isNew else { return channel.fail(id, "already_registered") }
        if isNew {
            append(
                #"{"type":"device_registered","device":"\#(Self.device)","kind":"mac","name":"\#(registration.name)","daily_key":"\#(registration.dailyKey)","approval_key":"\#(registration.approvalKey)","at_ms":1}"#)
        }
        channel.ok(id, #"{"device":"\#(Self.device)"}"#)
    }

    private func follow(_ from: UInt64, _ id: UInt64, _ channel: FakeChannel) {
        let (lines, through, oldest) = books.withLock { books in
            books.followedFrom.append(from)
            return (books.log, books.through, books.knobs.oldest)
        }
        if from < oldest { return channel.fail(id, "gone", #","oldest":\#(oldest)"#) }
        if from > through + 1 { return channel.fail(id, "ahead", #","latest":\#(through)"#) }
        channel.ok(id, #"{"latest":\#(through)}"#)
        for line in lines {
            guard case .record(let record)? = try? ServerLine.read(Data(line.utf8)), record.through >= from else { continue }
            channel.yield(line)
        }
        channel.isFollowing = true
        channel.yield(#"{"caught_up":{"through":\#(through)}}"#)
    }

    private func intent(_ json: [String: Any], _ id: UInt64, _ channel: FakeChannel) {
        guard
            let data = try? JSONSerialization.data(withJSONObject: json),
            let intent = try? JSONDecoder().decode(SignedIntent.self, from: data)
        else { return channel.fail(id, "dropped") }
        enum Reply { case drop, error(String, String), echo(seq: UInt64, event: String, repeated: Bool, line: String?, then: Bool) }
        let reply = books.withLock { books -> Reply in
            books.handedIn.append(intent)
            if books.knobs.dropBeforeIntent > 0 {
                books.knobs.dropBeforeIntent -= 1
                return .drop
            }
            guard
                let registration = books.registration, intent.device == Self.device,
                let daily = PublicKey(hex: registration.dailyKey), let approval = PublicKey(hex: registration.approvalKey),
                intent.isSigned(daily: daily, approval: approval)
            else {
                books.dropped += 1
                return .error("dropped", "")
            }
            if intent.number < books.next {
                guard let entry = books.ledger[intent.number] else { return .error("number_too_old", #","next":\#(books.next)"#) }
                guard entry.digest == intent.digest else { return .error("conflict", #","next":\#(books.next)"#) }
                return .echo(seq: entry.seq, event: entry.event, repeated: true, line: nil, then: false)
            }
            if intent.number > books.next { return .error("gap", #","next":\#(books.next)"#) }
            let content = (try? JSONSerialization.jsonObject(with: Data(intent.body.utf8)) as? [String: Any])?["content_id"] as? String ?? ""
            let now = UInt64(Int64(Date.now.timeIntervalSince1970 * 1000) + books.knobs.clock)
            let outcome: String
            if intent.createdAt > now + 5 * 60_000 {
                outcome = #"{"result":"refused","why":{"code":"in_future","text":"在未来"}}"#
            } else if let number = books.contents[content] {
                outcome = #"{"result":"already","number":\#(number)}"#
            } else if let why = books.refuseOutcome[intent.kind] {
                outcome = #"{"result":"refused","why":\#(why)}"#
            } else {
                books.contents[content] = intent.number
                books.executed.append((intent.kind, intent.body))
                outcome = intent.kind == "say" ? #"{"result":"done","message":"msg-\#(intent.number)"}"# : #"{"result":"done"}"#
            }
            let event =
                #"{"type":"intent_echoed","device":"\#(Self.device)","number":\#(intent.number),"digest":"\#(intent.digest)","content_id":"\#(content)","outcome":\#(outcome),"at_ms":\#(now)}"#
            let seq = books.through + 1
            let line = Self.append(event, 1, &books)
            books.ledger[intent.number] = (intent.digest, seq, event)
            books.next += 1
            let drops = books.knobs.dropAfterIntent > 0
            if drops { books.knobs.dropAfterIntent -= 1 }
            return .echo(seq: seq, event: event, repeated: false, line: line, then: drops)
        }
        switch reply {
        case .drop: dropAll()
        case .error(let code, let extra): channel.fail(id, code, extra)
        case .echo(let seq, let event, let repeated, let line, let drops):
            if drops { return dropAll() }
            channel.ok(id, #"{"seq":\#(seq),"event":\#(event)\#(repeated ? #","repeated":true"# : "")}"#)
            if let line {
                for open in books.withLock({ $0.open }) where open.isFollowing { open.yield(line) }
            }
        }
    }
}

final class FakeChannel: LineChannel, Sendable {
    let connection: UInt64
    let nonce: Data
    private let lmas: FakeLMAS
    private let stream: AsyncThrowingStream<Data, any Error>
    private let continuation: AsyncThrowingStream<Data, any Error>.Continuation
    private let following = Mutex(false)
    private let closed = Mutex(false)
    private let watching = Mutex(false)

    var isWatching: Bool {
        get { watching.withLock { $0 } }
        set { watching.withLock { $0 = newValue } }
    }

    var isFollowing: Bool {
        get { following.withLock { $0 } }
        set { following.withLock { $0 = newValue } }
    }

    init(lmas: FakeLMAS, connection: UInt64) {
        self.lmas = lmas
        self.connection = connection
        nonce = Data((0..<32).map { _ in UInt8.random(in: 0...255) })
        (stream, continuation) = AsyncThrowingStream.makeStream()
    }

    func lines() -> AsyncThrowingStream<Data, any Error> { stream }

    func send(_ line: Data) async throws {
        if closed.withLock({ $0 }) { throw POSIXError(.EPIPE) }
        lmas.take(line.dropLast(), on: self)
    }

    func close() {
        closed.withLock { $0 = true }
        isFollowing = false
        isWatching = false
        continuation.finish()
    }

    func yield(_ line: String) { continuation.yield(Data(line.utf8)) }
    func ok(_ id: UInt64, _ body: String) { yield(#"{"id":\#(id),"ok":\#(body)}"#) }
    func fail(_ id: UInt64?, _ code: String, _ extra: String = "") {
        yield(#"{\#(id.map { #""id":\#($0),"# } ?? "")"error":{"code":"\#(code)","text":"\#(code)"\#(extra)}}"#)
    }
}

final class FakeChest: KeyChest, Sendable {
    private let made: Mutex<(any DeviceKeys)?>
    private let makes: any DeviceKeys
    private let makings = Mutex(0)
    var timesMade: Int { makings.withLock { $0 } }
    init(made: (any DeviceKeys)? = nil, makes: any DeviceKeys = SoftwareKeys()) {
        self.made = Mutex(made)
        self.makes = makes
    }
    func keys() throws -> (any DeviceKeys)? { made.withLock { $0 } }
    func makeKeys() throws -> any DeviceKeys {
        makings.withLock { $0 += 1 }
        made.withLock { $0 = makes }
        return makes
    }
}

/// Keys whose approval signature waits for the test to say what the finger did.
final class FingerKeys: DeviceKeys, Sendable {
    private let keys = SoftwareKeys()
    private let allows = Mutex(true)
    private let prompts = Mutex<[String]>([])
    var daily: PublicKey { keys.daily }
    var approval: PublicKey { keys.approval }
    var prompted: [String] { prompts.withLock { $0 } }
    func allow(_ yes: Bool) { allows.withLock { $0 = yes } }
    func sign(_ preimage: Preimage, with use: KeyUse, reason: String) async throws -> String {
        if use == .approval {
            prompts.withLock { $0.append(reason) }
            guard allows.withLock({ $0 }) else { throw SigningRefused("Touch ID was cancelled.") }
        }
        return try keys.sign(preimage, with: use, reason: reason)
    }
}

final class MemoryStore: KeepsakeStore, Sendable {
    private let kept = Mutex(Keepsake())
    private let broken = Mutex(false)
    var keepsake: Keepsake { kept.withLock { $0 } }
    func breakDisk(_ yes: Bool) { broken.withLock { $0 = yes } }
    func load() throws -> Keepsake { keepsake }
    func save(_ keepsake: Keepsake) throws {
        if broken.withLock({ $0 }) { throw POSIXError(.ENOSPC) }
        kept.withLock { $0 = keepsake }
    }
}
