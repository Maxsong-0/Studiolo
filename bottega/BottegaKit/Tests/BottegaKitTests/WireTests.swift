// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation
import Testing
@testable import BottegaKit

struct WireTests {
    /// Every line of `events.jsonl` is a record this code reads in full, and each follows on from the last.
    @Test func everyRecordedEventIsRead() throws {
        let records = try Fixtures.records()
        #expect(records.count == 46)
        var through: UInt64 = 0
        for record in records {
            #expect(record.seq == through + 1)
            #expect(record.through >= record.seq)
            through = record.through
            for event in record.events {
                if case .unreadable(let type, _) = event { Issue.record("did not read \(type) in record \(record.seq)") }
                #expect(event.at != nil)
            }
        }
        #expect(through == 55)
    }

    /// Every `type` in the document is in the recording, and read as its own case.
    @Test func everyTypeIsCovered() throws {
        struct Raw: Decodable { struct Record: Decodable { var events: [Event] }; struct Event: Decodable { var type: String }; var record: Record }
        let types = Set(try Fixtures.lines("events.jsonl").flatMap { try JSONDecoder().decode(Raw.self, from: $0).record.events.map(\.type) })
        #expect(types == [
            "session_registered", "session_closed", "message_queued", "message_handed", "message_failed", "card_created",
            "card_started", "card_done", "card_failed", "limit_tripped", "tests_submitted", "tests_handed_over",
            "tests_accepted", "tests_finished", "review_started", "review_finished", "review_not_done", "merge_requested",
            "merge_checked", "merge_prepared", "merge_restarted", "merge_landing", "merge_finished", "merge_failed",
            "merge_cleaned", "ask_opened", "ask_answered", "device_registered", "intent_echoed",
        ])
    }

    @Test func fieldsComeThrough() throws {
        let events = try Fixtures.records().flatMap(\.events)
        guard case .cardStarted(let started)? = events.first(where: { if case .cardStarted(let e) = $0 { e.card == "card-2" } else { false } }) else {
            Issue.record("no card-2"); return
        }
        #expect(started.freshStart == .init(target: "main", conflicts: 1, named: ["src/settings.rs"]))
        let finished = events.compactMap { if case .testsFinished(let e) = $0 { e } else { nil } }
        #expect(finished.first?.report?.exitCode == 101)
        #expect(finished.first?.report?.logBytes == 4096)
        #expect(finished.first?.outcome.code == "failed")
        let requested = events.compactMap { if case .testsSubmitted(let e) = $0 { e } else { nil } }
        #expect(requested.map(\.request.memoryGib) == [4, 4])
        #expect(requested.last?.merge == "merge-1")
        let echoes = events.compactMap { if case .intentEchoed(let e) = $0 { e } else { nil } }
        #expect(echoes.map(\.outcome) == [
            .done(message: "msg-9"), .refused(.init(code: "not_on_thread", text: "coder-card-1 不在这条线上")),
            .already(number: 1),
            .refused(.init(code: "already_answered", text: "ask-1 已经答过了（approved），一张只答一次")),
            .refused(.init(code: "already_answered", text: "ask-2 已经答过了（withdrawn），一张只答一次")),
            .done(message: nil), .done(message: nil),
        ])
        let answered = events.compactMap { if case .askAnswered(let e) = $0 { e.via } else { nil } }
        #expect(answered == [.init(at: "dialog", name: nil, device: nil), .init(at: "session", name: "dispatcher", device: nil), .init(at: "device", name: nil, device: "device-1")])
    }

    // MARK: Growing without breaking

    private func record(_ json: String) throws -> Record {
        guard case .record(let record) = try ServerLine.read(Data(json.utf8)) else { throw Fixtures.NotARecord(line: json) }
        return record
    }

    @Test func unknownTypesFieldsAndCodesAreTaken() throws {
        let record = try record(
            """
            {"record":{"seq":5,"through":9,"later":true,"events":[
              {"type":"turn_started","session":"coder","text":"开始干活"},
              {"type":"card_failed","card":"card-9","why":{"code":"struck_by_lightning","text":"被雷劈了","more":[1]},"at_ms":1,"extra":{}},
              {"type":"session_registered","name":"x","role":"astronaut","at_ms":2},
              {"type":"ask_opened","ask":"a","about":{"kind":"deploy","where":"169"},"by":"d","text":"行不行","at_ms":3},
              {"type":"ask_answered","ask":"a","answer":"shelved","via":{"at":"watch","device":"w"},"at_ms":4},
              {"type":"intent_echoed","device":"d","number":1,"digest":"x","content_id":"y","outcome":{"result":"postponed"},"at_ms":5}
            ]}}
            """)
        #expect(record.events.count == 6)
        #expect(record.events[0] == .unreadable(type: "turn_started", text: "开始干活"))
        guard case .cardFailed(let failed) = record.events[1] else { Issue.record("card_failed"); return }
        #expect(failed.why == .init(code: "struck_by_lightning", text: "被雷劈了"))
        guard case .intentEchoed(let echo) = record.events[5] else { Issue.record("echo"); return }
        #expect(echo.outcome == .other("postponed"))
        var state = WorkshopState(startingAt: 5)
        try state.apply(record)
        #expect(state.through == 9)
        #expect(state.log.count == 6)
        #expect(state.dispatcherRows.contains { $0.kind == .unreadable(type: "turn_started", text: "开始干活") })
        #expect(state.asks["a"]?.answer?.word == "shelved")
    }

    /// A known type in a shape it cannot read, or an event that is not even an object, costs only itself.
    @Test(arguments: [
        #"{"type":"card_done","card":"card-1"}"#, #"{"type":"card_done","card":7,"result":"m","at_ms":1}"#,
        #"{"type":"card_done","card":"c","result":"m","at_ms":-1}"#, #"{"type":7}"#, #"{}"#, #"[]"#, #""x""#, "null", "3",
    ])
    func oneUnreadableEventDoesNotCostTheOthers(event: String) throws {
        let record = try record(#"{"record":{"seq":1,"through":2,"events":[\#(event),{"type":"session_closed","name":"x","at_ms":1}]}}"#)
        #expect(record.events.count == 2)
        guard case .unreadable = record.events[0] else { Issue.record("read \(event) as \(record.events[0])"); return }
        guard case .sessionClosed = record.events[1] else { Issue.record("lost the one beside it"); return }
    }

    @Test func linesThatAreNotOursAreMalformedOrUnknown() throws {
        for bad in ["", "x", "[]", "3", "null", #"{"record":{}}"#, #"{"record":{"seq":"1","through":1,"events":[]}}"#,
                    #"{"lmas":"control","version":1,"connection":1,"nonce":"\#(String(repeating: "ab", count: 32))"}"#,
                    #"{"lmas":"ui","version":1,"connection":1,"nonce":"abab"}"#] {
            #expect(throws: ServerLine.Malformed.self, "\(bad)") { try ServerLine.read(Data(bad.utf8)) }
        }
        #expect(try ServerLine.read(Data(#"{"news":1}"#.utf8)) == .unknown)
    }

    // MARK: A whole connection

    /// `session.jsonl`: what LMAS says is read, and what Bottega says is what this code writes.
    @Test func aRecordedConnection() throws {
        struct Turn: Decodable { var from: String; var line: JSON }
        let turns = try Fixtures.lines("session.jsonl").map { try JSONDecoder().decode(Turn.self, from: $0) }
        #expect(turns.count == 26)
        let vectors = try Fixtures.vectors()
        var nonce = Data(), connection: UInt64 = 0
        for turn in turns {
            let line = try JSONEncoder().encode(turn.line)
            if turn.from == "lmas" {
                let read = try ServerLine.read(line)
                #expect(read != .unknown, "\(String(decoding: line, as: UTF8.self))")
                if case .greeting(let greeting) = read { (nonce, connection) = (greeting.nonce, greeting.connection) }
                continue
            }
            let mine: Data
            switch turn.line["do"]?.string ?? turn.line["hello"]?.string {
            case "bottega-unregistered": mine = try ClientLine.helloUnregistered()
            case "bottega":
                let signature = try #require(turn.line["signature"]?.string)
                #expect(try vectors.daily.verifies(.handshake(nonce: nonce, connection: connection), signature: signature))
                mine = try ClientLine.hello(device: try #require(turn.line["device"]?.string), signature: signature)
            case let verb:
                let id = try #require(turn.line["id"]?.number)
                let request: Request? = switch verb {
                case "register": .register(vectors.register.registration)
                case "overview": .overview
                case "events": .events(from: 1, limit: 2)
                case "follow": .follow(from: 3)
                case "seen": .seen(through: 5)
                case "ping": .ping
                case "intent": .intent(vectors.intents[id == 4 ? 0 : 3].intent)
                default: nil
                }
                mine = try ClientLine.request(id: id, try #require(request, "\(verb ?? "")"))
            }
            #expect(mine.last == UInt8(ascii: "\n"))
            #expect(try JSONDecoder().decode(JSON.self, from: mine) == turn.line)
        }
    }

    @Test func answersAreReadAsWhatWasAsked() throws {
        func reply(_ json: String) throws -> Reply {
            guard case .reply(let reply) = try ServerLine.read(Data(json.utf8)) else { throw ServerLine.Malformed() }
            return reply
        }
        let overview: Overview = try reply(
            #"{"id":1,"ok":{"cards":{"merge_into":"main","repo":"/r","reviewers":["claude"],"tests":true,"vendors":["codex"]},"dropped_intents":2,"latest":3,"lmas":"0.0.0","now_ms":4,"oldest":1}}"#
        ).read()
        #expect(overview.cards?.mergeInto == "main")
        #expect(overview.oldest == 1)
        #expect(overview.droppedIntents == 2)
        let bare: Overview = try reply(#"{"id":1,"ok":{"latest":0,"lmas":"1","now_ms":4,"oldest":1}}"#).read()
        #expect(bare.cards == nil)
        #expect(bare.droppedIntents == nil)
        let echo: IntentReply = try reply(
            #"{"id":4,"ok":{"repeated":true,"seq":4,"event":{"type":"intent_echoed","device":"d","number":1,"digest":"x","content_id":"y","outcome":{"result":"done","message":"msg-7"},"at_ms":5}}}"#
        ).read()
        #expect(echo.repeated == true)
        #expect(echo.echo?.outcome == .done(message: "msg-7"))
        #expect(throws: (any Error).self) { let _: Overview = try reply(#"{"id":1,"ok":{}}"#).read() }
    }

    @Test func errorsCarryTheirNumbers() throws {
        func error(_ json: String) throws -> (UInt64?, WireError) {
            guard case .error(let id, let error) = try ServerLine.read(Data(json.utf8)) else { throw ServerLine.Malformed() }
            return (id, error)
        }
        let gap = try error(#"{"error":{"code":"gap","next":2,"text":"序号跳了"},"id":7}"#)
        #expect(gap.0 == 7 && gap.1.next == 2)
        #expect(try error(#"{"error":{"code":"gone","oldest":9,"text":"x"},"id":1}"#).1.oldest == 9)
        #expect(try error(#"{"error":{"code":"ahead","latest":4,"text":"x"},"id":1}"#).1.latest == 4)
        #expect(try error(#"{"error":{"code":"stopping","text":"LMAS 正在停"}}"#).0 == nil)
    }

    @Test func aLineLMASWouldDisconnectOnIsNeverWritten() throws {
        var intent = try #require(try Fixtures.vectors().intents.first).intent
        intent.body = String(repeating: "x", count: Wire.maxLineBytes)
        #expect(throws: ClientLine.TooLong.self) { try ClientLine.request(id: 1, .intent(intent)) }
        // The newline does not count: the longest line LMAS takes goes through, one byte more does not.
        intent.body = ""
        let rest = try ClientLine.request(id: 1, .intent(intent)).count - 1
        intent.body = String(repeating: "x", count: Wire.maxLineBytes - rest)
        #expect(try ClientLine.request(id: 1, .intent(intent)).count == Wire.maxLineBytes + 1)
        intent.body += "x"
        #expect(throws: ClientLine.TooLong.self) { try ClientLine.request(id: 1, .intent(intent)) }
    }

    @Test func namesAreCutToWhatLMASTakes() {
        #expect(Registration.usableName("书房的 Mac Studio") == "书房的 Mac Studio")
        #expect(Registration.usableName("  a\n b\u{7} ") == "a b")
        #expect(Registration.usableName(" \n") == nil)
        let long = Registration.usableName(String(repeating: "字", count: 40))
        #expect(long == String(repeating: "字", count: 21))
    }
}

/// JSON compared as values, whatever order its keys are in.
indirect enum JSON: Codable, Equatable {
    case object([String: JSON]), array([JSON]), string(String), number(UInt64), bool(Bool), null

    init(from decoder: any Decoder) throws {
        let value = try decoder.singleValueContainer()
        if let object = try? value.decode([String: JSON].self) { self = .object(object) }
        else if let array = try? value.decode([JSON].self) { self = .array(array) }
        else if let string = try? value.decode(String.self) { self = .string(string) }
        else if let bool = try? value.decode(Bool.self) { self = .bool(bool) }
        else if let number = try? value.decode(UInt64.self) { self = .number(number) }
        else { self = .null }
    }

    func encode(to encoder: any Encoder) throws {
        var value = encoder.singleValueContainer()
        switch self {
        case .object(let object): try value.encode(object)
        case .array(let array): try value.encode(array)
        case .string(let string): try value.encode(string)
        case .number(let number): try value.encode(number)
        case .bool(let bool): try value.encode(bool)
        case .null: try value.encodeNil()
        }
    }

    subscript(key: String) -> JSON? { if case .object(let object) = self { object[key] } else { nil } }
    var string: String? { if case .string(let string) = self { string } else { nil } }
    var number: UInt64? { if case .number(let number) = self { number } else { nil } }
}
