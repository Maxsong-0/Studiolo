import Foundation
import Synchronization
import Testing
@testable import DroppyCode

@MainActor private func fixture(provider: ProviderKind = .codex, turn: TurnRecord, earlier: [TurnRecord] = [], items: [TimelineItem], projectPath: String? = nil) async throws -> (AppModel, ThreadRuntime) {
    try FileManager.default.createDirectory(at: Storage.root, withIntermediateDirectories: true)
    let project = Project(name: "Regression", path: projectPath ?? Storage.root.path)
    var thread = ChatThread(projectID: project.id, provider: provider, model: "test", effort: nil, runtimeMode: .supervised)
    thread.providerSessionID = "existing-session"
    var library = Library()
    library.projects = [project]
    library.threads = [thread]
    var document = ThreadDocument(threadID: thread.id)
    document.turns = earlier + [turn]
    document.items = items
    let encoder = JSONEncoder()
    encoder.dateEncodingStrategy = .iso8601
    try encoder.encode(library).write(to: Storage.libraryURL)
    try encoder.encode(document).write(to: Storage.threadURL(thread.id))
    let model = AppModel()
    model.settings.setBinaryPath("/nonexistent/provider-for-regression", for: provider)
    // The history loads off the main thread; the tests below read it at once.
    let runtime = ThreadRuntime(threadID: thread.id, app: model)
    await runtime.ensureLoaded()
    return (model, runtime)
}

@Test @MainActor func turnDiffHandlesProjectsInsideGitSubdirectories() async throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent("subdir-diff-\(UUID().uuidString)")
    let nested = directory.appendingPathComponent("nested")
    try FileManager.default.createDirectory(at: nested, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: directory) }
    let git = Git(directory.path)
    try Git.check(await git.run(["init", "-q"]))
    let file = nested.appendingPathComponent("file.txt")
    try "old\n".write(to: file, atomically: true, encoding: .utf8)
    try await git.captureCheckpoint("refs/test/base")
    try "new\n".write(to: file, atomically: true, encoding: .utf8)
    try await git.captureCheckpoint("refs/test/end")
    var turn = TurnRecord(index: 0)
    turn.baseCheckpoint = "refs/test/base"
    turn.endCheckpoint = "refs/test/end"
    turn.touchedPaths = ["file.txt"]
    var call = ToolCall(kind: .edit, title: "Edit")
    call.edits = [FileEdit(path: file.path, diff: "@@ -1 +1 @@\n-old\n+new\n", additions: 1, deletions: 1)]
    let (model, runtime) = try await fixture(turn: turn, items: [TimelineItem(turnID: turn.id, content: .tool(call))], projectPath: nested.path)
    let files = await runtime.parsedDiff(selection: turn.id)
    #expect(files.map(\.path) == ["nested/file.txt"])
    #expect(files.first?.additions == 1)
    let repository = try await Git(nested.path).repositoryRoot()
    let patch = try await git.diff(from: "refs/test/base", to: "refs/test/end")
    let shellEdits = await ThreadRuntime.fileEdits(from: patch, repositoryRoot: repository)
    #expect(shellEdits.map(\.path) == [file.path])
    let shellFiles = TurnDiff.merge(snapshot: DiffParser.parse(patch), providerPatch: "", edits: shellEdits, touched: Set(shellEdits.map(\.path)), root: nested.path, repositoryRoot: repository.path)
    #expect(shellFiles.map(\.path) == ["nested/file.txt"])
    withExtendedLifetime(model) {}
}

@Test func turnDiffUsesNetSnapshotWithoutCountingReportedEditsAgain() {
    let patch = "diff --git a/file.txt b/file.txt\n--- a/file.txt\n+++ b/file.txt\n@@ -1 +1 @@\n-old\n+new\n"
    let edit = FileEdit(path: "/repo/file.txt", diff: patch, additions: 1, deletions: 1)
    let files = TurnDiff.merge(snapshot: DiffParser.parse(patch), providerPatch: patch, edits: [edit, edit], touched: ["file.txt"], root: "/repo")
    #expect(files.count == 1)
    #expect(files.first?.additions == 1)
    #expect(files.first?.deletions == 1)
    #expect(TurnDiff.merge(snapshot: [], providerPatch: patch, edits: [edit], touched: ["file.txt"], root: "/repo").isEmpty)
}

@Test func turnDiffIncludesRelativeExternalPaths() {
    let edit = FileEdit(path: "../other/file.txt", diff: "@@ -1 +1 @@\n-old\n+new\n", additions: 1, deletions: 1)
    let files = TurnDiff.merge(snapshot: [], providerPatch: "", edits: [edit], touched: nil, root: "/repo")
    #expect(files.map(\.path) == ["/other/file.txt"])
}

@Test @MainActor func claudeRevertPersistsItsResumeBoundary() async throws {
    var earlier = TurnRecord(index: 0)
    earlier.status = .completed
    earlier.providerAnchor = "previous-assistant-message"
    var turn = TurnRecord(index: 1)
    turn.status = .completed
    let (model, runtime) = try await fixture(provider: .claude, turn: turn, earlier: [earlier], items: [])
    try await runtime.revert(to: turn.id, restoreFiles: false)
    let thread = try #require(model.thread(runtime.threadID))
    let encoded = try JSONSerialization.jsonObject(with: JSONEncoder().encode(thread)) as? [String: Any]
    #expect(encoded?["providerResumeAt"] as? String == "previous-assistant-message")
    #expect(runtime.turns.map(\.id) == [earlier.id])
}

@Test @MainActor func failedProviderRevertKeepsConversation() async throws {
    var turn = TurnRecord(index: 0)
    turn.status = .completed
    turn.providerTurnID = "native-turn"
    let message = TimelineItem(turnID: turn.id, content: .user(UserMessage(text: "Keep until provider confirms", attachments: [])))
    turn.userItemID = message.id
    let (model, runtime) = try await fixture(turn: turn, items: [message])
    runtime.draft.text = "Existing draft"
    await #expect(throws: (any Error).self) {
        try await runtime.resend(turn.id, text: "Edited", attachments: [], restoreFiles: false)
    }
    #expect(runtime.turns.count == 1)
    #expect(runtime.entries.contains { $0.id == message.id })
    #expect(runtime.draft.text == "Existing draft")
    #expect(model.thread(runtime.threadID)?.providerSessionID == "existing-session")
}

@Test @MainActor func turnDiffIncludesReportedEditsOutsideTheProject() async throws {
    var turn = TurnRecord(index: 0)
    turn.status = .completed
    turn.providerDiff = "diff --git a/local.txt b/local.txt\n--- a/local.txt\n+++ b/local.txt\n@@ -1 +1 @@\n-old\n+new\n"
    let external = FileEdit(path: "/outside/project/other.swift", diff: "@@ -1 +1,2 @@\n-old\n+new\n+extra\n", additions: 2, deletions: 1)
    var call = ToolCall(kind: .edit, title: "Edit files")
    call.edits = [external]
    let entry = TimelineItem(turnID: turn.id, content: .tool(call))
    let (model, runtime) = try await fixture(turn: turn, items: [entry])
    let files = await runtime.parsedDiff(selection: turn.id)
    #expect(files.count == 2)
    #expect(files.reduce(0) { $0 + $1.additions } == 3)
    #expect(files.reduce(0) { $0 + $1.deletions } == 2)
    await runtime.refreshChangeStats()
    #expect(runtime.changeStats == FileChangeSummary(files: files))
    withExtendedLifetime(model) {}
}

@Test @MainActor func allChangesUsesNetEditsAcrossTurns() async throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent("multi-turn-diff-\(UUID().uuidString)")
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: directory) }
    let git = Git(directory.path)
    try Git.check(await git.run(["init", "-q"]))
    let file = directory.appendingPathComponent("shared.txt")
    try "original\n".write(to: file, atomically: true, encoding: .utf8)
    try await git.captureCheckpoint("refs/test/base")
    try "temporary\nextra\n".write(to: file, atomically: true, encoding: .utf8)
    try await git.captureCheckpoint("refs/test/middle")
    try "final\n".write(to: file, atomically: true, encoding: .utf8)
    try "added\n".write(to: directory.appendingPathComponent("new.txt"), atomically: true, encoding: .utf8)
    try await git.captureCheckpoint("refs/test/end")
    var first = TurnRecord(index: 0)
    first.baseCheckpoint = "refs/test/base"
    first.endCheckpoint = "refs/test/middle"
    first.touchedPaths = ["shared.txt"]
    var second = TurnRecord(index: 1)
    second.baseCheckpoint = "refs/test/middle"
    second.endCheckpoint = "refs/test/end"
    second.touchedPaths = ["shared.txt", "new.txt"]
    let (model, runtime) = try await fixture(turn: second, earlier: [first], items: [], projectPath: directory.path)
    await runtime.refreshChangeStats()
    let all = FileChangeSummary(files: await runtime.parsedDiff(selection: nil))
    #expect(all.files.count == 2)
    #expect(all.additions == 2)
    #expect(all.deletions == 1)
    #expect(runtime.changeStats == all)
    let selected = FileChangeSummary(files: await runtime.parsedDiff(selection: second.id))
    #expect(selected.files.count == 2)
    #expect(selected.additions == 2)
    #expect(selected.deletions == 2)
    withExtendedLifetime(model) {}
}

@Test @MainActor func allChangesKeepsEveryReportedFile() async throws {
    let turn = TurnRecord(index: 0)
    var call = ToolCall(kind: .edit, title: "Edit files")
    call.edits = (0..<137).map { FileEdit(path: "/external/file-\($0).txt", diff: "@@ -0,0 +1 @@\n+line\n", additions: 1, deletions: 0) }
    let (model, runtime) = try await fixture(turn: turn, items: [TimelineItem(turnID: turn.id, content: .tool(call))])
    await runtime.refreshChangeStats()
    let all = await runtime.parsedDiff(selection: nil)
    #expect(all.count == call.edits.count)
    #expect(runtime.changeStats?.files.count == all.count)
    #expect(runtime.changeStats?.additions == call.edits.count)
    #expect(runtime.turnsWithChanges().map(\.id) == [turn.id])
    withExtendedLifetime(model) {}
}

@Test @MainActor func allChangesIncludesLegacyTurnsBeforeCheckpointedTurns() async throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent("legacy-diff-\(UUID().uuidString)")
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: directory) }
    let git = Git(directory.path)
    try Git.check(await git.run(["init", "-q"]))
    try "legacy\n".write(to: directory.appendingPathComponent("before.txt"), atomically: true, encoding: .utf8)
    try await git.captureCheckpoint("refs/test/base")
    try "current\n".write(to: directory.appendingPathComponent("after.txt"), atomically: true, encoding: .utf8)
    try await git.captureCheckpoint("refs/test/end")
    var first = TurnRecord(index: 0)
    first.providerDiff = "diff --git a/before.txt b/before.txt\n--- /dev/null\n+++ b/before.txt\n@@ -0,0 +1 @@\n+legacy\n"
    var second = TurnRecord(index: 1)
    second.baseCheckpoint = "refs/test/base"
    second.endCheckpoint = "refs/test/end"
    second.touchedPaths = ["after.txt"]
    second.providerDiff = try await git.diff(from: "refs/test/base", to: "refs/test/end")
    let (model, runtime) = try await fixture(turn: second, earlier: [first], items: [], projectPath: directory.path)
    await runtime.refreshChangeStats()
    let all = await runtime.parsedDiff(selection: nil)
    #expect(Set(all.map(\.path)) == ["before.txt", "after.txt"])
    #expect(runtime.changeStats?.files.count == 2)
    #expect(runtime.changeStats?.additions == 2)
    #expect(runtime.turnsWithChanges().count == 2)
    withExtendedLifetime(model) {}
}

@Test @MainActor func timelineBuildsWhenTheFirstEntryHasNoTurn() {
    // A notice appended after a revert emptied the thread has no turn; `runs.last?.turnID`
    // matched its nil on an empty list and indexed -1.
    let notice = TimelineEntry(TimelineItem(turnID: nil, content: .notice(Notice(level: .error, message: "Files could not be restored."))))
    let blocks = DisplayBlock.build([notice], meta: TimelineMeta.build([notice]), showReasoning: true, isRunning: false, hydraMergeID: nil)
    #expect(blocks.count == 1)
}

@Test @MainActor func promptEditsFollowTheirPromptsOutOfTheThread() async throws {
    var earlier = TurnRecord(index: 0)
    earlier.status = .completed
    earlier.providerAnchor = "previous-assistant-message"
    var turn = TurnRecord(index: 1)
    turn.status = .completed
    let (model, runtime) = try await fixture(provider: .claude, turn: turn, earlier: [earlier], items: [])
    runtime.enqueueFollowUp(text: "Queued", attachments: [])
    let queued = try #require(runtime.followUps.first)
    runtime.followUpEdit = PromptEdit(id: queued.id, text: "Queued, edited", attachments: [])
    runtime.messageEdit = PromptEdit(id: turn.id, text: "Sent, edited", attachments: [])
    runtime.removeFollowUp(queued.id)
    #expect(runtime.followUpEdit == nil)
    #expect(runtime.messageEdit?.id == turn.id)
    try await runtime.revert(to: turn.id, restoreFiles: false)
    #expect(runtime.messageEdit == nil)
    withExtendedLifetime(model) {}
}

@Test @MainActor func resendKeepsTheComposerAndSendsTheEdit() async throws {
    var earlier = TurnRecord(index: 0)
    earlier.status = .completed
    earlier.providerAnchor = "previous-assistant-message"
    var turn = TurnRecord(index: 1)
    turn.status = .completed
    let message = TimelineItem(turnID: turn.id, content: .user(UserMessage(text: "Original", attachments: [])))
    turn.userItemID = message.id
    let (model, runtime) = try await fixture(provider: .claude, turn: turn, earlier: [earlier], items: [message])
    runtime.draft.text = "Existing draft"
    try await runtime.resend(turn.id, text: "Edited", attachments: [], restoreFiles: false)
    // The send starts its turn on the next hop.
    for _ in 0..<50 where runtime.turns.count < 2 { await Task.yield() }
    #expect(runtime.draft.text == "Existing draft")
    #expect(runtime.turns.count == 2)
    #expect(runtime.turns.last?.id != turn.id)
    let sent = runtime.entries.first { $0.turnID == runtime.turns.last?.id && $0.kind == .user }
    guard case .user(let resent)? = sent?.item.content else {
        Issue.record("The edit was not sent as a message")
        return
    }
    #expect(resent.text == "Edited")
    #expect(!runtime.entries.contains { $0.id == message.id })
    withExtendedLifetime(model) {}
}

@Test @MainActor func revertPreviewIsSharedBetweenHoverAndClick() async throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent("preview-cache-\(UUID().uuidString)")
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: directory) }
    let git = Git(directory.path)
    try Git.check(await git.run(["init", "-q"]))
    let file = directory.appendingPathComponent("file.txt")
    try "old\n".write(to: file, atomically: true, encoding: .utf8)
    try await git.captureCheckpoint("refs/test/base")
    try "new\n".write(to: file, atomically: true, encoding: .utf8)
    // Changed in the same checkout by someone else: never this thread's to undo.
    try "elsewhere\n".write(to: directory.appendingPathComponent("other.txt"), atomically: true, encoding: .utf8)
    var turn = TurnRecord(index: 0)
    turn.status = .completed
    turn.baseCheckpoint = "refs/test/base"
    turn.touchedPaths = ["file.txt"]
    let (model, runtime) = try await fixture(turn: turn, items: [], projectPath: directory.path)
    #expect(runtime.cachedRevertPreview(for: turn.id) == nil)
    let hover = runtime.revertPreview(for: turn.id)
    let click = runtime.revertPreview(for: turn.id)
    let preview = try await click.value
    #expect(preview.files.map(\.path) == ["file.txt"])
    _ = try await hover.value
    guard case .success(let cached)? = runtime.cachedRevertPreview(for: turn.id) else {
        Issue.record("The finished preview was not kept for the editor to open on")
        return
    }
    #expect(cached.additions == 1 && cached.deletions == 1)
    withExtendedLifetime(model) {}
}

@Test @MainActor func aTurnThatTouchedNothingHasNothingToUndo() async throws {
    // "hi" in a checkout other agents are busy in: the working tree differs from the
    // checkpoint everywhere, and none of it is this thread's.
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent("untouched-\(UUID().uuidString)")
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: directory) }
    let git = Git(directory.path)
    try Git.check(await git.run(["init", "-q"]))
    try "old\n".write(to: directory.appendingPathComponent("busy.txt"), atomically: true, encoding: .utf8)
    try await git.captureCheckpoint("refs/test/base")
    try "changed by another thread\n".write(to: directory.appendingPathComponent("busy.txt"), atomically: true, encoding: .utf8)
    try "and a new file\n".write(to: directory.appendingPathComponent("new.txt"), atomically: true, encoding: .utf8)
    var turn = TurnRecord(index: 0)
    turn.status = .completed
    turn.baseCheckpoint = "refs/test/base"
    turn.touchedPaths = []
    let (model, runtime) = try await fixture(turn: turn, items: [], projectPath: directory.path)
    let preview = try await runtime.revertPreview(for: turn.id).value
    #expect(preview.files.isEmpty)
    withExtendedLifetime(model) {}
}

@Test func workingTreeWatchReusesTreesUntilSomethingIsWritten() async throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent("watch-\(UUID().uuidString)").resolvingSymlinksInPath()
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: directory) }
    let git = Git(directory.path)
    try Git.check(await git.run(["init", "-q"]))
    let file = directory.appendingPathComponent("file.txt")
    try "old\n".write(to: file, atomically: true, encoding: .utf8)
    try Git.check(await git.run(["add", "."]))
    try Git.check(await git.run(["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "-m", "one"]))
    let committed = try await git.treeHash(of: "HEAD")
    let watch = try #require(WorkingTreeWatch.shared(for: directory.path, gitDirectory: directory.appendingPathComponent(".git").path))
    let captures = Mutex<[[String]?]>([])
    @Sendable func capture(_ scratch: URL?, _ paths: [String]?) async throws -> String {
        captures.withLock { $0.append(paths) }
        return try await git.captureTree()
    }
    let indexPath = directory.appendingPathComponent(".git/index").path
    // Nothing written between two asks: one capture serves both, and the first walks everything.
    #expect(try await watch.currentTree(indexPath: indexPath, capture: capture) == committed)
    #expect(try await watch.currentTree(indexPath: indexPath, capture: capture) == committed)
    #expect(captures.withLock { $0 } == [nil])
    // Two asks at once share one capture, which walks only what changed: the new directory
    // and the edited file at the root.
    let nested = directory.appendingPathComponent("src/deep", isDirectory: true)
    try FileManager.default.createDirectory(at: nested, withIntermediateDirectories: true)
    try "new\n".write(to: nested.appendingPathComponent("n.txt"), atomically: true, encoding: .utf8)
    try "new\n".write(to: file, atomically: false, encoding: .utf8)
    async let first = watch.currentTree(indexPath: indexPath, capture: capture)
    async let second = watch.currentTree(indexPath: indexPath, capture: capture)
    let (changed, same) = try await (first, second)
    #expect(changed != committed)
    #expect(changed == same)
    #expect(captures.withLock { $0 } == [nil, ["file.txt", "src"]])
    // Git's own writes under .git are not changes to the working files.
    try "x".write(to: directory.appendingPathComponent(".git/droppy-probe"), atomically: true, encoding: .utf8)
    #expect(try await watch.currentTree(indexPath: indexPath, capture: capture) == changed)
    #expect(captures.withLock { $0.count } == 2)
    // The index changing is a change, and one that means the whole tree: staging is part
    // of what write-tree writes.
    try Git.check(await git.run(["add", "file.txt"]))
    #expect(try await watch.currentTree(indexPath: indexPath, capture: capture) == changed)
    #expect(captures.withLock { $0 } == [nil, ["file.txt", "src"], nil])
    // Putting things back gives the committed tree again, not a stale one, from a walk
    // of the removed directory and the file alone; unstaging afterwards walks everything.
    try FileManager.default.removeItem(at: directory.appendingPathComponent("src"))
    try "old\n".write(to: file, atomically: false, encoding: .utf8)
    #expect(try await watch.currentTree(indexPath: indexPath, capture: capture) == committed)
    #expect(captures.withLock { $0 } == [nil, ["file.txt", "src"], nil, ["file.txt", "src"]])
    try Git.check(await git.run(["reset", "-q"]))
    #expect(try await watch.currentTree(indexPath: indexPath, capture: capture) == committed)
    #expect(captures.withLock { $0.count } == 5)
    // Two checkpoints on an untouched tree share one commit.
    try await git.captureCheckpoint("refs/droppy-code/test/a")
    try await git.captureCheckpoint("refs/droppy-code/test/b")
    #expect(try await git.commitHash(of: "refs/droppy-code/test/a") == git.commitHash(of: "refs/droppy-code/test/b"))
    try "again\n".write(to: file, atomically: true, encoding: .utf8)
    try await git.captureCheckpoint("refs/droppy-code/test/c")
    #expect(try await git.commitHash(of: "refs/droppy-code/test/c") != git.commitHash(of: "refs/droppy-code/test/a"))
    #expect(try await git.treeHash(of: "refs/droppy-code/test/c") != committed)
}

@Test func treeSnapshotsLeaveTheRealIndexAloneInCheckoutsAndWorktrees() async throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent("snapshot-\(UUID().uuidString)")
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: directory) }
    let git = Git(directory.path)
    try Git.check(await git.run(["init", "-q"]))
    let file = directory.appendingPathComponent("file.txt")
    try "old\n".write(to: file, atomically: true, encoding: .utf8)
    try Git.check(await git.run(["add", "."]))
    try Git.check(await git.run(["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "-m", "one"]))
    let committed = try await git.treeHash(of: "HEAD")
    #expect(try await git.captureTree() == committed)
    try "new\n".write(to: file, atomically: true, encoding: .utf8)
    try "untracked\n".write(to: directory.appendingPathComponent("extra.txt"), atomically: true, encoding: .utf8)
    let changed = try await git.captureTree()
    #expect(changed != committed)
    #expect(try await git.output(["ls-tree", "--name-only", changed]).split(separator: "\n").map(String.init) == ["extra.txt", "file.txt"])
    // The snapshot staged nothing in the checkout's own index.
    #expect(try await git.output(["status", "--porcelain"]) == " M file.txt\n?? extra.txt\n")
    // A worktree keeps its index under the main checkout's .git, and snapshots the same way.
    let worktree = directory.appendingPathComponent("wt")
    try await git.addWorktree(at: worktree.path, branch: "wt", base: nil)
    let inWorktree = Git(worktree.path)
    #expect(try await inWorktree.captureTree() == committed)
    try "edited\n".write(to: worktree.appendingPathComponent("file.txt"), atomically: true, encoding: .utf8)
    #expect(try await inWorktree.captureTree() != committed)
    #expect(try await inWorktree.output(["status", "--porcelain"]) == " M file.txt\n")
}

@Test func gitRunsTheDeveloperBinaryRatherThanTheShim() async throws {
    await LoginEnvironment.load()
    let git = LoginEnvironment.git
    let developer = try await Shell.run(URL(fileURLWithPath: "/usr/bin/xcode-select"), ["-p"], environment: LoginEnvironment.current)
    guard developer.succeeded, FileManager.default.isExecutableFile(atPath: developer.trimmedOutput + "/usr/bin/git") else {
        #expect(git.path == "/usr/bin/git")
        return
    }
    #expect(git.path == developer.trimmedOutput + "/usr/bin/git")
    #expect(try await Git(FileManager.default.temporaryDirectory.path).run(["--version"]).succeeded)
}

@Test func worktreeIndexPathIsResolvedOnceWhileItsGitFileStands() async throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent("index-path-\(UUID().uuidString)")
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: directory) }
    let git = Git(directory.path)
    try Git.check(await git.run(["init", "-q"]))
    try "one\n".write(to: directory.appendingPathComponent("file.txt"), atomically: true, encoding: .utf8)
    try Git.check(await git.run(["add", "."]))
    try Git.check(await git.run(["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "-m", "one"]))
    let worktree = directory.appendingPathComponent("wt")
    try await git.addWorktree(at: worktree.path, branch: "wt", base: nil)
    // A checkout with its own .git directory never asks git and is never cached.
    _ = try await git.currentTree()
    #expect(Git.worktreeIndexPaths.withLock { $0[directory.path] } == nil)
    // A worktree asks once; the answer is git's, exists, and is served to the next ask.
    let expected = try await Git(worktree.path).output(["rev-parse", "--path-format=absolute", "--git-path", "index"])
        .trimmingCharacters(in: .whitespacesAndNewlines)
    _ = try await Git(worktree.path).currentTree()
    let cached = Git.worktreeIndexPaths.withLock { $0[worktree.path] }
    #expect(cached?.path == expected)
    #expect(FileManager.default.fileExists(atPath: expected))
    let stamp = cached?.stamp
    _ = try await Git(worktree.path).currentTree()
    #expect(Git.worktreeIndexPaths.withLock { $0[worktree.path] }?.stamp == stamp)
    // Rewriting the .git file (as `git worktree repair` does) drops the cached answer.
    let gitFile = worktree.appendingPathComponent(".git")
    let content = try String(contentsOf: gitFile, encoding: .utf8)
    try (content + "\n").write(to: gitFile, atomically: true, encoding: .utf8)
    _ = try await Git(worktree.path).currentTree()
    #expect(Git.worktreeIndexPaths.withLock { $0[worktree.path] }?.stamp != stamp)
    #expect(Git.worktreeIndexPaths.withLock { $0[worktree.path] }?.path == expected)
}

@Test @MainActor func draftEmptinessIgnoresWhitespaceWithoutTrimming() {
    #expect(ComposerDraft(text: "").isEmpty)
    #expect(ComposerDraft(text: " \n\t\u{00A0}\u{2028}").isEmpty)
    #expect(!ComposerDraft(text: "\n a \n").isEmpty)
    #expect(!ComposerDraft(text: "", attachments: [Attachment(name: "a", path: "/a", mimeType: "text/plain")]).isEmpty)
    #expect(FollowUpPrompt(text: "   ").isEmpty)
    #expect(!PromptEdit(id: UUID(), text: "x", attachments: []).isEmpty)
}
