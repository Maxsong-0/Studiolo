import AppKit
import Testing
@testable import DroppyCode

@Test @MainActor func threadAndScriptDefaultShortcuts() {
    #expect(AppShortcut.nextThread.defaultChord == KeyChord(keyCode: 48, modifiers: .control))
    #expect(AppShortcut.previousThread.defaultChord == KeyChord(keyCode: 48, modifiers: [.control, .shift]))
    #expect(AppShortcut.runScript.defaultChord == KeyChord(keyCode: 15, modifiers: .command))
}

@Test @MainActor func archiveAndSettleShortcutsHaveTheirChords() {
    #expect(AppShortcut.archiveThread.defaultChord == KeyChord(keyCode: 13, modifiers: .command))
    #expect(AppShortcut.settleThread.defaultChord == KeyChord(keyCode: 1, modifiers: .command))
    // Command-W is the archive chord now, so it is no longer refused as reserved.
    #expect(ShortcutStore.shared.refusal(for: KeyChord(keyCode: 13, modifiers: .command), replacing: .archiveThread) == nil)
}

@Test @MainActor func settleShortcutTogglesAndArchiveShortcutArchivesWithoutARow() async throws {
    let app = AppModel()
    let project = app.addProject(at: FileManager.default.temporaryDirectory)
    let thread = try #require(app.newThread(in: project, workspace: .local))
    #expect(app.selectedThreadID == thread.id)
    func settled(_ expected: Bool) async -> Bool {
        let deadline = ContinuousClock.now + .seconds(4)
        while app.thread(thread.id)?.isSettled != expected, ContinuousClock.now < deadline {
            try? await Task.sleep(for: .milliseconds(10))
        }
        return app.thread(thread.id)?.isSettled == expected
    }
    // No sidebar row takes the request up here, so the model acts on it itself.
    app.settleSelectedThread()
    #expect(await settled(true))
    app.settleSelectedThread()
    #expect(await settled(false))
    app.askToArchiveSelectedThread()
    #expect(app.archiveRequest?.threadID == thread.id)
    let deadline = ContinuousClock.now + .seconds(4)
    while app.thread(thread.id)?.isArchived != true, ContinuousClock.now < deadline {
        try await Task.sleep(for: .milliseconds(10))
    }
    #expect(app.thread(thread.id)?.isArchived == true)
    #expect(app.archiveRequest == nil)
}

@Test @MainActor func controlTabCyclesWhileEditing() throws {
    NSApplication.shared.setActivationPolicy(.prohibited)
    let window = ThreadWindow(contentRect: NSRect(x: 0, y: 0, width: 400, height: 200), styleMask: .titled, backing: .buffered, defer: false)
    window.isReleasedWhenClosed = false
    let editor = NSTextView(frame: window.contentView!.bounds)
    editor.string = "Keep this draft"
    window.contentView?.addSubview(editor)
    window.makeFirstResponder(editor)
    var offsets: [Int] = []
    window.selectThread = { offsets.append($0) }
    for flags: NSEvent.ModifierFlags in [.control, [.control, .shift]] {
        let event = try #require(NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: flags, timestamp: 0, windowNumber: window.windowNumber, context: nil, characters: "\t", charactersIgnoringModifiers: "\t", isARepeat: false, keyCode: 48))
        #expect(window.performKeyEquivalent(with: event))
    }
    #expect(offsets == [1, -1])
    #expect(editor.string == "Keep this draft")
    window.close()
}

@Test func revertTouchesOnlyTheThreadsOwnFiles() async throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent("droppy-revert-\(UUID().uuidString)")
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: directory) }
    let git = Git(directory.path)
    try Git.check(await git.run(["init", "-q"]))
    func write(_ name: String, _ text: String) throws {
        try text.write(to: directory.appendingPathComponent(name), atomically: true, encoding: .utf8)
    }
    try write("mine.txt", "original\n")
    try write("theirs.txt", "theirs\n")
    try write("shared.txt", "shared\n")
    try Git.check(await git.run(["add", "."]))
    try Git.check(await git.run(["-c", "user.name=Test", "-c", "user.email=test@example.invalid", "commit", "-qm", "Initial"]))
    try await git.captureCheckpoint("refs/test/base")
    // The thread's turn: edits mine.txt and shared.txt, creates created.txt.
    try write("mine.txt", "replacement\nextra\n")
    try write("shared.txt", "shared by the thread\n")
    try write("created.txt", "new\n")
    try await git.captureCheckpoint("refs/test/end")
    // Afterwards: another thread edits theirs.txt, and someone edits shared.txt again.
    try write("theirs.txt", "changed elsewhere\n")
    try write("shared.txt", "shared, then changed again\n")
    let before = try await git.output(["status", "--porcelain=v1", "-z"])

    let preview = try await git.previewRestore(base: "refs/test/base", end: "refs/test/end", paths: ["mine.txt", "shared.txt", "created.txt"])
    #expect(preview.files.map(\.path) == ["created.txt", "mine.txt", "shared.txt"])
    #expect(!preview.files.contains { $0.path == "theirs.txt" })
    #expect(preview.files.first { $0.path == "shared.txt" }?.isKept == true)
    #expect(preview.restorable.map(\.path) == ["created.txt", "mine.txt"])
    #expect(preview.additions == 3 && preview.deletions == 1)
    // Only the preview ran: the index is as it was.
    #expect(try await git.output(["status", "--porcelain=v1", "-z"]) == before)

    try await git.restore(preview.restorable.map(\.path), from: "refs/test/base")
    #expect(try String(contentsOf: directory.appendingPathComponent("mine.txt"), encoding: .utf8) == "original\n")
    #expect(!FileManager.default.fileExists(atPath: directory.appendingPathComponent("created.txt").path))
    #expect(try String(contentsOf: directory.appendingPathComponent("theirs.txt"), encoding: .utf8) == "changed elsewhere\n")
    #expect(try String(contentsOf: directory.appendingPathComponent("shared.txt"), encoding: .utf8) == "shared, then changed again\n")
}

@main struct AppIntegrationTests {
    static func main() async {
        // Finished turns ask the app whether it is active; no window is ever shown.
        await MainActor.run { _ = NSApplication.shared.setActivationPolicy(.prohibited) }
        let result: CInt = await Testing.__swiftPMEntryPoint()
        exit(result)
    }
}
