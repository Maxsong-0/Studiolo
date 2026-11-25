// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import BottegaKit
import BottegaLink
import CryptoKit
import Foundation

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
        let client = LinkClient(
            connector: Recorder(
                inner: UnixSocketConnector(path: run.appending(path: "ui.sock").path),
                file: arguments.count > 3 ? URL(filePath: arguments[3]) : nil),
            chest: FileChest(folder: state), store: FileKeepsakeStore(url: state.appending(path: "link.json")))
        let ledger = Ledger()
        let running = Task { await client.run() }
        let watching = Task { for await update in client.updates { await ledger.take(update) } }

        for try await line in FileHandle.standardInput.bytes.lines {
            let words = line.split(separator: " ", maxSplits: 1).map(String.init)
            let rest = words.count > 1 ? words[1] : ""
            do {
                switch words.first {
                case "register": await client.register(name: rest)
                case "forget": try await client.forgetDevice()
                case "reconnect": await client.reconnect()
                case "reload": await client.reload()
                case "seen":
                    await client.seen(through: ledger.through)
                    print("seen: asked")
                case "say":
                    try await client.submit(.say(to: "dispatcher", thread: nil, text: rest, files: [], wantsReply: true))
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
    var through: UInt64 { state.through }

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
        }
    }

    func tell() {
        print(
            "state: events \(first ?? 0)...\(state.through), \(records) records, \(events) events, \(unreadable) not known;",
            "\(state.devices.count) devices, \(state.sessions.count) sessions, \(state.cards.count) cards,",
            "\(state.messages.count) messages, \(state.threads.count) lines, \(state.asks.count) asks")
    }
}

/// Passes everything on, and keeps the lines with records in a file the app can open as a recording.
struct Recorder: Connector {
    let inner: UnixSocketConnector
    let file: URL?

    func connect() async throws -> any LineChannel {
        let channel = try await inner.connect()
        guard let file else { return channel }
        if !FileManager.default.fileExists(atPath: file.path) { try Data().write(to: file) }
        return Channel(inner: channel, file: try FileHandle(forWritingTo: file))
    }

    struct Channel: LineChannel {
        let inner: any LineChannel
        let file: FileHandle

        func lines() -> AsyncThrowingStream<Data, any Error> {
            let (inner, file) = (inner, file)
            return AsyncThrowingStream { passed in
                let copying = Task {
                    do {
                        for try await line in inner.lines() {
                            if line.starts(with: Data(#"{"record":"#.utf8)) {
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

        func send(_ line: Data) async throws { try await inner.send(line) }
        func close() { inner.close() }
    }
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
