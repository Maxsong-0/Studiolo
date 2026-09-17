import AppKit
import Testing
@testable import DroppyCode

@MainActor private func waitUntil(_ condition: () -> Bool) async throws {
    let deadline = ContinuousClock.now + .seconds(4)
    while !condition(), ContinuousClock.now < deadline {
        try await Task.sleep(for: .milliseconds(5))
    }
    #expect(condition())
}

@Test @MainActor func duplicateCompletionFinalizesOnce() async throws {
    let app = AppModel()
    let runtime = ThreadRuntime(threadID: UUID(), app: app)
    runtime.rehearseTurn("First")
    let held = AsyncStream<Void>.makeStream()
    runtime.commandSettles["held"] = Task { for await _ in held.stream {} }
    runtime.rehearse(.turnCompleted(status: .completed, error: nil))
    runtime.rehearse(.exited(error: nil))
    try await Task.sleep(for: .milliseconds(20))
    #expect(runtime.isRunning)
    held.continuation.finish()
    try await waitUntil { !runtime.isRunning }
    try await Task.sleep(for: .milliseconds(20))
    #expect(runtime.entries.filter { $0.kind == .turnEnd }.count == 1)
}

@Test @MainActor func queuedCompletionCannotFinishReplacementTurn() async throws {
    let app = AppModel()
    let runtime = ThreadRuntime(threadID: UUID(), app: app)
    runtime.rehearseTurn("First")
    runtime.rehearse(.turnCompleted(status: .completed, error: nil))
    runtime.rehearseTurn("Replacement")
    let replacementID = runtime.turns.last?.id
    try await Task.sleep(for: .milliseconds(20))
    #expect(runtime.isRunning)
    #expect(runtime.currentTurnID == replacementID)
    #expect(runtime.entries.allSatisfy { $0.kind != .turnEnd })
}

@Test @MainActor func replacedAndStoppedSessionsCannotEmitEvents() {
    let app = AppModel()
    let runtime = ThreadRuntime(threadID: UUID(), app: app)
    let configuration = SessionConfiguration(provider: .deepseek, executable: nil, workingDirectory: FileManager.default.temporaryDirectory, environment: [:], resumeID: nil, resumeAt: nil, model: nil, effort: nil, runtimeMode: .fullAccess, interactionMode: .build, apiKey: "test")
    let first = DeepSeekSession(configuration: configuration)
    runtime.observe(first)
    let stale = first.onEvent
    let second = DeepSeekSession(configuration: configuration)
    runtime.observe(second)
    stale?(.messageCompleted(id: "stale", text: "Old session"))
    second.onEvent?(.messageCompleted(id: "current", text: "Current session"))
    #expect(runtime.entries.map(\.id) == ["current"])
    let stopped = second.onEvent
    runtime.stopSession()
    stopped?(.messageCompleted(id: "stopped", text: "Stopped session"))
    #expect(runtime.entries.map(\.id) == ["current"])
}

@Test @MainActor func queuedInterruptCannotStopReplacementTurn() async throws {
    let app = AppModel()
    let runtime = ThreadRuntime(threadID: UUID(), app: app)
    let configuration = SessionConfiguration(provider: .deepseek, executable: nil, workingDirectory: FileManager.default.temporaryDirectory, environment: [:], resumeID: nil, resumeAt: nil, model: nil, effort: nil, runtimeMode: .fullAccess, interactionMode: .build, apiKey: "test")
    let provider = DeepSeekSession(configuration: configuration)
    runtime.session = provider
    runtime.rehearseTurn("First")
    runtime.interrupt()
    runtime.rehearseTurn("Replacement")
    try await Task.sleep(for: .milliseconds(20))
    #expect(!provider.interrupted)
    runtime.interruptWatchdog?.cancel()
}

@Test @MainActor func archivedRuntimeCanBeRestoredAndRestarted() async throws {
    let app = AppModel()
    app.settings.autoContinueAfterLimit = false
    let project = app.addProject(at: FileManager.default.temporaryDirectory)
    let thread = try #require(app.newThread(in: project, workspace: .local))
    let runtime = app.runtime(for: thread.id)
    runtime.rehearseTurn("Before archive")
    app.archive(thread.id)
    try await waitUntil { !runtime.isRunning }
    #expect(!runtime.isRunning)
    #expect(runtime.entries.filter { $0.kind == .turnEnd }.count == 1)
    app.unarchive(thread.id)
    runtime.rehearseTurn("After restore")
    #expect(runtime.isRunning)
    runtime.rehearse(.turnCompleted(status: .completed, error: nil))
    try await waitUntil { !runtime.isRunning }
    #expect(runtime.entries.filter { $0.kind == .turnEnd }.count == 2)
}

@Test @MainActor func scheduledPromptSurvivesAnotherTurnStartingFirst() async {
    let app = AppModel()
    let runtime = ThreadRuntime(threadID: UUID(), app: app)
    runtime.rehearseTurn("Winner")
    let winningTurnID = runtime.currentTurnID
    await runtime.startTurn(text: "Still send this", attachments: [])
    #expect(runtime.currentTurnID == winningTurnID)
    #expect(runtime.followUps.map(\.text) == ["Still send this"])
}

@Test @MainActor func stoppedArchivedRuntimeKeepsPendingPromptWithoutRestarting() async throws {
    let app = AppModel()
    app.settings.autoContinueAfterLimit = false
    let project = app.addProject(at: FileManager.default.temporaryDirectory)
    let thread = try #require(app.newThread(in: project, workspace: .local))
    let runtime = ThreadRuntime(threadID: thread.id, app: app)
    runtime.rehearseTurn("Before archive")
    runtime.pendingSend = .init(text: "Keep this prompt", attachments: [])
    app.updateThread(thread.id) { $0.isArchived = true }
    runtime.stopSession()
    try await waitUntil { !runtime.isRunning }
    try await Task.sleep(for: .milliseconds(20))
    #expect(!runtime.isRunning)
    #expect(runtime.pendingSend?.text == "Keep this prompt")
    #expect(runtime.turns.count == 1)
}

@Test @MainActor func sessionReplacementDoesNotFinalizeStartingTurn() async throws {
    let app = AppModel()
    let runtime = ThreadRuntime(threadID: UUID(), app: app)
    runtime.rehearseTurn("Starting")
    let startingTurnID = runtime.currentTurnID
    runtime.releaseSession(stop: true)
    try await Task.sleep(for: .milliseconds(20))
    #expect(runtime.isRunning)
    #expect(runtime.currentTurnID == startingTurnID)
    #expect(runtime.entries.allSatisfy { $0.kind != .turnEnd })
    runtime.stopSession()
    try await waitUntil { !runtime.isRunning }
    #expect(runtime.entries.filter { $0.kind == .turnEnd }.count == 1)
}

@Test func backgroundCommandDiffPreservesSectionsAndStatistics() async throws {
    let patch = "diff --git a/one.txt b/one.txt\n--- a/one.txt\n+++ b/one.txt\n@@ -1 +1 @@\n-old\n+new\ndiff --git a/two.txt b/two.txt\nnew file mode 100644\n--- /dev/null\n+++ b/two.txt\n@@ -0,0 +1 @@\n+added\n"
    let edits = await ThreadRuntime.fileEdits(from: patch, repositoryRoot: URL(fileURLWithPath: "/repo", isDirectory: true))
    #expect(edits.map(\.path) == ["/repo/one.txt", "/repo/two.txt"])
    #expect(edits.map(\.additions) == [1, 1])
    #expect(edits.map(\.deletions) == [1, 0])
    #expect(edits.first?.diff?.contains("a/two.txt") == false)
    #expect(edits.last?.diff?.contains("+added") == true)
}

@Test @MainActor func archivedBusyThreadKeepsQueuedTextWithoutSpawningHead() {
    let app = AppModel()
    app.settings.hydraEnabled = true
    app.settings.hydraQueueHeads = true
    app.settings.hydraIsolateHeads = false
    let project = app.addProject(at: FileManager.default.temporaryDirectory)
    guard let thread = app.newThread(in: project, workspace: .local) else {
        Issue.record("Could not create fixture thread")
        return
    }
    let runtime = ThreadRuntime(threadID: thread.id, app: app)
    runtime.rehearseTurn("Finishing archived turn")
    app.updateThread(thread.id) { $0.isArchived = true }
    runtime.enqueueFollowUp(text: "Keep the queued instruction", attachments: [])
    let heads = app.hydraHeads(of: thread.id)
    for head in heads { app.updateHydraHead(head.id) { $0.status = .stopped } }
    #expect(heads.isEmpty)
    #expect(runtime.followUps.map(\.text) == ["Keep the queued instruction"])
    runtime.cancelPersistence()
}

@Test @MainActor func commandEditsInNestedProjectRemainInTurnSummary() async throws {
    let fixture = try NativeFileFixture()
    defer { fixture.remove() }
    let nested = fixture.directory.appendingPathComponent("nested", isDirectory: true)
    try FileManager.default.createDirectory(at: nested, withIntermediateDirectories: true)
    let git = Git(fixture.directory.path)
    _ = try await git.output(["init", "-q"])
    try "old\n".write(to: nested.appendingPathComponent("file.txt"), atomically: true, encoding: .utf8)
    try "old sibling\n".write(to: fixture.directory.appendingPathComponent("sibling.txt"), atomically: true, encoding: .utf8)
    _ = try await git.output(["add", "."])
    let app = AppModel()
    let project = app.addProject(at: nested)
    let thread = try #require(app.newThread(in: project, workspace: .local))
    let runtime = ThreadRuntime(threadID: thread.id, app: app)
    defer { runtime.cancelPersistence() }
    runtime.rehearseTurn("Edit nested file")
    let turnID = try #require(runtime.currentTurnID)
    let checkpoint = Git.checkpointRef(thread: thread.id, turn: 0, phase: "start")
    try await git.captureCheckpoint(checkpoint)
    runtime.updateTurn(turnID) { $0.baseCheckpoint = checkpoint }
    runtime.rehearse(.toolStarted(id: "nested-command", call: ToolCall(kind: .command, title: "Change file")))
    let baseline = try #require(runtime.commandTrees["nested-command"])
    _ = await baseline.value
    try "new\n".write(to: nested.appendingPathComponent("file.txt"), atomically: true, encoding: .utf8)
    try "new sibling\n".write(to: fixture.directory.appendingPathComponent("sibling.txt"), atomically: true, encoding: .utf8)
    _ = try await git.output(["add", "sibling.txt"])
    runtime.rehearse(.toolUpdated(id: "nested-command", update: ToolUpdate(status: .completed)))
    runtime.rehearse(.turnCompleted(status: .completed, error: nil))
    try await waitUntil { !runtime.isRunning }
    // The command's edit inside the project counts, repository-relative as the diff names
    // it; the sibling it wrote above the project is no part of the thread's work.
    let files = await runtime.parsedDiff(selection: turnID)
    #expect(files.map(\.path) == ["nested/file.txt"])
    #expect(files.first?.additions == 1)
    #expect(files.first?.deletions == 1)
    let summaryEntry = try #require(runtime.entries.first { $0.kind == .turnEnd })
    guard case .turnEnd(let summary) = summaryEntry.item.content else {
        Issue.record("Missing turn summary")
        return
    }
    #expect(summary.changes?.files.map(\.path) == ["nested/file.txt"])
}

@Test @MainActor func repositoryRootLookupCoalescesOnlyWhileRunning() async throws {
    let fixture = try NativeFileFixture()
    defer { fixture.remove() }
    let nested = fixture.directory.appendingPathComponent("nested", isDirectory: true)
    try FileManager.default.createDirectory(at: nested, withIntermediateDirectories: true)
    _ = try await Git(fixture.directory.path).output(["init", "-q"])
    let app = AppModel()
    let runtime = ThreadRuntime(threadID: UUID(), app: app)
    defer { runtime.cancelPersistence() }
    let git = Git(nested.path)
    let first = runtime.repositoryRoot(for: git)
    let lookupID = try #require(runtime.repositoryRootLookup?.id)
    let second = runtime.repositoryRoot(for: git)
    #expect(runtime.repositoryRootLookup?.id == lookupID)
    #expect(await first.value.path == fixture.directory.path)
    #expect(await second.value.path == fixture.directory.path)
    #expect(runtime.repositoryRootLookup == nil)
    _ = try await git.output(["init", "-q"])
    #expect(await runtime.repositoryRoot(for: git).value.path == nested.path)
    #expect(runtime.repositoryRootLookup == nil)
}

@Test @MainActor func canceledWaitCannotEraseReplacement() async throws {
    let app = AppModel()
    app.settings.autoContinueAfterLimit = true
    let project = app.addProject(at: FileManager.default.temporaryDirectory)
    let thread = try #require(app.newThread(in: project, workspace: .local))
    let waiting = AutoContinue(app: app)
    waiting.noteLimit(thread.id, resetsAt: .now.addingTimeInterval(3600))
    #expect(waiting.turnFinished(thread.id, status: .failed, continues: false))
    try await waitUntil { waiting.resumesAt[thread.id] != nil }
    waiting.cancel(thread.id)
    waiting.noteLimit(thread.id, resetsAt: .now.addingTimeInterval(7200))
    #expect(waiting.turnFinished(thread.id, status: .failed, continues: false))
    try await Task.sleep(for: .milliseconds(20))
    #expect(waiting.waits[thread.id] != nil)
    #expect(waiting.resumesAt[thread.id] != nil)
    waiting.cancel(thread.id)
}

private final class ToolStreamProtocol: URLProtocol, @unchecked Sendable {
    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() {
        let response = HTTPURLResponse(url: request.url!, statusCode: 200, httpVersion: "HTTP/1.1", headerFields: ["Content-Type": "text/event-stream"])!
        let event = "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"test-tool\",\"function\":{\"name\":\"run_command\",\"arguments\":\"{\\\"command\\\":\\\"sleep 2\\\"}\"}}]}}]}\n\ndata: [DONE]\n\n"
        client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: Data(event.utf8))
        client?.urlProtocolDidFinishLoading(self)
    }
    override func stopLoading() {}
}

@Test @MainActor func nativeInterruptCancelsActiveTool() async throws {
    #expect(URLProtocol.registerClass(ToolStreamProtocol.self))
    defer { URLProtocol.unregisterClass(ToolStreamProtocol.self) }
    for provider in [ProviderKind.deepseek, .meta] {
        for supervised in [false, true] {
            let mode: RuntimeMode = supervised ? .supervised : .fullAccess
            let configuration = SessionConfiguration(provider: provider, executable: nil, workingDirectory: FileManager.default.temporaryDirectory, environment: ProcessInfo.processInfo.environment, resumeID: nil, resumeAt: nil, model: nil, effort: nil, runtimeMode: mode, interactionMode: .build, apiKey: "test")
            let session: any ProviderSession = provider == .deepseek ? DeepSeekSession(configuration: configuration) : MetaSession(configuration: configuration)
            var toolStarted = false
            var awaitingApproval = false
            var completions = 0
            session.onEvent = { event in
                if case .toolStarted = event { toolStarted = true }
                if case .approval = event { awaitingApproval = true }
                if case .turnCompleted(let status, _) = event {
                    #expect(status == .interrupted)
                    completions += 1
                }
            }
            _ = try await session.start()
            let task = Task { try await session.send(TurnInput(text: "Run the fixture", images: [], model: nil, effort: nil, runtimeMode: mode, interactionMode: .build)) }
            try await waitUntil { supervised ? awaitingApproval : toolStarted }
            try await Task.sleep(for: .milliseconds(50))
            let start = ContinuousClock.now
            if supervised { task.cancel() } else { await session.interrupt() }
            try await task.value
            #expect(start.duration(to: .now) < .seconds(1))
            #expect(completions == 1)
            session.stop()
        }
    }
}

@Test @MainActor func canceledNativeFileToolsCannotMutateFiles() async throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    for provider in [ProviderKind.deepseek, .meta] {
        let configuration = SessionConfiguration(provider: provider, executable: nil, workingDirectory: root, environment: [:], resumeID: nil, resumeAt: nil, model: nil, effort: nil, runtimeMode: .fullAccess, interactionMode: .build, apiKey: "test")
        let deepseek = DeepSeekSession(configuration: configuration)
        let meta = MetaSession(configuration: configuration)
        let path = root.appendingPathComponent(provider.rawValue + ".txt")
        let arguments = String(data: try JSONSerialization.data(withJSONObject: ["path": path.lastPathComponent, "content": "canceled write"]), encoding: .utf8)!
        let task = Task {
            if provider == .deepseek {
                return await deepseek.executeTool(.init(id: "file-tool", name: "write_file", arguments: arguments))
            }
            return await meta.executeTool(.init(id: "file-tool", name: "write_file", arguments: arguments))
        }
        task.cancel()
        let result = await task.value
        #expect(result.hasPrefix("Error:"))
        #expect(!FileManager.default.fileExists(atPath: path.path))
    }
}

private struct NativeFileFixture {
    let directory = URL(fileURLWithPath: FileManager.default.currentDirectoryPath).appendingPathComponent("build.noindex/file-tests-" + UUID().uuidString, isDirectory: true)
    var tools: NativeFileTools { NativeFileTools(workingDirectory: directory.path) }

    init() throws {
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    }

    func remove() {
        try? FileManager.default.removeItem(at: directory)
    }
}

@Test func boundedReadRejectsOversizedSparseFiles() async throws {
    let fixture = try NativeFileFixture()
    defer { fixture.remove() }
    let url = fixture.directory.appendingPathComponent("sparse.txt")
    #expect(FileManager.default.createFile(atPath: url.path, contents: nil))
    let handle = try FileHandle(forWritingTo: url)
    try handle.truncate(atOffset: 4_000_000_000)
    try handle.close()
    do {
        _ = try await fixture.tools.readFile("sparse.txt")
        Issue.record("An oversized file was accepted")
    } catch {
        #expect(error.localizedDescription == "That file is too large to read (4000000000 bytes).")
    }
}

@Test func nativeReadPreservesSlicesEncodingAndLimits() async throws {
    let fixture = try NativeFileFixture()
    defer { fixture.remove() }
    let tools = fixture.tools
    try await tools.writeFile(path: "lines.txt", content: "first\n\nthird\n")
    #expect(try await tools.readFile("lines.txt", startLine: 2, lineCount: 2) == "(lines 2-3 of 4)\n\nthird")
    #expect(try await tools.readFile("lines.txt", startLine: 3, lineCount: Int.max) == "(lines 3-4 of 4)\nthird\n")
    #expect(try await tools.readFile("lines.txt", startLine: 5) == "(the file has 4 lines; start_line 5 is past the end)")
    #expect(try await tools.readFile("lines.txt", startLine: -1, lineCount: 0) == "(lines 1-1 of 4)\nfirst")
    try Data([0x63, 0x61, 0x66, 0xe9]).write(to: fixture.directory.appendingPathComponent("latin.txt"))
    #expect(try await tools.readFile("latin.txt") == "café")
    try await tools.writeFile(path: "limit.txt", content: String(repeating: "a", count: 1_000_000))
    #expect(try await tools.readFile("limit.txt") == String(repeating: "a", count: 60_000) + "\n…(truncated, 1000000 chars total; read the rest with start_line and line_count)")
    try await tools.writeFile(path: "empty.txt", content: "")
    #expect(try await tools.readFile("empty.txt") == "")
    #expect(try await tools.readFile("empty.txt", startLine: 1) == "(lines 1-1 of 1)\n")
}

@Test func nativeEditPreservesExactMatchingAndErrors() async throws {
    let fixture = try NativeFileFixture()
    defer { fixture.remove() }
    let tools = fixture.tools
    try await tools.writeFile(path: "nested/edit.txt", content: "before cafe\u{301} after")
    try await tools.editFile(path: "nested/edit.txt", old: "café", new: "tea")
    #expect(try await tools.readFile("nested/edit.txt") == "before tea after")
    for (content, old, expected) in [
        ("unchanged", "missing", "That exact text was not found in nested/edit.txt. Read the file and copy it exactly."),
        ("aaaaa", "aa", "That text appears 2 times in nested/edit.txt. Include more context so it matches once."),
        ("café cafe\u{301}", "café", "That text appears 2 times in nested/edit.txt. Include more context so it matches once."),
        ("unchanged", "", "old_string must not be empty. Read the file first, then edit an exact block.")
    ] {
        try await tools.writeFile(path: "nested/edit.txt", content: content)
        do {
            try await tools.editFile(path: "nested/edit.txt", old: old, new: "replacement")
            Issue.record("An ambiguous or missing match was accepted")
        } catch {
            #expect(error.localizedDescription == expected)
        }
        #expect(try await tools.readFile("nested/edit.txt") == content)
    }
}

@Test func nativeListPreservesOrderingSkipsAndLimit() async throws {
    let fixture = try NativeFileFixture()
    defer { fixture.remove() }
    let tools = fixture.tools
    for path in ["z.txt", "a.txt", ".hidden", ".DS_Store", "node_modules/skip.txt", "folder/inside.txt"] {
        try await tools.writeFile(path: path, content: path)
    }
    #expect(try await tools.listFiles("", recursive: false) == ".hidden\na.txt\nfolder/\nz.txt")
    let recursive = try await tools.listFiles("", recursive: true)
    #expect(Set(recursive.split(separator: "\n")) == ["a.txt", "z.txt", "folder/inside.txt"])
    for index in 0..<301 {
        try await tools.writeFile(path: "many/\(index).txt", content: "")
    }
    for recursive in [false, true] {
        let entries = try await tools.listFiles("many", recursive: recursive).split(separator: "\n")
        #expect(entries.count == 301)
        #expect(entries.last == "…(truncated)")
    }
}

@Test func canceledNativeFileOperationsThrowBeforeWork() async throws {
    let fixture = try NativeFileFixture()
    defer { fixture.remove() }
    let tools = fixture.tools
    try await tools.writeFile(path: "unchanged.txt", content: "original")
    for operation in 0..<4 {
        let task = Task {
            withUnsafeCurrentTask { $0?.cancel() }
            switch operation {
            case 0: _ = try await tools.readFile("unchanged.txt")
            case 1: try await tools.writeFile(path: "unchanged.txt", content: "changed")
            case 2: try await tools.editFile(path: "unchanged.txt", old: "original", new: "changed")
            default: _ = try await tools.listFiles("", recursive: true)
            }
        }
        task.cancel()
        do {
            try await task.value
            Issue.record("Canceled operation completed")
        } catch {
            #expect(error is CancellationError)
        }
    }
    #expect(try await tools.readFile("unchanged.txt") == "original")
}

@Test func concurrentNativeEditsPreserveEveryChange() async throws {
    let fixture = try NativeFileFixture()
    defer { fixture.remove() }
    let path = fixture.directory.appendingPathComponent("shared.txt")
    let original = String(repeating: "x", count: 2_000_000) + (0..<8).map { " old-\($0) " }.joined()
    try original.write(to: path, atomically: true, encoding: .utf8)
    try await withThrowingTaskGroup(of: Void.self) { group in
        for index in 0..<8 {
            group.addTask {
                let tools = NativeFileTools(workingDirectory: fixture.directory.path)
                try await tools.editFile(path: "shared.txt", old: " old-\(index) ", new: " new-\(index) ")
            }
        }
        try await group.waitForAll()
    }
    let actual = try String(contentsOf: path, encoding: .utf8)
    for index in 0..<8 {
        let preserved = actual.contains(" new-\(index) ")
        #expect(preserved)
    }
}

@main struct RuntimePerformanceTests {
    static func main() async {
        await MainActor.run { _ = NSApplication.shared.setActivationPolicy(.prohibited) }
        let result: CInt = await Testing.__swiftPMEntryPoint()
        exit(result)
    }
}
