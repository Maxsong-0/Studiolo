// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation
import Testing
@testable import BottegaKit

/// Section 11: arranging lines, projects, direct lines, models, stopping a turn.
struct LineTests {
    private func state(_ events: [Event]...) throws -> WorkshopState {
        var state = WorkshopState()
        for (index, events) in events.enumerated() {
            let seq = UInt64(index + 1)
            try state.apply(Record(seq: seq, through: seq, events: events))
        }
        return state
    }

    private func letter(_ id: String, from: String, to: String, thread: String? = nil, kind: String = "task") -> Event {
        .messageQueued(
            .init(
                message: .init(
                    id: id, from: from, to: to, kind: kind, thread: thread ?? id, hop: 0, replyTo: nil, task: nil, body: id,
                    files: nil, atMs: 1, device: from == "user" ? "device-1" : nil)))
    }

    private func arranged(_ line: String, title: String? = nil, pinned: Bool? = nil, archived: Bool? = nil) -> Event {
        .lineArranged(.init(line: line, title: title, pinned: pinned, archived: archived, atMs: 1))
    }

    // MARK: As recorded

    @Test func theRecordedLines() throws {
        let state = try Fixtures.state()
        #expect(state.arrangement(of: "msg-9") == {
            var arrangement = Arrangement()
            arrangement.title = "清空缓存"
            arrangement.isPinned = true
            return arrangement
        }())
        #expect(try state.title(of: #require(state.threads["msg-9"])) == "清空缓存")
        #expect(state.arrangement(of: "dispatcher").readThrough == 120)
        #expect(state.projectList.isEmpty)

        let one = try #require(state.directLines["line-1"])
        #expect(one.project == "studio" && one.vendor == "codex" && one.role == "coder")
        #expect(one.model == .init(model: "gpt-6-luna", effort: "high"))
        #expect(one.firstText == "看看登录页为什么这么慢。" && one.firstFiles.isEmpty)
        #expect(
            one.place == .init(
                worktree: "/Users/max/lmas-lines/line-1", gitDir: "/Users/max/studio/.git/worktrees/line-1", branch: "line-1",
                base: "3f2a3f2a3f2a3f2a3f2a3f2a3f2a3f2a3f2a3f2a", at: 1764000150000))
        #expect(one.worker == .init(state: "down", run: 1, why: .init(code: "process_exited", text: "CLI 进程退出了"), at: 1764000150000))
        #expect(one.failure == nil && !one.isBeingSetUp)
        #expect(state.arrangement(of: "line-1").isArchived)
        #expect(state.vendors["line-1"] == "codex")

        let two = try #require(state.directLines["line-2"])
        #expect(two.failure?.why.code == "setup" && two.place == nil && two.worker == nil && two.model == nil && !two.isBeingSetUp)
        #expect(two.firstFiles == [.init(name: "要求.txt", content: "只看 src/。\n")])
    }

    @Test func theRecordedModels() throws {
        let state = try Fixtures.state()
        let codex = try #require(state.catalogs["codex"]?.listed)
        #expect(codex.roles == ["coder", "reviewer"] && codex.cliVersion == "0.171.0")
        #expect(codex.defaultModel == "gpt-6.2-sol" && codex.defaultEffort == "medium")
        #expect(codex.models.map(\.id) == ["gpt-6.2-sol", "gpt-6-luna"])
        #expect(codex.models[0].efforts == ["low", "medium", "high", "max"] && codex.models[0].about == nil)
        #expect(codex.models[1].name == "GPT-6 Luna" && codex.models[1].about == "Fast and cheap." && codex.models[1].defaultEffort == "low")
        guard case .unavailable(let why, _)? = state.catalogs["grok"] else { Issue.record("grok"); return }
        #expect(why.code == "not_found")
        #expect(state.catalog(for: "line-1") == codex)
        #expect(state.catalog(for: "line-2") == nil)

        #expect(state.models["line-1"]?.current == .init(model: "gpt-6.2-sol", effort: "max"))
        #expect(state.models["line-1"]?.requested == nil && state.models["line-1"]?.refusal == nil)
        #expect(state.models["coder-card-1"]?.refusal?.choice == .init(model: "gpt-6-luna", effort: "high"))
        #expect(state.models["coder-card-1"]?.refusal?.why.code == "refused")
        #expect(state.turns["turn-2"]?.stop == .failed(at: 1764000150000))
    }

    // MARK: Arranging

    @Test func whatIsNotSaidStaysAsItWas() throws {
        var state = try state([arranged("a", title: "t", pinned: true)], [arranged("a", archived: true)])
        #expect(state.arrangement(of: "a").title == "t" && state.arrangement(of: "a").isPinned && state.arrangement(of: "a").isArchived)
        try state.apply(Record(seq: 3, through: 3, events: [arranged("a", title: "", pinned: false, archived: false)]))
        #expect(state.arrangement(of: "a") == Arrangement())
    }

    @Test func readingOnlyGoesForward() throws {
        func read(_ through: UInt64) -> Event { .lineRead(.init(line: "a", through: through, atMs: 1)) }
        #expect(try state([read(5)], [read(3)]).arrangement(of: "a").readThrough == 5)
        #expect(try state([read(5)], [read(7)]).arrangement(of: "a").readThrough == 7)
        // Neither touches the other.
        let both = try state([read(5)], [arranged("a", pinned: true)], [read(6)])
        #expect(both.arrangement(of: "a").isPinned && both.arrangement(of: "a").readThrough == 6)
    }

    @Test func pinnedLinesComeFirstAndArchivedOnesLast() throws {
        let letters = (1...4).map { letter("m\($0)", from: "x", to: "y") }
        let state = try state(
            [letters[0]], [letters[1]], [letters[2]], [letters[3]], [arranged("m1", pinned: true)],
            // Archived counts for more than pinned.
            [arranged("m4", pinned: true, archived: true)])
        #expect(state.threads(of: .agents).map(\.id) == ["m1", "m3", "m2", "m4"])
    }

    @Test func aTitleIsShownAsText() throws {
        let state = try state([letter("m1", from: "x", to: "y")], [arranged("m1", title: "a\u{202E}b")])
        #expect(try state.title(of: #require(state.threads["m1"])) == UntrustedText.line("a\u{202E}b", limit: 80))
        // One of which nothing can be shown is no name.
        let blank = try self.state([letter("m1", from: "x", to: "y")], [arranged("m1", title: "\u{202E}")])
        #expect(try blank.title(of: #require(blank.threads["m1"])) == "m1")
    }

    /// The user's talk with a session is all the lines they started with it; what anyone else started is not.
    @Test func theTalkWithASession() throws {
        let state = try state(
            [letter("m1", from: "user", to: "line-1")], [letter("m2", from: "line-1", to: "user", thread: "m1", kind: "result")],
            [letter("m3", from: "x", to: "line-1")], [letter("m4", from: "user", to: "line-1")],
            [letter("m5", from: "user", to: "dispatcher")])
        #expect(state.threads(with: "line-1").map(\.id) == ["m1", "m4"])
        #expect(try state.conversation(of: #require(state.threads["m3"])) == nil)
        #expect(try state.conversation(of: #require(state.threads["m5"])) == "dispatcher")
    }

    @Test func readEitherAsALineOrAsPartOfTheTalk() throws {
        func read(_ line: String, _ through: UInt64) -> Event { .lineRead(.init(line: line, through: through, atMs: 1)) }
        let told = [[letter("m1", from: "user", to: "line-1")], [letter("m2", from: "line-1", to: "user", thread: "m1", kind: "result")]]
        func unread(_ more: Event...) throws -> Bool {
            var state = try state(told[0], told[1])
            for (index, event) in more.enumerated() {
                try state.apply(Record(seq: UInt64(3 + index), through: UInt64(3 + index), events: [event]))
            }
            return try state.hasUnread(#require(state.threads["m1"]))
        }
        #expect(try unread())
        #expect(try unread(read("m1", 1)))
        #expect(try !unread(read("m1", 2)))
        #expect(try !unread(read("line-1", 2)))
        #expect(try unread(read("dispatcher", 9)))
        #expect(try !unread(read("line-1", 1), read("m1", 2)))
        #expect(try unread(read("m1", 2), letter("m3", from: "line-1", to: "user", thread: "m1", kind: "question")))
        // What the user said themself is not news to them.
        #expect(try !unread(read("m1", 2), letter("m3", from: "user", to: "line-1", thread: "m1")))
    }

    @Test func whatTheUserSaysDoesNotCount() throws {
        let state = try state(
            [letter("m1", from: "user", to: "a")], [letter("m2", from: "a", to: "b", thread: "m1")],
            [letter("m3", from: "user", to: "b", thread: "m1")], [letter("m4", from: "b", to: "a", thread: "m1", kind: "result")])
        #expect(try state.lettersThatCount(on: #require(state.threads["m1"])) == 2)
    }

    // MARK: Projects and direct lines

    @Test func projectsComeAndGo() throws {
        func added(_ name: String) -> Event { .projectAdded(.init(project: name, path: "/\(name)", atMs: 4)) }
        let state = try state([added("b")], [added("a")], [added("c")], [.projectRemoved(.init(project: "b", atMs: 5))])
        #expect(state.projectList.map(\.name) == ["a", "c"])
        #expect(state.projects["a"] == Project(name: "a", path: "/a", addedAt: 4))
    }

    /// History may start in the middle: what is told about a line never heard of is kept.
    @Test func aLineWhoseOpeningWasNotSeen() throws {
        let up = Event.lineWorker(.init(line: "line-7", state: "up", run: 3, why: nil, atMs: 2))
        let state = try state([up])
        let line = try #require(state.directLines["line-7"])
        #expect(line.worker?.isUp == true && line.worker?.run == 3 && line.isBeingSetUp && line.vendor == nil)
    }

    @Test func theLastStartOfAnAgentCounts() throws {
        func worker(_ state: String, _ run: Int) -> Event { .lineWorker(.init(line: "line-1", state: state, run: run, why: nil, atMs: 2)) }
        let state = try state([worker("up", 1)], [worker("down", 1)], [worker("up", 2)], [worker("hibernating", 2)])
        #expect(state.directLines["line-1"]?.worker?.run == 2)
        #expect(state.directLines["line-1"]?.worker?.isUp == false)
        #expect(state.directLines["line-1"]?.worker?.state == "hibernating")
    }

    // MARK: Models

    private func asked(_ model: String, _ effort: String? = nil, _ session: String = "s") -> Event {
        .modelRequested(.init(session: session, model: model, effort: effort, atMs: 1))
    }
    private func became(_ model: String, _ effort: String? = nil) -> Event {
        .modelChanged(.init(session: "s", model: model, effort: effort, atMs: 1))
    }
    private func stayed(_ model: String, _ effort: String? = nil) -> Event {
        .modelNotChanged(.init(session: "s", model: model, effort: effort, why: .init(code: "refused", text: "x"), atMs: 1))
    }

    @Test func aChangeIsPendingUntilItIsTold() throws {
        #expect(try state([asked("a")]).models["s"]?.requested == .init(model: "a"))
        #expect(try state([asked("a")]).models["s"]?.current == nil)
        let done = try state([asked("a")], [became("a", "high")]).models["s"]
        // The effort was left to the model.
        #expect(done?.requested == nil && done?.current == .init(model: "a", effort: "high"))
        let refused = try state([asked("a", "max")], [stayed("a", "max")]).models["s"]
        #expect(refused?.requested == nil && refused?.current == nil && refused?.refusal?.choice == .init(model: "a", effort: "max"))
        // Asking again, or getting somewhere, puts the refusal behind.
        #expect(try state([stayed("a")], [asked("b")]).models["s"]?.refusal == nil)
        #expect(try state([stayed("a")], [became("b")]).models["s"]?.refusal == nil)
    }

    /// What comes of an earlier change says nothing about a later one.
    @Test func theLastOneAskedForCounts() throws {
        for outcome in [became("a"), stayed("a")] {
            #expect(try state([asked("a")], [asked("b")], [outcome]).models["s"]?.requested == .init(model: "b"))
        }
        for outcome in [became("a", "low"), stayed("a", "low")] {
            #expect(try state([asked("a", "low")], [asked("a", "high")], [outcome]).models["s"]?.requested == .init(model: "a", effort: "high"))
        }
    }

    @Test func aTurnTellsWhatItRanWith() throws {
        func turn(_ model: String?, _ effort: String?) -> Event {
            .turnStarted(.init(turn: "t", session: "s", vendor: "codex", model: model, effort: effort, cliVersion: nil, messages: nil, atMs: 1))
        }
        let told = try state([turn("a", "low")])
        #expect(told.models["s"]?.current == .init(model: "a", effort: "low"))
        #expect(told.turns["t"]?.effort == "low")
        #expect(try state([became("a")], [turn(nil, nil)]).models["s"]?.current == .init(model: "a"))
        // A turn that was already on its way does not settle what is asked for from the next one on.
        #expect(try state([asked("b")], [turn("a", nil)]).models["s"]?.requested == .init(model: "b"))
    }

    /// LMAS remembers what was asked for only for direct lines.
    @Test func aSessionThatLeavesForgetsWhatWasAskedFor() throws {
        let closed = Event.sessionClosed(.init(name: "s", atMs: 2))
        #expect(try state([asked("a")], [closed]).models["s"]?.requested == nil)
        let opened = Event.lineOpened(
            .init(line: "s", project: "p", vendor: "codex", role: "coder", model: nil, effort: nil, text: nil, files: nil, atMs: 1))
        #expect(try state([opened], [asked("a")], [closed]).models["s"]?.requested == .init(model: "a"))
    }

    @Test func theLastListCounts() throws {
        func list(_ models: String...) -> Event {
            .modelsListed(
                .init(
                    vendor: "v", roles: ["reviewer"], cliVersion: nil,
                    models: models.map { .init(model: $0, name: $0, about: nil, efforts: nil, defaultEffort: nil) },
                    defaultModel: nil, defaultEffort: nil, atMs: 1))
        }
        let gone = Event.modelsUnavailable(.init(vendor: "v", why: .init(code: "x", text: "x"), atMs: 2))
        #expect(try state([list("a")], [list("b")]).catalogs["v"]?.listed?.models.map(\.model) == ["b"])
        #expect(try state([list("a")], [gone]).catalogs["v"]?.listed == nil)
        #expect(try state([gone], [list("a")]).catalogs["v"]?.listed?.models.count == 1)
    }

    // MARK: Stopping a turn

    @Test func stoppingATurn() throws {
        let wanted = Event.turnStopRequested(.init(turn: "t", session: "s", atMs: 1))
        let failed = Event.turnNotStopped(.init(turn: "t", session: "s", atMs: 2))
        let over = Event.turnFinished(
            .init(
                turn: "t", session: "s", outcome: .init(code: "stopped_by_user", text: "x"), modelTurns: 0, tools: nil, denied: 0,
                text: nil, atMs: 3))
        #expect(try state([wanted]).turns["t"]?.stop == .requested(at: 1))
        #expect(try state([wanted], [failed]).turns["t"]?.stop == .failed(at: 2))
        #expect(try state([wanted], [failed], [wanted]).turns["t"]?.stop == .requested(at: 1))
        let turn = try state([wanted], [over]).turns["t"]
        #expect(turn?.isRunning == false && turn?.end?.outcome.code == "stopped_by_user")
    }

    /// Too new for the one before, too little for this one: each costs only itself.
    @Test(arguments: [
        #"{"type":"line_arranged","at_ms":1}"#, #"{"type":"line_worker","line":"l","state":"up","at_ms":1}"#,
        #"{"type":"models_listed","vendor":"v","roles":[],"models":[{"name":"x"}],"at_ms":1}"#,
        #"{"type":"line_read","line":"l","through":-1,"at_ms":1}"#,
        #"{"type":"line_ready","line":"l","worktree":"/w","branch":"b","base":"c","at_ms":1}"#,
    ])
    func oneThatCannotBeReadCostsOnlyItself(event: String) throws {
        let line = #"{"record":{"seq":1,"through":2,"events":[\#(event),{"type":"project_removed","project":"p","at_ms":1}]}}"#
        guard case .record(let record) = try ServerLine.read(Data(line.utf8)) else { Issue.record("no record"); return }
        guard case .unreadable = record.events[0] else { Issue.record("read \(event)"); return }
        guard case .projectRemoved = record.events[1] else { Issue.record("lost the one beside it"); return }
    }

    @Test func cardsTellTheirGitDirectoryWhenLMASDoes() throws {
        func started(_ more: String) throws -> Card.Work? {
            let line = #"{"record":{"seq":1,"through":1,"events":[{"type":"card_started","card":"c","worker":"w","worktree":"/w","branch":"b","base":"x","task":"m"\#(more),"at_ms":1}]}}"#
            guard case .record(let record) = try ServerLine.read(Data(line.utf8)) else { return nil }
            var state = WorkshopState()
            try state.apply(record)
            return state.cards["c"]?.work
        }
        #expect(try started(#","git_dir":"/g""#)?.gitDir == "/g")
        #expect(try #require(try started("")).gitDir == nil)
    }
}
