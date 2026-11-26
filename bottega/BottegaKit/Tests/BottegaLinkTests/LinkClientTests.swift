// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import BottegaKit
import BottegaLink
import Foundation
import Testing

/// A client with everything it says collected, and a way to wait for something to become true.
@MainActor
final class Rig {
    let lmas: FakeLMAS
    let store: MemoryStore
    let client: LinkClient
    private(set) var statuses: [LinkClient.Status] = []
    private(set) var settled: [Outbox.Settled] = []
    private(set) var registrationFailures: [Why] = []
    private(set) var registeredNames: [String?] = []
    private(set) var outbox: [Outbox.Item] = []
    private(set) var overview: Overview?
    private(set) var resets: [UInt64] = []
    private(set) var state = WorkshopState()
    private(set) var brokenSequences = 0
    /// What `watching` said, and the turns as far as pushed.
    private(set) var watching: [[String]?] = []
    private(set) var turns: [String: TurnDetails] = [:]
    private var tasks: [Task<Void, Never>] = []

    init(lmas: FakeLMAS = FakeLMAS(), chest: any KeyChest = FakeChest(), store: MemoryStore = MemoryStore()) {
        self.lmas = lmas
        self.store = store
        client = LinkClient(
            connector: lmas, chest: chest, store: store,
            pace: .init(retry: [.milliseconds(5)], handshake: .seconds(5), batch: .milliseconds(2)))
        let client = client
        tasks.append(Task { await client.run() })
        tasks.append(Task { for await update in client.updates { take(update) } })
    }

    private func take(_ update: LinkClient.Update) {
        switch update {
        case .status(let status): statuses.append(status)
        case .reset(let start):
            resets.append(start)
            state = WorkshopState(startingAt: start)
        case .records(let records):
            for record in records {
                do { try state.apply(record) } catch { brokenSequences += 1 }
            }
        case .settled(let item): settled.append(item)
        case .outbox(let items): outbox = items
        case .registrationFailed(let why): registrationFailures.append(why)
        case .registeredName(let name): registeredNames.append(name)
        case .overview(let overview): self.overview = overview
        case .watching(let running):
            watching.append(running)
            for id in turns.keys where running?.contains(id) != true { turns[id]?.fellBehind() }
        case .live(let lines):
            for line in lines { turns[line.turn, default: TurnDetails()].take(line.what) }
        case .welcome, .notice: break
        }
    }

    var status: LinkClient.Status? { statuses.last }
    var isLive: Bool { status == .live }

    func until(_ what: String, _ condition: () -> Bool, sourceLocation: SourceLocation = #_sourceLocation) async throws {
        for _ in 0..<2000 {
            if condition() { return }
            try await Task.sleep(for: .milliseconds(2))
        }
        Issue.record("never: \(what); status \(String(describing: status))", sourceLocation: sourceLocation)
        throw CancellationError()
    }

    func registered() async throws {
        try await until("asks to register") { status == .needsRegistration }
        await client.register(name: "书房的 Mac Studio")
        try await until("live") { isLive }
    }

    func stop() { for task in tasks { task.cancel() } }
}

private let sayHi = IntentDraft.say(to: "dispatcher", thread: nil, text: "hi", files: [], wantsReply: true)

/// Within every limit as typed, and more than a line holds once written out: an ESC takes seven bytes there.
private let longOnTheWire: IntentDraft = {
    let escapes = String(repeating: "\u{1B}", count: 60_000)
    return .say(
        to: "dispatcher", thread: nil, text: escapes,
        files: [TextFile(name: "a.txt", content: escapes), TextFile(name: "b.txt", content: escapes)], wantsReply: true)
}()

private extension WireError {
    static let peerRefused = WireError(code: "peer_refused", text: "不是认得的程序")
}

@MainActor
@Suite(.timeLimit(.minutes(1)))
struct LinkClientTests {
    // MARK: Registering

    @Test func registersOnceAndFollows() async throws {
        let rig = Rig()
        defer { rig.stop() }
        try await rig.registered()
        #expect(rig.lmas.isRegistered)
        #expect(rig.store.keepsake.device == FakeLMAS.device)
        try await rig.until("sees its own registration") { rig.state.devices[FakeLMAS.device] != nil }
        #expect(rig.state.devices[FakeLMAS.device]?.name == "书房的 Mac Studio")
    }

    /// Nothing is registered, and no connection held open, until the user says so.
    @Test func doesNotRegisterByItself() async throws {
        let rig = Rig()
        defer { rig.stop() }
        try await rig.until("asks") { rig.status == .needsRegistration }
        try await Task.sleep(for: .milliseconds(50))
        #expect(!rig.lmas.isRegistered)
        #expect(rig.statuses == [.needsRegistration])
    }

    @Test func aRefusedFingerRegistersNothing() async throws {
        let keys = FingerKeys()
        keys.allow(false)
        let rig = Rig(chest: FakeChest(makes: keys))
        defer { rig.stop() }
        try await rig.until("asks") { rig.status == .needsRegistration }
        await rig.client.register(name: "Mac")
        try await rig.until("fails") { rig.registrationFailures.count == 1 }
        #expect(rig.registrationFailures.first?.code == "not_signed")
        #expect(!rig.lmas.isRegistered)
        #expect(keys.prompted.first?.contains("Mac") == true)
        keys.allow(true)
        try await rig.until("asks again") { rig.status == .needsRegistration }
        await rig.client.register(name: "Mac")
        try await rig.until("live") { rig.isLive }
    }

    @Test func aLostRegistrationReceiptIsAskedForAgain() async throws {
        let chest = FakeChest()
        let first = Rig(chest: chest)
        try await first.registered()
        first.stop()
        // The device number never reached the disk; the keys did.
        let second = Rig(lmas: first.lmas, chest: chest)
        defer { second.stop() }
        try await second.registered()
        #expect(second.store.keepsake.device == FakeLMAS.device)
        #expect(second.state.devices.count == 1)
    }

    /// LMAS gives the same device number back only under the same name, so it is kept before anything is sent.
    @Test func theNameIsKeptForRegisteringAgain() async throws {
        let rig = Rig()
        try await rig.registered()
        rig.stop()
        #expect(rig.store.keepsake.name == "书房的 Mac Studio")
        let again = Rig(lmas: rig.lmas, store: rig.store)
        defer { again.stop() }
        try await again.until("is told") { !again.registeredNames.isEmpty }
        #expect(again.registeredNames == ["书房的 Mac Studio"])
    }

    /// A refused name is not the one LMAS knows: the one before it stays for the next try.
    @Test func aRefusedNameDoesNotReplaceTheOneLMASKnows() async throws {
        let rig = Rig()
        defer { rig.stop() }
        try await rig.registered()
        try await rig.client.forgetDevice()
        try await rig.until("asks") { rig.status == .needsRegistration }
        await rig.client.register(name: "别的名字")
        try await rig.until("fails") { rig.registrationFailures.count == 1 }
        #expect(rig.registrationFailures.first?.code == "already_registered")
        #expect(rig.store.keepsake.name == "书房的 Mac Studio")
        #expect(rig.registeredNames.last == "书房的 Mac Studio")
        await rig.client.register(name: "书房的 Mac Studio")
        try await rig.until("live") { rig.isLive }
        #expect(rig.store.keepsake.device == FakeLMAS.device)
    }

    @Test func someoneElsesLMASSaysNo() async throws {
        let first = Rig()
        try await first.registered()
        first.stop()
        let second = Rig(lmas: first.lmas)
        defer { second.stop() }
        try await second.until("asks") { second.status == .needsRegistration }
        await second.client.register(name: "Mac")
        try await second.until("fails") { !second.registrationFailures.isEmpty }
        #expect(second.registrationFailures.first?.code == "already_registered")
    }

    @Test func aRefusedProgramIsToldAndStaysPut() async throws {
        let rig = Rig()
        defer { rig.stop() }
        try await rig.registered()
        rig.lmas.set { $0.refuseNext = .peerRefused }
        rig.lmas.dropAll()
        try await rig.until("stopped") { rig.status == .stopped(.init(code: "peer_refused", text: "不是认得的程序")) }
        let count = rig.statuses.count
        try await Task.sleep(for: .milliseconds(50))
        #expect(rig.statuses.count == count)
        await rig.client.reconnect()
        try await rig.until("live again") { rig.isLive }
    }

    // MARK: Following

    @Test func missesNothingAcrossADrop() async throws {
        let rig = Rig()
        defer { rig.stop() }
        try await rig.registered()
        rig.lmas.append(#"{"type":"session_registered","name":"a","role":"coder","at_ms":1}"#, spans: 3)
        try await rig.until("a") { rig.state.sessions["a"] != nil }
        rig.lmas.set { $0.isRunning = false }
        rig.lmas.dropAll()
        try await rig.until("retrying") { if case .retrying = rig.status { true } else { false } }
        rig.lmas.append(#"{"type":"session_registered","name":"b","role":"coder","at_ms":1}"#)
        rig.lmas.append("", spans: 2)
        rig.lmas.set { $0.isRunning = true }
        try await rig.until("live") { rig.isLive && rig.state.through == rig.lmas.through }
        #expect(rig.state.sessions["b"] != nil)
        #expect(rig.lmas.followedFrom.last == 5)
        #expect(rig.brokenSequences == 0)
        // Only the registration started from nothing.
        #expect(rig.resets == [1])
    }

    @Test func stopsWhenNumbersDoNotFollowOn() async throws {
        let rig = Rig()
        defer { rig.stop() }
        try await rig.registered()
        rig.lmas.append(#"{"type":"session_registered","name":"a","role":"coder","at_ms":1}"#)
        try await rig.until("a") { rig.state.sessions["a"] != nil }
        rig.lmas.sendBroken()
        try await rig.until("stopped") { if case .stopped(let why) = rig.status { why.code == "sequence" } else { false } }
        let through = rig.state.through
        rig.lmas.append(#"{"type":"session_registered","name":"b","role":"coder","at_ms":1}"#)
        try await Task.sleep(for: .milliseconds(30))
        #expect(rig.state.through == through)
        await rig.client.reload()
        try await rig.until("live") { rig.isLive && rig.state.sessions["b"] != nil }
    }

    /// A line that cannot be read is not cured by trying again: it stops there, says so, and skips nothing.
    @Test(arguments: [#"{"record":{"seq":"two"}}"#, #"{"id":1,"error":{"code":5}}"#, "not json"])
    func stopsAtALineItCannotRead(line: String) async throws {
        let rig = Rig()
        defer { rig.stop() }
        try await rig.registered()
        let connections = rig.lmas.connections
        rig.lmas.send(line)
        try await rig.until("stopped") { if case .stopped(let why) = rig.status { why.code == "malformed" } else { false } }
        // The retry pace here is 5 ms: left to itself it would have connected several times over by now.
        try await Task.sleep(for: .milliseconds(60))
        #expect(rig.lmas.connections == connections)
        if case .stopped = rig.status {} else { Issue.record("went on by itself: \(String(describing: rig.status))") }
        rig.lmas.append(#"{"type":"session_registered","name":"b","role":"coder","at_ms":1}"#)
        await rig.client.reconnect()
        try await rig.until("live") { rig.isLive && rig.state.sessions["b"] != nil }
        #expect(rig.brokenSequences == 0)
    }

    /// An answer that is not what was asked for is a line that cannot be read, like any other.
    @Test func stopsAtAnAnswerItCannotRead() async throws {
        let rig = Rig()
        defer { rig.stop() }
        try await rig.registered()
        rig.lmas.set { $0.garblesFollow = true }
        rig.lmas.dropAll()
        try await rig.until("stopped") { if case .stopped(let why) = rig.status { why.code == "malformed" } else { false } }
        let connections = rig.lmas.connections
        try await Task.sleep(for: .milliseconds(60))
        #expect(rig.lmas.connections == connections)
        rig.lmas.set { $0.garblesFollow = false }
        await rig.client.reconnect()
        try await rig.until("live") { rig.isLive }
    }

    /// The other things trying again does not cure: a line longer than is taken, LMAS not able to read its own log.
    @Test func stopsAtALineLongerThanItTakes() async throws {
        let rig = Rig()
        defer { rig.stop() }
        try await rig.registered()
        rig.lmas.overrun()
        try await rig.until("stopped") { if case .stopped(let why) = rig.status { why.code == "too_long" } else { false } }
        let attempts = rig.lmas.attempts
        try await Task.sleep(for: .milliseconds(60))
        #expect(rig.lmas.attempts == attempts)
        await rig.client.reconnect()
        try await rig.until("live") { rig.isLive }
    }

    @Test func stopsWhenThePathToTheSocketIsTooLong() async throws {
        let rig = Rig()
        defer { rig.stop() }
        try await rig.registered()
        rig.lmas.set { $0.connectError = .ENAMETOOLONG }
        rig.lmas.dropAll()
        try await rig.until("stopped") { if case .stopped(let why) = rig.status { why.code == "socket_path" } else { false } }
        let attempts = rig.lmas.attempts
        try await Task.sleep(for: .milliseconds(60))
        #expect(rig.lmas.attempts == attempts)
        rig.lmas.set { $0.connectError = nil }
        await rig.client.reconnect()
        try await rig.until("live") { rig.isLive }
    }

    @Test func stopsWhenLMASCannotReadItsOwnLog() async throws {
        let rig = Rig()
        defer { rig.stop() }
        try await rig.registered()
        rig.lmas.set { $0.followError = "unreadable" }
        rig.lmas.dropAll()
        try await rig.until("stopped") { if case .stopped(let why) = rig.status { why.code == "unreadable" } else { false } }
        let attempts = rig.lmas.attempts
        try await Task.sleep(for: .milliseconds(60))
        #expect(rig.lmas.attempts == attempts)
        rig.lmas.set { $0.followError = nil }
        await rig.client.reconnect()
        try await rig.until("live") { rig.isLive }
    }

    /// The other side of stopping: what passes by itself is tried again, and comes right without anyone's doing.
    @Test(arguments: ["busy", "stopping"])
    func keepsTryingWhileLMASSaysLater(code: String) async throws {
        let rig = Rig()
        defer { rig.stop() }
        try await rig.registered()
        rig.lmas.set { $0.followError = code }
        rig.lmas.dropAll()
        try await rig.until("retrying") { if case .retrying(let why) = rig.status { why.code == code } else { false } }
        let attempts = rig.lmas.attempts
        try await rig.until("tries again and again") { rig.lmas.attempts >= attempts + 3 }
        #expect(!rig.statuses.contains { if case .stopped = $0 { true } else { false } })
        rig.lmas.set { $0.followError = nil }
        try await rig.until("live by itself") { rig.isLive }
    }

    /// The same for a connection that will not come about for a while.
    @Test func keepsTryingWhileNobodyIsThere() async throws {
        let rig = Rig()
        defer { rig.stop() }
        try await rig.registered()
        rig.lmas.set { $0.connectError = .ECONNREFUSED }
        rig.lmas.dropAll()
        try await rig.until("retrying") { if case .retrying(let why) = rig.status { why.code == "not_running" } else { false } }
        let attempts = rig.lmas.attempts
        try await rig.until("tries again and again") { rig.lmas.attempts >= attempts + 3 }
        rig.lmas.set { $0.connectError = nil }
        try await rig.until("live by itself") { rig.isLive }
    }

    /// What goes on inside turns is an extra: an answer to `watch` that cannot be read does not stop the events.
    @Test func followsAllTheSameWhenWatchCannotBeRead() async throws {
        let lmas = FakeLMAS()
        lmas.set { $0.turns = [:]; $0.garblesWatch = true }
        let rig = Rig(lmas: lmas)
        defer { rig.stop() }
        try await rig.registered()
        rig.lmas.append(#"{"type":"session_registered","name":"a","role":"coder","at_ms":1}"#)
        try await rig.until("a") { rig.state.sessions["a"] != nil }
        #expect(rig.isLive)
        #expect(rig.watching.isEmpty)
        // With an answer that can be read, it is watching.
        lmas.set { $0.garblesWatch = false }
        rig.lmas.dropAll()
        try await rig.until("watching") { rig.watching.contains { $0 != nil } }
    }

    /// A kind of line this Bottega does not know is passed over, and what follows is taken as before.
    @Test func aKindOfLineItDoesNotKnowIsPassedOver() async throws {
        let rig = Rig()
        defer { rig.stop() }
        try await rig.registered()
        let connections = rig.lmas.connections
        rig.lmas.send(#"{"coffee":{"cups":2}}"#)
        rig.lmas.append(#"{"type":"session_registered","name":"a","role":"coder","at_ms":1}"#)
        try await rig.until("a") { rig.state.sessions["a"] != nil }
        #expect(rig.isLive)
        #expect(rig.lmas.connections == connections)
    }

    @Test func startsWhereTheLogStarts() async throws {
        let rig = Rig()
        defer { rig.stop() }
        for index in 0..<5 { rig.lmas.append(#"{"type":"session_registered","name":"s\#(index)","role":"coder","at_ms":1}"#, spans: 2) }
        rig.lmas.set { $0.oldest = 6 }
        try await rig.registered()
        // 6 falls inside the record that began at 5, which comes whole.
        #expect(rig.state.start == 6)
        #expect(rig.state.sessions["s2"] != nil && rig.state.sessions["s1"] == nil)
        #expect(rig.brokenSequences == 0)
    }

    /// The log was restored to an older one: what was shown is not its past.
    @Test func aShorterLogIsAnotherLog() async throws {
        let chest = FakeChest()
        let rig = Rig(chest: chest)
        defer { rig.stop() }
        try await rig.registered()
        rig.lmas.append(#"{"type":"session_registered","name":"a","role":"coder","at_ms":1}"#)
        try await rig.until("a") { rig.state.sessions["a"] != nil }
        rig.lmas.wipe()
        try await rig.until("handshake refused") { if case .stopped(let why) = rig.status { why.code == "handshake" } else { false } }
        #expect(chest.timesMade == 1)
        try await rig.client.forgetDevice()
        try await rig.registered()
        // A device LMAS may still know by its keys: they are not thrown away.
        #expect(chest.timesMade == 1)
        #expect(rig.state.sessions["a"] == nil)
        #expect(rig.state.through == rig.lmas.through)
    }

    @Test func seenGoesOut() async throws {
        let rig = Rig()
        defer { rig.stop() }
        try await rig.registered()
        #expect(await rig.client.seen(through: 1))
        #expect(rig.lmas.seen == [1])
        // Only what LMAS took note of counts as said.
        #expect(await rig.client.seen(through: 99) == false)
        rig.lmas.set { $0.isRunning = false }
        rig.lmas.dropAll()
        try await rig.until("gone") { !rig.isLive }
        #expect(await rig.client.seen(through: 1) == false)
    }

    /// Someone speaking under this device's number shows up, and so does this client's own mistake.
    @Test func aDroppedIntentIsCounted() async throws {
        let rig = Rig()
        defer { rig.stop() }
        try await rig.registered()
        try await rig.until("overview") { rig.overview?.droppedIntents == 0 }
        rig.lmas.forgetKeys()
        try await rig.client.submit(sayHi)
        try await rig.until("settled") { rig.settled.count == 1 }
        #expect(rig.settled.first?.result == .notSent(.init(code: "dropped", text: "dropped")))
        try await rig.until("counted") { rig.overview?.droppedIntents == 1 }
    }

    // MARK: Intents

    @Test func saysSomething() async throws {
        let rig = Rig()
        defer { rig.stop() }
        try await rig.registered()
        let id = try await rig.client.submit(sayHi)
        try await rig.until("settled") { rig.settled.count == 1 }
        #expect(rig.settled.first?.item.id == id)
        #expect(rig.settled.first?.result == .done(message: "msg-1"))
        #expect(rig.outbox.isEmpty)
        #expect(rig.lmas.executed.count == 1)
        #expect(rig.store.keepsake.outbox.isEmpty)
    }

    @Test func numbersRunInOrderWithoutGaps() async throws {
        let rig = Rig()
        defer { rig.stop() }
        try await rig.registered()
        for index in 0..<10 {
            try await rig.client.submit(.say(to: "dispatcher", thread: nil, text: "\(index)", files: [], wantsReply: true))
        }
        try await rig.until("settled") { rig.settled.count == 10 }
        #expect(rig.lmas.handedIn.map(\.number) == Array(1...10))
        #expect(rig.lmas.executed.count == 10)
        #expect(rig.settled.map { $0.item.draft.typed?.text } == (0..<10).map { "\($0)" })
    }

    /// The answer was lost: the same number and the same bytes go out again, and it is done once.
    @Test(arguments: [(1, 0), (0, 1), (3, 0), (2, 2)])
    func aLostAnswerIsDoneOnce(before: Int, after: Int) async throws {
        let rig = Rig()
        defer { rig.stop() }
        try await rig.registered()
        rig.lmas.set { $0.dropBeforeIntent = before; $0.dropAfterIntent = after }
        try await rig.client.submit(sayHi)
        try await rig.until("settled") { rig.settled.count == 1 }
        #expect(rig.settled.first?.result == .done(message: "msg-1"))
        #expect(rig.lmas.executed.count == 1)
        let handedIn = rig.lmas.handedIn
        #expect(handedIn.count >= 2)
        #expect(handedIn.allSatisfy { $0 == handedIn[0] })
    }

    /// The app went away between sending and hearing back.
    @Test func survivesARestart() async throws {
        let chest = FakeChest(), store = MemoryStore()
        let first = Rig(chest: chest, store: store)
        try await first.registered()
        first.lmas.set { $0.dropAfterIntent = 1; $0.isRunning = true }
        try await first.client.submit(sayHi)
        try await first.until("handed in") { !first.lmas.handedIn.isEmpty }
        first.stop()
        let before = first.lmas.executed.count
        let second = Rig(lmas: first.lmas, chest: chest, store: store)
        defer { second.stop() }
        try await second.until("live") { second.isLive }
        try await second.until("nothing left") { store.keepsake.outbox.isEmpty }
        #expect(before == 1)
        #expect(second.lmas.executed.count == 1)
    }

    @Test func whatIsSaidOfflineGoesOutLater() async throws {
        let rig = Rig()
        defer { rig.stop() }
        try await rig.registered()
        rig.lmas.set { $0.isRunning = false }
        rig.lmas.dropAll()
        try await rig.until("retrying") { if case .retrying = rig.status { true } else { false } }
        try await rig.client.submit(sayHi)
        try await rig.until("queued") { rig.outbox.count == 1 }
        rig.lmas.set { $0.isRunning = true }
        try await rig.until("settled") { rig.settled.count == 1 }
        #expect(rig.lmas.executed.count == 1)
    }

    /// Someone used up the number with something else: looked up in the events, then said under another number.
    @Test func aConflictIsNumberedAgain() async throws {
        let rig = Rig()
        defer { rig.stop() }
        try await rig.registered()
        rig.lmas.useUpANumber()
        try await rig.client.submit(sayHi)
        try await rig.until("settled") { rig.settled.count == 1 }
        #expect(rig.settled.first?.result == .done(message: "msg-2"))
        #expect(rig.lmas.handedIn.map(\.number) == [1, 2])
        #expect(Set(rig.lmas.handedIn.map(\.body)).count == 1)
        #expect(rig.lmas.executed.count == 1)
    }

    /// LMAS's books went back (a restored log): the number is too high, so it takes the one LMAS names.
    @Test func aGapTakesTheNumberLMASNames() async throws {
        let chest = FakeChest(), store = MemoryStore()
        let first = Rig(chest: chest, store: store)
        try await first.registered()
        for _ in 0..<3 { try await first.client.submit(sayHi) }
        try await first.until("settled") { first.settled.count == 3 }
        first.lmas.set { $0.dropBeforeIntent = 1_000 }
        try await first.client.submit(sayHi)
        try await first.until("handed in") { first.lmas.handedIn.count >= 4 }
        first.stop()
        #expect(store.keepsake.outbox.head?.signed?.number == 4)
        first.lmas.wipe()
        first.lmas.set { $0.dropBeforeIntent = 0 }
        var keepsake = store.keepsake
        keepsake.device = nil
        try store.save(keepsake)
        let second = Rig(lmas: first.lmas, chest: chest, store: store)
        defer { second.stop() }
        try await second.registered()
        try await second.until("settled") { second.settled.count == 1 }
        #expect(second.settled.first?.result == .done(message: "msg-1"))
    }

    @Test func aRefusalGivesTheWordsBack() async throws {
        let rig = Rig()
        defer { rig.stop() }
        try await rig.registered()
        rig.lmas.refuse("say", code: "no_such_session", text: "没有这个会话")
        try await rig.client.submit(sayHi)
        try await rig.until("settled") { rig.settled.count == 1 }
        let settled = try #require(rig.settled.first)
        #expect(settled.result == .refused(.init(code: "no_such_session", text: "没有这个会话")))
        #expect(settled.isUndone)
        #expect(settled.item.draft.typed?.text == "hi")
        // The refusal used up its number; the next one goes on from there.
        rig.lmas.refuse("say", code: "x", text: "x")
        try await rig.client.submit(.decline(ask: "ask-1"))
        try await rig.until("settled") { rig.settled.count == 2 }
        #expect(rig.lmas.handedIn.map(\.number) == [1, 2])
    }

    @Test func aFastClockIsSetBack() async throws {
        let rig = Rig()
        defer { rig.stop() }
        try await rig.registered()
        rig.lmas.set { $0.clock = -3_600_000 }
        try await rig.client.submit(sayHi)
        try await rig.until("settled") { rig.settled.count == 1 }
        #expect(rig.settled.first?.result == .done(message: "msg-2"))
        try await rig.client.submit(sayHi)
        try await rig.until("settled") { rig.settled.count == 2 }
        // The second one had the right time at once.
        #expect(rig.lmas.handedIn.count == 3)
    }

    @Test func tooLongIsSaidBeforeAnythingIsSent() async throws {
        let rig = Rig()
        defer { rig.stop() }
        try await rig.registered()
        await #expect(throws: IntentDraft.Problem.self) {
            try await rig.client.submit(.say(to: "d", thread: nil, text: String(repeating: "x", count: 70_000), files: [], wantsReply: true))
        }
        #expect(rig.lmas.handedIn.isEmpty)
    }

    /// Too long only once written out: said before anything is kept, like every other limit.
    @Test func tooLongOnTheWireIsSaidBeforeAnythingIsKept() async throws {
        let rig = Rig()
        defer { rig.stop() }
        try await rig.registered()
        await #expect(throws: IntentDraft.Problem.self) { try await rig.client.submit(longOnTheWire) }
        #expect(rig.store.keepsake.outbox.isEmpty)
        #expect(rig.lmas.handedIn.isEmpty)
    }

    /// Kept by a build that did not check for it, it can never go out: it is given back, and what waits behind it goes.
    @Test func whatCanNeverGoOutDoesNotHoldUpTheRest() async throws {
        let chest = FakeChest(), store = MemoryStore()
        let first = Rig(chest: chest, store: store)
        try await first.registered()
        first.stop()
        var keepsake = store.keepsake
        keepsake.outbox.add(longOnTheWire, id: ContentID.random(), at: 1)
        keepsake.outbox.add(sayHi, id: ContentID.random(), at: 2)
        try store.save(keepsake)
        let second = Rig(lmas: first.lmas, chest: chest, store: store)
        defer { second.stop() }
        try await second.until("both settled") { second.settled.count == 2 }
        guard case .notSent(let why) = second.settled[0].result else {
            Issue.record("settled as \(second.settled[0].result)")
            return
        }
        #expect(why.code == "too_long")
        #expect(second.settled[0].item.draft == longOnTheWire)
        #expect(second.lmas.executed.map(\.kind) == ["say"])
        #expect(second.lmas.handedIn.count == 1)
        #expect(store.keepsake.outbox.isEmpty)
    }

    /// LMAS did it and its answer cannot be read: whether it was done is not known, so it stops with the signed
    /// bytes kept, and hands in those same bytes again once someone says to go on. Done once.
    @Test func anAnswerToAnIntentItCannotReadStopsAndIsAskedForAgain() async throws {
        let rig = Rig()
        defer { rig.stop() }
        try await rig.registered()
        rig.lmas.set { $0.garblesIntent = true }
        try await rig.client.submit(sayHi)
        try await rig.until("stopped") { if case .stopped(let why) = rig.status { why.code == "malformed" } else { false } }
        #expect(rig.settled.isEmpty)
        let kept = try #require(rig.store.keepsake.outbox.head?.signed)
        let attempts = rig.lmas.attempts
        try await Task.sleep(for: .milliseconds(60))
        #expect(rig.lmas.attempts == attempts)
        rig.lmas.set { $0.garblesIntent = false }
        await rig.client.reconnect()
        try await rig.until("settled") { rig.settled.count == 1 }
        #expect(rig.settled.first?.result == .done(message: "msg-1"))
        #expect(rig.lmas.handedIn == [kept, kept])
        #expect(rig.lmas.executed.count == 1)
    }

    /// What cannot be kept is not sent: a crash after sending could do it twice.
    @Test func nothingGoesOutThatIsNotOnDisk() async throws {
        let rig = Rig()
        defer { rig.stop() }
        try await rig.registered()
        rig.store.breakDisk(true)
        await #expect(throws: (any Error).self) { try await rig.client.submit(sayHi) }
        #expect(rig.lmas.handedIn.isEmpty)
        rig.store.breakDisk(false)
        try await rig.client.submit(sayHi)
        try await rig.until("settled") { rig.settled.count == 1 }
    }

    // MARK: Approving

    @Test func approvingPromptsEachTimeAndDecliningNever() async throws {
        let keys = FingerKeys()
        let rig = Rig(chest: FakeChest(makes: keys))
        defer { rig.stop() }
        try await rig.registered()
        for index in 1...3 {
            try await rig.client.submit(
                .approve(ask: "ask-\(index)", textDigest: Digest.sha256("合？")), touchIDReason: "merge card-\(index) into main")
        }
        try await rig.client.submit(.decline(ask: "ask-4"))
        try await rig.until("settled") { rig.settled.count == 4 }
        #expect(keys.prompted.dropFirst() == ["merge card-1 into main", "merge card-2 into main", "merge card-3 into main"])
        #expect(rig.lmas.executed.map(\.kind) == ["approve", "approve", "approve", "decline"])
        #expect(rig.lmas.executed[0].body.contains(Digest.sha256("合？")))
    }

    @Test func aRefusedFingerSendsNothingAndUsesNoNumber() async throws {
        let keys = FingerKeys()
        let rig = Rig(chest: FakeChest(makes: keys))
        defer { rig.stop() }
        try await rig.registered()
        keys.allow(false)
        try await rig.client.submit(.approve(ask: "ask-1", textDigest: "d"), touchIDReason: "x")
        try await rig.until("settled") { rig.settled.count == 1 }
        #expect(rig.settled.first?.result == .notSent(.init(code: "not_signed", text: "Touch ID was cancelled.")))
        #expect(rig.lmas.handedIn.isEmpty)
        try await rig.client.submit(sayHi)
        try await rig.until("settled") { rig.settled.count == 2 }
        #expect(rig.lmas.handedIn.map(\.number) == [1])
    }

    /// No prompt pops up later, when LMAS comes back.
    @Test func approvingNeedsAConnection() async throws {
        let keys = FingerKeys()
        let rig = Rig(chest: FakeChest(makes: keys))
        defer { rig.stop() }
        try await rig.registered()
        rig.lmas.set { $0.isRunning = false }
        rig.lmas.dropAll()
        try await rig.until("retrying") { if case .retrying = rig.status { true } else { false } }
        await #expect(throws: LinkClient.ConnectionLost.self) {
            try await rig.client.submit(.approve(ask: "ask-1", textDigest: "d"), touchIDReason: "x")
        }
        rig.lmas.set { $0.isRunning = true }
        try await rig.until("live") { rig.isLive }
        #expect(keys.prompted.count == 1)  // The registration's.
    }

    /// Decided looking at how things were, it is not done later when they are otherwise: given back there and then.
    @Test(arguments: [
        IntentDraft.decline(ask: "ask-1"), .stopTurn("turn-1"), .setModel(line: "m1", session: "a", model: "x", effort: nil),
        .removeProject(name: "p"), .addProject(name: "p", path: "/tmp/p"), .approve(ask: "ask-1", textDigest: "d"),
    ])
    func decidingNeedsAConnection(draft: IntentDraft) async throws {
        #expect(!draft.keeps)
        let rig = Rig()
        defer { rig.stop() }
        try await rig.registered()
        rig.lmas.set { $0.isRunning = false }
        rig.lmas.dropAll()
        try await rig.until("retrying") { if case .retrying = rig.status { true } else { false } }
        await #expect(throws: LinkClient.ConnectionLost.self) { try await rig.client.submit(draft, touchIDReason: "x") }
        #expect(rig.store.keepsake.outbox.isEmpty)
        rig.lmas.set { $0.isRunning = true }
        try await rig.until("live") { rig.isLive }
        // Nothing of it was kept to go out now.
        try await Task.sleep(for: .milliseconds(20))
        #expect(rig.lmas.handedIn.isEmpty)
        // Asked again while connected, it goes.
        try await rig.client.submit(draft, touchIDReason: "x")
        try await rig.until("settled") { rig.settled.count == 1 }
        #expect(rig.lmas.handedIn.map(\.kind) == [draft.kind])
    }

    /// The other side: what is said, answered, opened, arranged and read is kept while there is no connection.
    @Test(arguments: [
        sayHi, .answer(message: "msg-1", text: "yes", files: []),
        .openLine(.init(project: "p", vendor: "codex", role: "coder", title: nil, text: "go", files: [])),
        .arrange(line: "msg-1", title: nil, pinned: true, archived: nil), .markRead(line: "msg-1", through: 3),
    ])
    func whatIsKeptIsTakenWithoutAConnection(draft: IntentDraft) async throws {
        #expect(draft.keeps)
        let rig = Rig()
        defer { rig.stop() }
        try await rig.registered()
        rig.lmas.set { $0.isRunning = false }
        rig.lmas.dropAll()
        try await rig.until("retrying") { if case .retrying = rig.status { true } else { false } }
        try await rig.client.submit(draft)
        #expect(rig.store.keepsake.outbox.head?.draft == draft)
        rig.lmas.set { $0.isRunning = true }
        try await rig.until("settled") { rig.settled.count == 1 }
        #expect(rig.lmas.executed.map(\.kind) == [draft.kind])
    }

    /// Connected is not enough: until everything LMAS has is here, what is shown is how things were.
    @Test func decidingWaitsUntilCaughtUp() async throws {
        let rig = Rig()
        defer { rig.stop() }
        try await rig.registered()
        rig.lmas.set { $0.holdsCaughtUp = true }
        rig.lmas.dropAll()
        try await rig.until("catching up") { if case .catchingUp = rig.status { true } else { false } }
        await #expect(throws: LinkClient.ConnectionLost.self) { try await rig.client.submit(.decline(ask: "ask-1")) }
        await #expect(throws: LinkClient.ConnectionLost.self) { try await rig.client.submit(.stopTurn("turn-1")) }
        // What is said is kept all the same.
        try await rig.client.submit(sayHi)
        rig.lmas.catchUp()
        try await rig.until("live") { rig.isLive }
        try await rig.until("said") { rig.settled.count == 1 }
        try await rig.client.submit(.decline(ask: "ask-1"))
        try await rig.until("declined") { rig.settled.count == 2 }
        #expect(rig.lmas.executed.map(\.kind) == ["say", "decline"])
    }
}

struct UnixSocketTests {
    /// A real socket: what one end writes the other reads, line by line, and hanging up ends it.
    @Test(.timeLimit(.minutes(1))) func carriesLines() async throws {
        let path = NSTemporaryDirectory() + "b\(UInt32.random(in: 0...UInt32.max)).sock"
        let listener = try Listener(path: path)
        defer { listener.close() }
        let channel = try await UnixSocketConnector(path: path).connect()
        let peer = try listener.accept()
        try peer.write(contentsOf: Data("one\n二\npar".utf8))
        var lines = channel.lines().makeAsyncIterator()
        #expect(try await lines.next() == Data("one".utf8))
        #expect(try await lines.next() == Data("二".utf8))
        try peer.write(contentsOf: Data("t\n".utf8))
        #expect(try await lines.next() == Data("part".utf8))
        let big = Data(repeating: UInt8(ascii: "x"), count: 600_000) + Data("\n".utf8)
        async let sent: Void = channel.send(big)
        var got = Data()
        var chunk = [UInt8](repeating: 0, count: 65_536)
        // Not FileHandle's read, which waits until it has as much as was asked for.
        while got.count < big.count {
            let count = read(peer.fileDescriptor, &chunk, chunk.count)
            try #require(count > 0)
            got.append(contentsOf: chunk[..<count])
        }
        try await sent
        #expect(got == big)
        try peer.close()
        #expect(try await lines.next() == nil)
        await #expect(throws: (any Error).self) {
            for _ in 0..<50 { try await channel.send(Data("late\n".utf8)) }
        }
    }

    @Test func saysWhenNobodyIsThere() async throws {
        await #expect(throws: POSIXError(.ENOENT)) {
            _ = try await UnixSocketConnector(path: NSTemporaryDirectory() + "nobody.sock").connect()
        }
        await #expect(throws: POSIXError(.ENAMETOOLONG)) {
            _ = try await UnixSocketConnector(path: "/" + String(repeating: "x", count: 200)).connect()
        }
    }

    private struct Listener {
        let descriptor: Int32
        let path: String

        init(path: String) throws {
            self.path = path
            descriptor = socket(AF_UNIX, SOCK_STREAM, 0)
            var address = sockaddr_un()
            address.sun_family = sa_family_t(AF_UNIX)
            withUnsafeMutableBytes(of: &address.sun_path) { $0.copyBytes(from: Array(path.utf8)) }
            let bound = withUnsafePointer(to: &address) {
                $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { bind(descriptor, $0, socklen_t(MemoryLayout<sockaddr_un>.size)) }
            }
            guard descriptor >= 0, bound == 0, listen(descriptor, 1) == 0 else { throw POSIXError(.init(rawValue: errno) ?? .EIO) }
        }

        func accept() throws -> FileHandle {
            let peer = Darwin.accept(descriptor, nil, nil)
            guard peer >= 0 else { throw POSIXError(.init(rawValue: errno) ?? .EIO) }
            return FileHandle(fileDescriptor: peer, closeOnDealloc: false)
        }

        func close() {
            Darwin.close(descriptor)
            unlink(path)
        }
    }
}

struct FileKeepsakeTests {
    /// A folder that was there already is closed to others first; one that is a link is not written through.
    @Test func theFolderIsTightened() throws {
        let files = FileManager.default
        let root = files.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? files.removeItem(at: root) }
        let folder = root.appendingPathComponent("open")
        try files.createDirectory(at: folder, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o755])
        let url = folder.appendingPathComponent("key")
        try PrivateFile.write(Data("one".utf8), to: url)
        try PrivateFile.write(Data("two".utf8), to: url)
        #expect(try files.attributesOfItem(atPath: folder.path)[.posixPermissions] as? Int == 0o700)
        #expect(try files.attributesOfItem(atPath: url.path)[.posixPermissions] as? Int == 0o600)
        #expect(try Data(contentsOf: url) == Data("two".utf8))
        #expect(try files.contentsOfDirectory(atPath: folder.path) == ["key"])

        let link = root.appendingPathComponent("link")
        try files.createSymbolicLink(at: link, withDestinationURL: folder)
        #expect(throws: (any Error).self) { try PrivateFile.write(Data(), to: link.appendingPathComponent("key")) }
        #expect(try Data(contentsOf: url) == Data("two".utf8))
    }

    @Test func keptPrivately() throws {
        let folder = FileManager.default.temporaryDirectory.appendingPathComponent("keepsake-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: folder) }
        let url = folder.appendingPathComponent("deep/link.json")
        let store = FileKeepsakeStore(url: url)
        #expect(try store.load() == Keepsake())
        var keepsake = Keepsake()
        keepsake.device = "device-1"
        keepsake.outbox.add(sayHi, id: "a", at: 1)
        try store.save(keepsake)
        try store.save(keepsake)
        #expect(try store.load() == keepsake)
        let mode = try FileManager.default.attributesOfItem(atPath: url.path)[.posixPermissions] as? Int
        #expect(mode == 0o600)
        #expect(try FileManager.default.contentsOfDirectory(atPath: url.deletingLastPathComponent().path) == ["link.json"])
    }

    /// A record that cannot be read is not quietly replaced by an empty one.
    @Test func aBrokenFileIsAnError() throws {
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("broken-\(UUID().uuidString).json")
        defer { try? FileManager.default.removeItem(at: url) }
        try Data("{".utf8).write(to: url)
        #expect(throws: (any Error).self) { try FileKeepsakeStore(url: url).load() }
    }
}
