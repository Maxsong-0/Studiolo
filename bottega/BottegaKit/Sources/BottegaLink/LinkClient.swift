// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
public import BottegaKit
import Foundation

/// The connection to LMAS, kept up: greets, proves which device this is, follows the events from where it left off,
/// hands in what the user asks one intent at a time. Everything it learns comes out of `updates`, in order.
public actor LinkClient {
    public enum Status: Equatable, Sendable {
        case connecting
        /// This Mac has no device number with that LMAS. Nothing happens until `register`.
        case needsRegistration
        case registering
        /// Connected; events up to `latest` are still coming.
        case catchingUp(latest: UInt64)
        case live
        /// Not connected; trying again by itself.
        case retrying(Why)
        /// Not connected, and trying again would change nothing. `reconnect` tries anyway.
        case stopped(Why)
    }

    public enum Update: Sendable {
        case status(Status)
        /// Forget every event so far; history starts again at this number.
        case reset(start: UInt64)
        case records([Record])
        /// What goes on inside turns is being pushed from here on, and these are running; nil: it is not, or no longer.
        /// Of a turn that is not among them, nothing more comes.
        case watching([String]?)
        case live([Live])
        case overview(Overview)
        case welcome(Welcome)
        /// What this Mac was registered as, or may have been. For registering again under the same name.
        case registeredName(String?)
        case outbox([Outbox.Item])
        case settled(Outbox.Settled)
        /// Something went wrong that changes nothing else.
        case notice(Why)
        /// The registration did not go through.
        case registrationFailed(Why)
    }

    public nonisolated let updates: AsyncStream<Update>
    private let emit: AsyncStream<Update>.Continuation

    private let connector: any Connector
    private let chest: any KeyChest
    private let store: any KeepsakeStore
    private let pace: Pace

    /// Waits, for tests to shorten.
    public struct Pace: Sendable {
        public var retry: [Duration]
        /// How long LMAS may take over one line of the handshake.
        public var handshake: Duration
        /// Records that arrive within this are shown together.
        public var batch: Duration

        public init(
            retry: [Duration] = [.milliseconds(300), .seconds(1), .seconds(2), .seconds(5), .seconds(10)],
            handshake: Duration = .seconds(10), batch: Duration = .milliseconds(33)
        ) {
            self.retry = retry
            self.handshake = handshake
            self.batch = batch
        }
    }

    private var keepsake = Keepsake()
    private var isLoaded = false
    private var keys: (any DeviceKeys)?
    /// Asked for and not done yet.
    private var registrationName: String?

    private var channel: (any LineChannel)?
    private var isReady = false
    private var nextRequest: UInt64 = 1
    private var replies: [UInt64: CheckedContinuation<Reply, any Error>] = [:]

    private var start: UInt64 = 1
    private var through: UInt64 = 0
    private var isFresh = true
    private var batch: [Record] = []
    private var lives: [Live] = []
    /// The `watch` that has been sent and not answered.
    private var watchRequest: UInt64?
    private var isWatching = false
    private var flushing: Task<Void, Never>?
    private var catchUps: [(through: UInt64, resume: CheckedContinuation<Void, any Error>)] = []
    /// What became of this device's intents, for telling a conflict from a repeat.
    private var echoes: [String: Outcome] = [:]

    private var nextNumber: UInt64 = 1
    private var clockOffset: Int64 = 0
    private var isPumping = false
    /// What the Touch ID prompt says for an approval, by content id. Only for this run: see `dropUnsignedDecisions`.
    private var reasons: [String: String] = [:]
    /// Tries in a row that never got as far as following.
    private var failures = 0
    /// Why following did not start.
    private var openingFailure: (any Error)?

    private var wakeUps: [CheckedContinuation<Void, Never>] = []
    private var isWoken = false

    public init(connector: any Connector, chest: any KeyChest, store: any KeepsakeStore, pace: Pace = Pace()) {
        self.connector = connector
        self.chest = chest
        self.store = store
        self.pace = pace
        (updates, emit) = AsyncStream.makeStream()
    }

    // MARK: What the app asks

    /// Runs until cancelled.
    public func run() async {
        await withTaskCancellationHandler {
            while !Task.isCancelled {
                do {
                    try load()
                    try await connectOnce()
                    // A registration went through: connect again as that device.
                } catch let stop as Stop {
                    status(stop.status)
                    await sleepUntilWoken()
                    failures = 0
                } catch {
                    status(.retrying(Self.why(error)))
                    await sleepUntilWoken(atMost: pace.retry[min(failures, pace.retry.count - 1)])
                    failures += 1
                }
                hangUp()
            }
            emit.finish()
        } onCancel: {
            Task { await self.cancelled() }
        }
    }

    /// Registers this Mac under `name`. Asks for Touch ID.
    public func register(name: String) {
        guard keepsake.device == nil, let name = Registration.usableName(name) else {
            return send(.registrationFailed(.init(code: "bad_name", text: "Give this Mac a name of one line.")))
        }
        registrationName = name
        wake()
    }

    /// Forgets which device this is to that LMAS, so it can be registered again. The keys stay: to an LMAS that still
    /// knows them they are the same device, and it takes no other until its slice 8.
    public func forgetDevice() throws {
        keepsake.device = nil
        try store.save(keepsake)
        reset(start: 1)
        channel?.close()
        wake()
    }

    public func reconnect() {
        channel?.close()
        wake()
    }

    /// Drops every event and follows again from the start.
    public func reload() {
        reset(start: 1)
        reconnect()
    }

    /// Takes over what the user asked for. Returns its content id, to recognise it by when it settles.
    ///
    /// An approval only while connected: `touchIDReason` says in the prompt what is being approved.
    @discardableResult
    public func submit(_ draft: IntentDraft, touchIDReason: String = "") throws -> String {
        try draft.check()
        try load()
        if draft.key == .approval, !isReady { throw ConnectionLost() }
        let id = ContentID.random()
        if !touchIDReason.isEmpty { reasons[id] = touchIDReason }
        var changed = keepsake
        changed.outbox.add(draft, id: id, at: Self.now())
        try store.save(changed)
        keepsake = changed
        outboxChanged()
        pump()
        return id
    }

    /// Takes back something that was never signed.
    public func withdraw(_ id: String) {
        guard let item = keepsake.outbox.withdraw(id) else { return }
        saveOrSay()
        outboxChanged()
        send(.settled(.init(item: item, result: .notSent(.init(code: "withdrawn", text: "Taken back before it was sent.")))))
    }

    /// Everything up to here has been on the user's screen. Whether LMAS took note.
    public func seen(through: UInt64) async -> Bool {
        do {
            _ = try await request(.seen(through: through))
            return true
        } catch {
            return false
        }
    }

    /// What LMAS says about itself, anew.
    public func refreshOverview() async {
        do {
            let overview: Overview = try await request(.overview).read()
            send(.overview(overview))
        } catch {
            // Not connected; it comes with the next connection.
        }
    }

    /// Everything LMAS has about a turn, however many pages it comes in. Throws LMAS's `no_such_turn` and `no_details`.
    public func turn(_ turn: String) async throws -> [TurnPage] {
        var pages: [TurnPage] = []
        var next = 1
        while true {
            let page: TurnPage = try await request(.turn(turn, from: next)).read()
            pages.append(page)
            if page.through >= page.last { return pages }
            guard page.through >= next else { throw ServerLine.Malformed() }
            next = page.through + 1
        }
    }

    // MARK: One connection

    private typealias Lines = AsyncThrowingStream<Data, any Error>.AsyncIterator
    private struct Stop: Error { var status: Status }
    public struct ConnectionLost: Error, Equatable {}

    private func load() throws {
        guard !isLoaded else { return }
        do {
            keepsake = try store.load()
        } catch {
            throw Stop(status: .stopped(.init(code: "keepsake", text: "Cannot read this Mac's record of LMAS: \(error.localizedDescription)")))
        }
        isLoaded = true
        send(.registeredName(keepsake.name))
        let dropped = keepsake.outbox.dropUnsignedDecisions()
        if !dropped.isEmpty { saveOrSay() }
        for settled in dropped { send(.settled(settled)) }
        outboxChanged()
    }

    private func connectOnce() async throws {
        if let device = keepsake.device {
            status(.connecting)
            let (channel, greeting, lines) = try await greet()
            var iterator = lines
            try await handshake(as: device, greeting, &iterator, channel)
            try await follow(&iterator)
        } else if let name = registrationName {
            status(.registering)
            registrationName = nil
            do {
                try await register(name)
            } catch let error as CancellationError {
                throw error
            } catch {
                send(.registrationFailed(Self.why(error)))
                throw Stop(status: .needsRegistration)
            }
        } else {
            throw Stop(status: .needsRegistration)
        }
    }

    private func greet() async throws -> (any LineChannel, Greeting, Lines) {
        let channel = try await connector.connect()
        self.channel = channel
        var lines = channel.lines().makeAsyncIterator()
        guard case .greeting(let greeting) = try await handshakeLine(&lines, channel), greeting.version == Wire.version else {
            throw Stop(status: .stopped(.init(code: "version", text: "That is not an LMAS this Bottega can talk to.")))
        }
        return (channel, greeting, lines)
    }

    private func handshake(
        as device: String, _ greeting: Greeting, _ lines: inout Lines, _ channel: any LineChannel
    ) async throws {
        let keys = try deviceKeys()
        let preimage = Preimage.handshake(nonce: greeting.nonce, connection: greeting.connection)
        let signature = try await keys.sign(preimage, with: .daily, reason: "")
        try await channel.send(try ClientLine.hello(device: device, signature: signature))
        guard case .welcome(let welcome) = try await handshakeLine(&lines, channel), welcome.device == device else {
            throw Stop(status: .stopped(.init(code: "handshake", text: "LMAS did not welcome this Mac.")))
        }
        // A log that ends before what is already shown is another log.
        if welcome.latest < through { reset(start: 1) }
        nextNumber = welcome.nextIntent
        send(.welcome(welcome))
        status(.catchingUp(latest: welcome.latest))
    }

    private func register(_ name: String) async throws {
        let keys = try self.keys ?? chest.keys() ?? chest.makeKeys()
        self.keys = keys
        let (channel, greeting, iterator) = try await greet()
        var lines = iterator
        try await channel.send(try ClientLine.helloUnregistered())
        guard case .registerNow = try await handshakeLine(&lines, channel) else { throw ConnectionLost() }
        let preimage = Preimage.register(
            kind: Registration.mac, name: name, dailyKey: keys.daily.bytes, approvalKey: keys.approval.bytes,
            nonce: greeting.nonce, connection: greeting.connection)
        let registration = Registration(
            name: name, dailyKey: keys.daily.hex, approvalKey: keys.approval.hex,
            dailySignature: try await keys.sign(preimage, with: .daily, reason: ""),
            approvalSignature: try await keys.sign(
                preimage, with: .approval, reason: "register “\(name)” with LMAS as a device that can approve"))
        // Before it is sent: the receipt may be lost. Told no, the name before it still stands.
        let nameBefore = keepsake.name
        try keep(name: name)
        try await channel.send(try ClientLine.request(id: newRequest(), .register(registration)))
        let registered: Registered
        do {
            guard case .reply(let reply) = try await handshakeLine(&lines, channel) else { throw ConnectionLost() }
            registered = try reply.read()
        } catch let refusal as WireError {
            try keep(name: nameBefore)
            throw refusal
        }
        // The registration is in LMAS's log, so having to make one means the log is a new one.
        reset(start: 1)
        keepsake.device = registered.device
        try store.save(keepsake)
    }

    private func keep(name: String?) throws {
        keepsake.name = name
        try store.save(keepsake)
        send(.registeredName(name))
    }

    /// One line of the handshake. An error in its place decides whether trying again makes sense.
    private func handshakeLine(
        _ lines: inout Lines, _ channel: any LineChannel
    ) async throws -> ServerLine {
        let patience = pace.handshake
        let watchdog = Task {
            try await Task.sleep(for: patience)
            channel.close()
        }
        defer { watchdog.cancel() }
        guard let line = try await lines.next(isolation: self) else { throw ConnectionLost() }
        let read = try ServerLine.read(line)
        guard case .error(_, let error) = read else { return read }
        switch error.code {
        case "peer_refused", "version", "handshake", "malformed", "too_long":
            throw Stop(status: .stopped(.init(code: error.code, text: error.text)))
        default: throw error
        }
    }

    private func follow(_ lines: inout Lines) async throws {
        isReady = true
        openingFailure = nil
        let opening = Task { try await open() }
        defer { opening.cancel() }
        while let line = try await lines.next(isolation: self) {
            switch try ServerLine.read(line) {
            case .reply(let reply):
                if reply.id == watchRequest { try startWatching(reply) }
                replies.removeValue(forKey: reply.id)?.resume(returning: reply)
            case .error(let id?, let error): replies.removeValue(forKey: id)?.resume(throwing: error)
            case .error(nil, let error): throw error
            case .record(let record): try take(record)
            case .caughtUp(let through):
                guard through == self.through else { throw broken(expected: self.through, got: through) }
                flush()
                failures = 0
                status(.live)
                pump()
            case .live(let live):
                lives.append(live)
                flushSoon()
            case .greeting, .welcome, .registerNow, .unknown: break
            }
        }
        throw openingFailure ?? ConnectionLost()
    }

    /// Follows first: a client that asks for anything else first would still get every record, only later.
    private func open() async throws {
        do {
            do {
                try await followOn()
            } catch let error as WireError where error.code == "gone" {
                reset(start: error.oldest ?? 1)
                try await followOn()
            } catch let error as WireError where error.code == "ahead" {
                reset(start: 1)
                try await followOn()
            }
            let overview: Overview = try await request(.overview).read()
            send(.overview(overview))
            try await watch()
        } catch is CancellationError {
        } catch {
            // Nothing can be shown without following; start over.
            openingFailure = error
            channel?.close()
        }
    }

    private func followOn() async throws {
        let _: Following = try await request(.follow(from: through + 1)).read()
    }

    /// What goes on inside turns is an extra: an LMAS that does not tell is followed all the same.
    private func watch() async throws {
        do {
            _ = try await request(.watch)
        } catch is WireError {
            watchRequest = nil
        }
    }

    /// Where the answer is read, not where it was waited for: what is pushed comes right behind it.
    private func startWatching(_ reply: Reply) throws {
        watchRequest = nil
        let watching: Watching = try reply.read()
        isWatching = true
        send(.watching(watching.turns))
    }

    private func take(_ record: Record) throws {
        let follows = isFresh ? record.seq <= start && record.through >= start : record.seq == through + 1
        guard follows, record.through >= record.seq else { throw broken(expected: through + 1, got: record.seq) }
        isFresh = false
        through = record.through
        for event in record.events {
            if case .intentEchoed(let echo) = event, echo.device == keepsake.device { echoes[echo.digest] = echo.outcome }
        }
        batch.append(record)
        flushSoon()
        catchUps.removeAll { waiting in
            guard waiting.through <= through else { return false }
            waiting.resume.resume()
            return true
        }
    }

    private func flushSoon() {
        guard flushing == nil else { return }
        let wait = pace.batch
        flushing = Task {
            try? await Task.sleep(for: wait)
            flush()
        }
    }

    private func flush() {
        flushing = nil
        if !batch.isEmpty { send(.records(batch)) }
        batch = []
        if !lives.isEmpty { send(.live(lives)) }
        lives = []
    }

    private func broken(expected: UInt64, got: UInt64) -> Stop {
        flush()
        return Stop(status: .stopped(.init(
            code: "sequence", text: "The events do not follow on: after \(through) came \(got). Nothing past that is shown.")))
    }

    private func reset(start: UInt64) {
        flushing?.cancel()
        flushing = nil
        batch = []
        lives = []
        self.start = max(1, start)
        through = self.start - 1
        isFresh = true
        echoes = [:]
        send(.reset(start: self.start))
    }

    private func hangUp() {
        isReady = false
        channel?.close()
        channel = nil
        flush()
        watchRequest = nil
        if isWatching { send(.watching(nil)) }
        isWatching = false
        let lost = replies
        replies = [:]
        for continuation in lost.values { continuation.resume(throwing: ConnectionLost()) }
        let waiting = catchUps
        catchUps = []
        for waiting in waiting { waiting.resume.resume(throwing: ConnectionLost()) }
        let dropped = keepsake.outbox.dropUnsignedDecisions()
        for settled in dropped { finish(settled) }
    }

    private func newRequest() -> UInt64 {
        defer { nextRequest += 1 }
        return nextRequest
    }

    private func request(_ request: Request) async throws -> Reply {
        guard isReady, let channel else { throw ConnectionLost() }
        let id = newRequest()
        let line = try ClientLine.request(id: id, request)
        if request == .watch { watchRequest = id }
        return try await withCheckedThrowingContinuation { continuation in
            replies[id] = continuation
            Task {
                do {
                    try await channel.send(line)
                } catch {
                    replies.removeValue(forKey: id)?.resume(throwing: ConnectionLost())
                }
            }
        }
    }

    // MARK: Handing in intents

    /// Works through the outbox while connected. One loop at a time; it ends when there is nothing left or the
    /// connection is gone, and what was in the air stays at the head, signed.
    private func pump() {
        guard !isPumping, isReady, keepsake.outbox.head != nil else { return }
        isPumping = true
        Task {
            defer { isPumping = false }
            while isReady, let item = keepsake.outbox.head {
                do {
                    try await handIn(item)
                } catch is ConnectionLost {
                    // `isReady` again means there is a new connection by now: hand the same bytes in there.
                } catch {
                    channel?.close()
                    break
                }
            }
        }
    }

    private func handIn(_ item: Outbox.Item) async throws {
        guard let device = keepsake.device else { throw ConnectionLost() }
        let intent: SignedIntent
        if let signed = item.signed, signed.device == device {
            intent = signed
        } else {
            do {
                intent = try await sign(item, as: device)
            } catch {
                let text = (error as? SigningRefused)?.text ?? error.localizedDescription
                finish(keepsake.outbox.notSigned(item.id, .init(code: "not_signed", text: text)))
                return
            }
            // Signing may have waited for a finger: the item may be gone, or the connection may be another one
            // where that number is taken.
            guard keepsake.outbox.head?.id == item.id else { return }
            keepsake.outbox.sign(intent)
            do {
                try store.save(keepsake)
            } catch {
                // Sent without being on disk, a crash could make it a second intent.
 keepsake.outbox.unsign()
                finish(keepsake.outbox.notSigned(item.id, .init(code: "disk", text: "Could not be kept on disk, so it was not sent: \(error.localizedDescription)")))
                return
            }
            outboxChanged()
        }
        let step: Outbox.Step?
        do {
            let reply: IntentReply = try await request(.intent(intent)).read()
            guard let echo = reply.echo, echo.digest == intent.digest else { throw ServerLine.Malformed() }
            nextNumber = max(nextNumber, intent.number + 1)
            step = keepsake.outbox.take(echo.outcome, at: echo.atMs, now: Self.now())
        } catch let error as WireError {
            if let next = error.next { nextNumber = next }
            step = keepsake.outbox.take(error)
            if error.code == "dropped" { Task { await refreshOverview() } }
        }
        try await act(on: step, digest: intent.digest)
    }

    private func act(on step: Outbox.Step?, digest: String) async throws {
        switch step {
        case .settled(let settled): finish(settled)
        case .signAgain(let number, let offset):
            nextNumber = number
            // Only ever back: an echo's clock is never later than LMAS's real one.
            if offset < 0 { clockOffset = min(clockOffset, offset) }
            saveOrSay()
            outboxChanged()
        case .catchUp(let next):
            let overview: Overview = try await request(.overview).read()
            try await caughtUp(to: overview.latest)
            try await act(on: keepsake.outbox.resolveConflict(next: next, echo: echoes[digest]), digest: digest)
        case nil: break
        }
    }

    private func caughtUp(to latest: UInt64) async throws {
        guard through < latest else { return }
        try await withCheckedThrowingContinuation { catchUps.append((latest, $0)) }
    }

    private func finish(_ settled: Outbox.Settled?) {
        guard let settled else { return }
        reasons[settled.item.id] = nil
        saveOrSay()
        outboxChanged()
        send(.settled(settled))
    }

    private func sign(_ item: Outbox.Item, as device: String) async throws -> SignedIntent {
        let createdAt = UInt64(clamping: Int64(clamping: Self.now()) + clockOffset)
        var intent = SignedIntent(
            device: device, number: nextNumber, createdAt: createdAt, kind: item.draft.kind,
            body: try item.draft.body(contentID: item.id), signature: "")
        intent.signature = try await deviceKeys().sign(intent.preimage, with: item.draft.key, reason: reasons[item.id] ?? "")
        return intent
    }

    private func deviceKeys() throws -> any DeviceKeys {
        if let keys { return keys }
        do {
            guard let found = try chest.keys() else {
                throw Stop(status: .stopped(.init(code: "keys", text: "This Mac's keys are gone. Register it again.")))
            }
            keys = found
            return found
        } catch let stop as Stop {
            throw stop
        } catch {
            throw Stop(status: .stopped(.init(code: "keys", text: "Cannot open this Mac's keys: \(error.localizedDescription)")))
        }
    }

    // MARK: Small things

    private func send(_ update: Update) { emit.yield(update) }
    private func status(_ status: Status) { send(.status(status)) }
    private func outboxChanged() { send(.outbox(keepsake.outbox.items)) }

    /// Losing this write costs at most a repeat LMAS recognises by its content id.
    private func saveOrSay() {
        do {
            try store.save(keepsake)
        } catch {
            send(.notice(.init(code: "disk", text: "Cannot keep the outbox on disk: \(error.localizedDescription)")))
        }
    }

    private static func now() -> UInt64 { UInt64(Date.now.timeIntervalSince1970 * 1000) }

    private static func why(_ error: any Error) -> Why {
        switch error {
        case let error as WireError: Why(code: error.code, text: error.text)
        case is ConnectionLost: Why(code: "lost", text: "LMAS closed the connection.")
        case let stop as Stop:
            switch stop.status {
            case .stopped(let why), .retrying(let why): why
            default: Why(code: "error", text: "\(stop.status)")
            }
        case is ServerLine.Malformed: Why(code: "malformed", text: "LMAS wrote a line Bottega cannot read.")
        case let error as POSIXError where error.code == .ENOENT || error.code == .ECONNREFUSED:
            Why(code: "not_running", text: "LMAS is not running there.")
        case let error as SigningRefused: Why(code: "not_signed", text: error.text)
        default: Why(code: "error", text: error.localizedDescription)
        }
    }

    private func wake() {
        isWoken = true
        let waiting = wakeUps
        wakeUps = []
        for continuation in waiting { continuation.resume() }
    }

    private func cancelled() {
        channel?.close()
        wake()
    }

    private func sleepUntilWoken(atMost limit: Duration? = nil) async {
        hangUp()
        let alarm = limit.map { limit in
            Task {
                try await Task.sleep(for: limit)
                wake()
            }
        }
        defer { alarm?.cancel() }
        if !isWoken, !Task.isCancelled {
            await withCheckedContinuation { wakeUps.append($0) }
        }
        isWoken = false
    }
}
