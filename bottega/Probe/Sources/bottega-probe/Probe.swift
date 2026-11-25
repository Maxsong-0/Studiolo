// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import BottegaKit
import BottegaLink
import CryptoKit
import Foundation
import Synchronization

/// `bottega-probe <run directory> <state directory>`, then a command a line. For a throwaway LMAS only, such as
/// `lmas-local demo`: what it registers is a device whose keys lie in files.
@main
struct Probe {
    static func main() async throws {
        let arguments = CommandLine.arguments
        guard arguments.count == 3 || arguments.count == 4 else {
            print("usage: bottega-probe <LMAS run directory> <state directory> [<file for the records, as LMAS wrote them>]")
            exit(2)
        }
        setvbuf(stdout, nil, _IOLBF, 0)
        let run = URL(filePath: arguments[1], directoryHint: .isDirectory)
        let state = URL(filePath: arguments[2], directoryHint: .isDirectory)
        let connector = Recorder(
            inner: UnixSocketConnector(path: run.appending(path: "ui.sock").path),
            file: arguments.count > 3 ? URL(filePath: arguments[3]) : nil)
        let client = LinkClient(
            connector: connector, chest: FileChest(folder: state), store: FileKeepsakeStore(url: state.appending(path: "link.json")))
        let ledger = Ledger()
        let running = Task { await client.run() }
        let watching = Task { for await update in client.updates { await ledger.take(update) } }

        for try await line in FileHandle.standardInput.bytes.lines {
            let words = line.split(separator: " ", maxSplits: 1).map(String.init)
            let rest = words.count > 1 ? words[1] : ""
            /// The first `count` words of the rest, then what is left of it.
            func parts(_ count: Int) -> [String] {
                let parts = rest.split(separator: " ", maxSplits: count).map(String.init)
                return parts + Array(repeating: "", count: max(0, count + 1 - parts.count))
            }
            do {
                switch words.first {
                case "register": await client.register(name: rest)
                case "forget": try await client.forgetDevice()
                case "reconnect": await client.reconnect()
                case "reload": await client.reload()
                case "seen":
                    if let through = await ledger.seenThrough {
                        print("seen: \(through), noted \(await client.seen(through: through))")
                    } else {
                        print("seen: nothing to say")
                    }
                case "seen-at": print("seen: noted \(await client.seen(through: UInt64(rest) ?? 0))")
                case "say":
                    let to = parts(1)
                    try await client.submit(.say(to: to[0], thread: nil, text: to[1], files: [], wantsReply: true))
                case "interject", "tell":
                    let to = parts(2)
                    let thread = to[0] == "last" ? await ledger.lastLine : to[0]
                    try await client.submit(
                        .say(to: to[1], thread: thread, text: to[2], files: [], wantsReply: words[0] == "interject"))
                case "file":
                    let to = parts(1)
                    let files = [TextFile(name: "说明.txt", content: String(repeating: "x", count: Int(to[1]) ?? 10))]
                    try await client.submit(.say(to: to[0], thread: nil, text: "看附件。", files: files, wantsReply: true))
                case "answer":
                    let to = parts(1)
                    let message = to[0] == "last" ? await ledger.lastQuestion : to[0]
                    try await client.submit(.answer(message: message, text: to[1], files: []))
                case "approve-unread":
                    let ask = rest == "last" ? await ledger.lastAsk.id : rest
                    try await client.submit(.approve(ask: ask, textDigest: Digest.sha256("别的字")))
                case "asks": await ledger.tellAsks()
                case "approve", "decline":
                    let ask = rest == "last" ? await ledger.lastAsk : (id: rest, text: await ledger.text(of: rest))
                    try await client.submit(
                        words[0] == "approve"
                            ? .approve(ask: ask.id, textDigest: Digest.sha256(ask.text)) : .decline(ask: ask.id))
                case "flood":
                    let to = parts(1)
                    for count in 1...(Int(to[1]) ?? 1) {
                        try await client.submit(.say(to: to[0], thread: nil, text: "第 \(count) 句", files: [], wantsReply: true))
                    }
                case "cut":
                    connector.losesNextReply.withLock { $0 = true }
                    print("cut: the answer to the next intent gets lost")
                case "overview": await client.refreshOverview()
                case "turns": await ledger.tellTurns()
                case "turn": await ledger.compare(rest, with: try await client.turn(rest))
                case "mail": await ledger.tellMail()
                case "state": await ledger.tell()
                case "wait": try await Task.sleep(for: .milliseconds(Int(rest) ?? 1000))
                case "quit":
                    running.cancel()
                    await running.value
                    await watching.value
                    return
                case nil: break
                default: print("? \(line)")
                }
            } catch {
                print("failed: \(error)")
            }
        }
    }
}

/// Everything the client said, reduced as the app reduces it.
actor Ledger {
    private var state = WorkshopState()
    private var records = 0
    private var events = 0
    private var unreadable = 0
    private var first: UInt64?
    private var inside: [String: TurnDetails] = [:]
    /// With every letter taken as shown.
    var seenThrough: UInt64? { state.seenThrough(shown: Set(state.messagesToUser)) }
    var lastLine: String { state.messages.values.filter(\.isFromUser).max { $0.seq < $1.seq }?.thread ?? "" }
    var lastQuestion: String { state.messagesToUser.last { state.messages[$0]?.isOpenQuestion == true } ?? "" }
    var lastAsk: (id: String, text: String) {
        state.asks.values.filter { $0.answer == nil }.max { $0.id < $1.id }.map { ($0.id, $0.text) } ?? ("", "")
    }

    func text(of ask: String) -> String { state.asks[ask]?.text ?? "" }

    func tellAsks() {
        for ask in state.asks.values.sorted(by: { $0.at < $1.at }) {
            let answer = ask.answer.map { "\($0.word) at \($0.via.at) \($0.via.device ?? $0.via.name ?? "")" } ?? "open"
            print("ask: \(ask.id) \(ask.about.kind) by \(ask.by): \(answer): \(UntrustedText.line(ask.text, limit: 70))")
        }
        for merge in state.merges.values { print("merge: \(merge.id) \(merge.stage)") }
    }

    func tellTurns() {
        for turn in state.turns.values.sorted(by: { ($0.startedAt ?? 0) < ($1.startedAt ?? 0) }) {
            let end = turn.end.map { "\($0.outcome)" } ?? "running"
            print("turn: \(turn.id) \(turn.session) \(turn.vendor ?? "-"): \(end), \(inside[turn.id]?.items.count ?? 0) things seen")
        }
    }

    /// What was pushed and what is fetched should come to the same.
    func compare(_ id: String, with pages: [TurnPage]) {
        var fetched = TurnDetails()
        fetched.take(pages)
        print("turn: \(id) \(pages.count) pages, \(fetched.completeness), \(fetched.items.count) things")
        for item in fetched.items {
            print("  \(item.item.n) \(item.item.kind) \(item.item.status) \(item.text.end) bytes")
        }
        guard let seen = inside[id] else { return print("  not pushed") }
        print("  pushed: \(seen.completeness), \(seen.items.count) things, \(seen.items == fetched.items ? "the same" : "NOT THE SAME")")
        for (pushed, kept) in zip(seen.items, fetched.items) where pushed != kept {
            print("  pushed \(pushed)\n  kept   \(kept)")
        }
    }

    /// Everything the user said or was told.
    func tellMail() {
        for message in state.messages.values.filter({ $0.isFromUser || $0.isToUser }).sorted(by: { $0.seq < $1.seq }) {
            print(
                "mail: \(message.seq) \(message.id) [\(message.thread)] \(message.from) → \(message.to) \(message.kind)",
                "\(message.state) by \(message.device ?? "-") re \(message.replyTo ?? "-"): \(UntrustedText.line(message.body, limit: 70))")
        }
    }

    func take(_ update: LinkClient.Update) {
        switch update {
        case .status(let status):
            print("status: \(status) — through \(state.through)")
        case .reset(let start):
            print("reset: from \(start)")
            state = WorkshopState(startingAt: start)
            (records, events, unreadable, first) = (0, 0, 0, nil)
        case .records(let batch):
            for record in batch {
                do {
                    try state.apply(record)
                    if first == nil { first = record.seq }
                    records += 1
                    events += record.events.count
                    unreadable += record.events.count { if case .unreadable = $0 { true } else { false } }
                } catch {
                    print("SEQUENCE BROKEN: \(error)")
                }
            }
            print("records: +\(batch.count), \(batch.first!.seq)...\(batch.last!.through)")
        case .welcome(let welcome): print("welcome: \(welcome)")
        case .overview(let overview): print("overview: \(overview)")
        case .registeredName(let name): print("registered name: \(name ?? "none")")
        case .outbox(let items): print("outbox: \(items.count)")
        case .settled(let settled): print("settled: \(settled.result), undone \(settled.isUndone)")
        case .notice(let why): print("notice: \(why)")
        case .registrationFailed(let why): print("registration failed: \(why)")
        case .watching(let running):
            print("watching: \(running.map { "\($0)" } ?? "no")")
            for id in inside.keys where running?.contains(id) != true { inside[id]?.fellBehind() }
        case .live(let batch):
            for live in batch {
                if inside[live.turn] == nil { print("live: \(live.turn)") }
                inside[live.turn, default: TurnDetails()].take(live.what)
            }
        }
    }

    func tell() {
        print(
            "state: events \(first ?? 0)...\(state.through), \(records) records, \(events) events, \(unreadable) not known;",
            "\(state.devices.count) devices, \(state.sessions.count) sessions, \(state.cards.count) cards,",
            "\(state.messages.count) messages, \(state.threads.count) lines, \(state.asks.count) asks")
    }
}

/// Passes everything on, and keeps the lines with records and turns in a file the app can open as a recording.
final class Recorder: Connector {
    let inner: UnixSocketConnector
    let file: URL?
    /// Hang up once the next intent has gone out, before its answer is heard.
    let losesNextReply = Mutex(false)

    func connect() async throws -> any LineChannel {
        let channel = try await inner.connect()
        if let file, !FileManager.default.fileExists(atPath: file.path) { try Data().write(to: file) }
        return Channel(inner: channel, file: try file.map(FileHandle.init(forWritingTo:)), recorder: self)
    }

    init(inner: UnixSocketConnector, file: URL?) {
        self.inner = inner
        self.file = file
    }

    struct Channel: LineChannel {
        let inner: any LineChannel
        let file: FileHandle?
        let recorder: Recorder
        let isDeaf = Flag()

        func lines() -> AsyncThrowingStream<Data, any Error> {
            let (inner, file, isDeaf) = (inner, file, isDeaf)
            return AsyncThrowingStream { passed in
                let copying = Task {
                    do {
                        for try await line in inner.lines() {
                            if isDeaf.value.withLock({ $0 }) { continue }
                            if let file, [#"{"record":"#, #"{"live":"#].contains(where: { line.starts(with: Data($0.utf8)) }) {
                                try file.seekToEnd()
                                try file.write(contentsOf: line + Data("\n".utf8))
                            }
                            passed.yield(line)
                        }
                        passed.finish()
                    } catch {
                        passed.finish(throwing: error)
                    }
                }
                passed.onTermination = { _ in copying.cancel() }
            }
        }

        func send(_ line: Data) async throws {
            let isIntent = line.range(of: Data(#""do":"intent""#.utf8)) != nil
            let cuts = isIntent && recorder.losesNextReply.withLock { losing in
                defer { losing = false }
                return losing
            }
            if cuts { isDeaf.value.withLock { $0 = true } }
            try await inner.send(line)
            guard cuts else { return }
            try await Task.sleep(for: .milliseconds(500))
            print("cut: hung up without hearing the answer")
            inner.close()
        }

        func close() { inner.close() }
    }
}

final class Flag: Sendable {
    let value = Mutex(false)
}

/// Keys in private files, no prompt.
struct FileChest: KeyChest {
    let folder: URL

    func keys() throws -> (any DeviceKeys)? {
        guard let daily = try read("daily"), let approval = try read("approval") else { return nil }
        return SoftwareKeys(daily: daily, approval: approval)
    }

    func makeKeys() throws -> any DeviceKeys {
        let (daily, approval) = (P256.Signing.PrivateKey(), P256.Signing.PrivateKey())
        try PrivateFile.write(daily.rawRepresentation, to: folder.appending(path: "daily"))
        try PrivateFile.write(approval.rawRepresentation, to: folder.appending(path: "approval"))
        print("keys: made")
        return SoftwareKeys(daily: daily, approval: approval)
    }

    private func read(_ name: String) throws -> P256.Signing.PrivateKey? {
        do {
            return try .init(rawRepresentation: Data(contentsOf: folder.appending(path: name)))
        } catch CocoaError.fileReadNoSuchFile {
            return nil
        }
    }
}
