import AppKit
import Foundation
import ImageIO
import SwiftUI
import Synchronization
import Testing
import UniformTypeIdentifiers
@testable import DroppyCode

private func waitForSignal(_ signal: DispatchSemaphore) async -> Bool {
    await withCheckedContinuation { continuation in
        DispatchQueue.global().async {
            continuation.resume(returning: signal.wait(timeout: .now() + 3) == .success)
        }
    }
}

@Test func recentCacheHonorsCostAcrossPromotionReplacementAndRemoval() {
    var cache = RecentCache<String, String>(limit: 4, costLimit: 10)
    cache.insert("old", for: "a", cost: 6)
    cache.insert("b", for: "b", cost: 6)
    #expect(cache.value(for: "a") == "old")
    cache.insert("replacement", for: "a", cost: 4)
    cache.insert("c", for: "c", cost: 6)
    #expect(cache.removeValue(for: "a") == "replacement")
    #expect(!cache.contains("a"))
    #expect(cache.removeValue(for: "a") == nil)
    cache.insert("oversized", for: "c", cost: 11)
    #expect(!cache.contains("c"))
    cache.insert("d", for: "d", cost: 10)
    cache.insert("e", for: "e", cost: 10)
    #expect(!cache.contains("b"))
    #expect(cache.value(for: "d") == "d")
    #expect(cache.value(for: "e") == "e")
}

@Test func recentCacheHandlesCountAndIntegerLimits() {
    var cache = RecentCache<Int, Int>(limit: 1, costLimit: .max)
    cache.insert(1, for: 1, cost: .max)
    cache.insert(2, for: 2, cost: .max)
    #expect(cache.value(for: 1) == 1)
    cache.insert(3, for: 3, cost: .max)
    #expect(!cache.contains(2))
    #expect(cache.removeValue(for: 1) == 1)
    #expect(cache.removeValue(for: 3) == 3)
    #expect(!cache.contains(1))
}

private final class PrefetchProbe: Sendable {
    let entered = DispatchSemaphore(value: 0)
    let proceed = DispatchSemaphore(value: 0)
    let calls = Mutex<[UUID]>([])
    let gate = Mutex(true)

    func waitForEntry() async -> Bool {
        await waitForSignal(entered)
    }

    func load(_ id: UUID) -> (document: ThreadDocument, cost: Int)? {
        calls.withLock { $0.append(id) }
        if gate.withLock({ value in let result = value; value = false; return result }) {
            entered.signal()
            proceed.wait()
        }
        return (ThreadDocument(threadID: id), 1)
    }
}

private struct GatedSnapshot: Encodable, Sendable {
    let value: String
    let entered: DispatchSemaphore
    let proceed: DispatchSemaphore

    func encode(to encoder: any Encoder) throws {
        entered.signal()
        proceed.wait()
        var container = encoder.singleValueContainer()
        try container.encode(value)
    }
}

@Test func finalWriteFollowsAlreadyQueuedSnapshot() async throws {
    let url = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: url) }
    let entered = DispatchSemaphore(value: 0)
    let proceed = DispatchSemaphore(value: 0)
    defer { proceed.signal() }
    let writer = DiskWriter()
    writer.encodeAndWrite(GatedSnapshot(value: "old", entered: entered, proceed: proceed), to: url)
    try #require(await waitForSignal(entered))
    let finalWrite = Task.detached { writer.writeSynchronously("new", to: url) }
    proceed.signal()
    await finalWrite.value
    #expect(try JSONDecoder().decode(String.self, from: Data(contentsOf: url)) == "new")
}

@Test func deletionFollowsAlreadyQueuedSnapshot() async throws {
    let url = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: url) }
    let entered = DispatchSemaphore(value: 0)
    let proceed = DispatchSemaphore(value: 0)
    defer { proceed.signal() }
    let writer = DiskWriter()
    writer.encodeAndWrite(GatedSnapshot(value: "old", entered: entered, proceed: proceed), to: url)
    try #require(await waitForSignal(entered))
    let deletion = Task.detached { writer.removeSynchronously(url) }
    proceed.signal()
    await deletion.value
    #expect(!FileManager.default.fileExists(atPath: url.path))
}

private final class CountingSnapshot: Encodable, Sendable {
    static let encoded = Mutex<[String]>([])
    let value: String

    init(_ value: String) {
        self.value = value
    }

    func encode(to encoder: any Encoder) throws {
        Self.encoded.withLock { $0.append(value) }
        var container = encoder.singleValueContainer()
        try container.encode(value)
    }
}

@Test func queuedSnapshotsForOneFileCoalesceToTheNewest() async throws {
    let url = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    let other = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: url); try? FileManager.default.removeItem(at: other) }
    let entered = DispatchSemaphore(value: 0)
    let proceed = DispatchSemaphore(value: 0)
    let writer = DiskWriter()
    let blocker = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: blocker) }
    writer.encodeAndWrite(GatedSnapshot(value: "blocking", entered: entered, proceed: proceed), to: blocker)
    try #require(await waitForSignal(entered))
    CountingSnapshot.encoded.withLock { $0.removeAll() }
    for value in ["one", "two", "three"] { writer.encodeAndWrite(CountingSnapshot(value), to: url) }
    writer.encodeAndWrite(CountingSnapshot("other"), to: other)
    proceed.signal()
    writer.waitForQueuedWrites()
    #expect(CountingSnapshot.encoded.withLock { $0 } == ["three", "other"])
    #expect(try JSONDecoder().decode(String.self, from: Data(contentsOf: url)) == "three")
    #expect(try JSONDecoder().decode(String.self, from: Data(contentsOf: other)) == "other")
}

@Test func urgentPrefetchRunsAheadOfWhatLaunchQueued() async throws {
    let probe = PrefetchProbe()
    let cache = DocumentPrefetch(limit: 20, pendingLimit: 24, load: { probe.load($0) })
    let launch = (0..<12).map { _ in UUID() }
    let hovered = UUID()
    let task = cache.warm(launch)
    #expect(await probe.waitForEntry())
    cache.warm([hovered, launch[11]], urgent: true)
    probe.proceed.signal()
    await task?.value
    let calls = probe.calls.withLock { $0 }
    #expect(calls.count == 13)
    #expect(Array(calls.prefix(3)) == [launch[0], hovered, launch[11]])
}

@Test func prefetchBoundsPendingWorkAndCoalescesRequests() async throws {
    let probe = PrefetchProbe()
    let cache = DocumentPrefetch(limit: 2, pendingLimit: 3, load: { probe.load($0) })
    let ids = (0..<20).map { _ in UUID() }
    let task = cache.warm(ids)
    #expect(await probe.waitForEntry())
    cache.warm(ids)
    probe.proceed.signal()
    await task?.value
    let calls = probe.calls.withLock { $0 }
    #expect(calls.count <= 4)
    #expect(Set(calls).count == calls.count)
    let loaded = ids.compactMap { cache.take($0) }
    #expect(loaded.count <= 4)
    #expect(!loaded.isEmpty)
    #expect(ids.allSatisfy { cache.take($0) == nil })
}

@Test func prefetchDiscardsInvalidatedDecodeAndAllowsFreshGeneration() async throws {
    let probe = PrefetchProbe()
    let cache = DocumentPrefetch(limit: 2, load: { probe.load($0) })
    let id = UUID()
    let task = cache.warm([id])
    #expect(await probe.waitForEntry())
    cache.forget(id)
    cache.warm([id])
    probe.proceed.signal()
    await task?.value
    #expect(probe.calls.withLock { $0.count } == 2)
    #expect(cache.take(id)?.threadID == id)
    #expect(cache.take(id) == nil)
}

@Test func takingInFlightPrefetchCancelsItsPublication() async throws {
    let probe = PrefetchProbe()
    let cache = DocumentPrefetch(limit: 2, load: { probe.load($0) })
    let id = UUID()
    let task = cache.warm([id])
    #expect(await probe.waitForEntry())
    #expect(cache.take(id) == nil)
    probe.proceed.signal()
    await task?.value
    #expect(cache.take(id) == nil)
    await cache.warm([id])?.value
    #expect(cache.take(id)?.threadID == id)
}

@Test func cancellingPrefetchStopsQueuedWork() async {
    let probe = PrefetchProbe()
    let cache = DocumentPrefetch(limit: 2, load: { probe.load($0) })
    let ids = (0..<4).map { _ in UUID() }
    let task = cache.warm(ids)
    #expect(await probe.waitForEntry())
    task?.cancel()
    probe.proceed.signal()
    await task?.value
    #expect(probe.calls.withLock { $0.count } == 1)
    #expect(ids.allSatisfy { cache.take($0) == nil })
    await cache.warm([ids[1]])?.value
    #expect(cache.take(ids[1])?.threadID == ids[1])
}

@Test func prefetchRejectsOversizedDocumentsAndRetriesMissingOnes() async {
    let attempts = Mutex(0)
    let id = UUID()
    let cache = DocumentPrefetch(limit: 2, costLimit: 2, load: { id in
        let number = attempts.withLock { value in value += 1; return value }
        return number == 1 ? nil : (ThreadDocument(threadID: id), number == 2 ? 3 : 1)
    })
    await cache.warm([id])?.value
    #expect(cache.take(id) == nil)
    await cache.warm([id])?.value
    #expect(cache.take(id) == nil)
    await cache.warm([id])?.value
    #expect(cache.take(id)?.threadID == id)
}

@Test func deletingCheckpointsLeavesOtherRefsIntact() async throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent("droppy-checkpoints-\(UUID())")
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: directory) }
    let git = Git(directory.path)
    try Git.check(await git.run(["init", "-q"]))
    try Git.check(await git.run(["-c", "user.name=Test", "-c", "user.email=test@example.invalid", "commit", "--allow-empty", "-qm", "Initial"]))
    let target = UUID()
    let other = UUID()
    let ownRefs = (0..<150).map { Git.checkpointRef(thread: target, turn: $0, phase: "before") }
    let otherRef = Git.checkpointRef(thread: other, turn: 1, phase: "before")
    let input = (ownRefs + [otherRef, "refs/test/keep"]).map { "update \($0) HEAD\n" }.joined()
    try Git.check(await git.run(["update-ref", "--stdin"], input: Data(input.utf8)))
    await git.deleteCheckpoints(thread: target)
    await git.deleteCheckpoints(thread: target)
    let refs = try await git.output(["for-each-ref", "--format=%(refname)"])
    #expect(!refs.contains(target.uuidString.lowercased()))
    #expect(refs.contains(otherRef))
    #expect(refs.contains("refs/test/keep"))
}

@Suite(.serialized)
struct LibraryPerformanceTests {
    @MainActor private func makeModel(projects: [Project], threads: [ChatThread]) throws -> AppModel {
        let root = try #require(WebsiteCaptures.storageRoot)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        var library = Library()
        library.projects = projects
        library.threads = threads
        try JSONEncoder.storage.encode(library).write(to: root.appendingPathComponent("library.json"))
        return AppModel()
    }

    private func thread(in project: Project, age: Int) -> ChatThread {
        var thread = ChatThread(projectID: project.id, provider: .codex, model: nil, effort: nil, runtimeMode: .supervised)
        thread.createdAt = Date(timeIntervalSince1970: Double(1_000 - age))
        thread.updatedAt = thread.createdAt
        return thread
    }

    @Test @MainActor func firstDeltaAfterALullShowsAtOnceAndTheRestCoalesce() throws {
        let project = Project(name: "Stream", path: "/tmp/droppy-stream-project")
        let thread = thread(in: project, age: 0)
        let model = try makeModel(projects: [project], threads: [thread])
        model.settings.hydraQueueHeads = false
        model.settings.hydraAlwaysHeads = false
        let runtime = model.runtime(for: thread.id)
        runtime.rehearseTurn("Question")
        func shown() -> String? {
            guard let entry = runtime.entries.first(where: { $0.item.id == "reply" }), case .assistant(let message) = entry.item.content else { return nil }
            return message.text
        }
        runtime.rehearse(.messageDelta(id: "reply", text: "Hel"))
        // On screen before any timer: the first token of a reply.
        #expect(shown() == "Hel")
        // The next one, inside the window, waits for the flush.
        runtime.rehearse(.messageDelta(id: "reply", text: "lo"))
        #expect(shown() == "Hel")
        runtime.rehearse(.messageCompleted(id: "reply", text: "Hello"))
        #expect(shown() == "Hello")
    }

    @Test @MainActor func quitPreservesBufferedTextAndFollowUpsAndRejectsLaterSaves() async throws {
        let project = Project(name: "Quit", path: "/tmp/droppy-no-quit-project")
        let thread = thread(in: project, age: 0)
        let model = try makeModel(projects: [project], threads: [thread])
        model.settings.hydraQueueHeads = false
        model.settings.hydraAlwaysHeads = false
        let runtime = model.runtime(for: thread.id)
        await runtime.ensureLoaded()
        runtime.rehearseTurn("Question")
        runtime.saveNow()
        runtime.rehearse(.messageDelta(id: "last", text: "Final buffered text"))
        runtime.enqueueFollowUp(text: "Next question", attachments: [])
        model.saveBeforeQuit()
        let url = Storage.threadURL(thread.id)
        let saved = try Data(contentsOf: url)
        let document = try JSONDecoder.storage.decode(ThreadDocument.self, from: saved)
        let item = try #require(document.items.first { $0.id == "last" })
        guard case .assistant(let message) = item.content else {
            Issue.record("The final assistant message was not saved")
            return
        }
        #expect(message.text == "Final buffered text")
        #expect(document.followUps.map(\.text) == ["Next question"])
        runtime.rehearse(.messageCompleted(id: "last", text: "Late callback"))
        runtime.saveNow()
        model.saveBeforeQuit()
        let barrier = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: barrier) }
        DiskWriter.shared.writeSynchronously("barrier", to: barrier)
        #expect(try Data(contentsOf: url) == saved)
    }

    @Test @MainActor func idleRuntimeIsReleasedAndReadsBackWhatItSaved() async throws {
        let project = Project(name: "Idle", path: "/tmp/droppy-no-idle-project")
        let thread = thread(in: project, age: 0)
        let kept = self.thread(in: project, age: 1)
        let model = try makeModel(projects: [project], threads: [thread, kept])
        model.settings.hydraQueueHeads = false
        model.settings.hydraAlwaysHeads = false
        model.selectedThreadID = kept.id
        let runtime = model.runtime(for: thread.id)
        runtime.rehearseTurn("Question")
        runtime.rehearse(.messageCompleted(id: "reply", text: "Answer"))
        // Its finish is still under way: the runtime stays until the turn is over.
        runtime.rehearse(.turnCompleted(status: .completed, error: nil))
        #expect(!model.releaseIdleRuntime(thread.id))
        let deadline = ContinuousClock.now + .seconds(5)
        while runtime.isRunning, ContinuousClock.now < deadline { try await Task.sleep(for: .milliseconds(5)) }
        #expect(!runtime.isRunning)
        runtime.enqueueFollowUp(text: "Later", attachments: [])
        let snapshot = runtime.snapshotForPersistence()
        // With a draft, or selected, the runtime stays.
        runtime.draft = ComposerDraft(text: "typing")
        #expect(!model.releaseIdleRuntime(thread.id))
        runtime.draft = ComposerDraft()
        #expect(!model.releaseIdleRuntime(kept.id))
        #expect(model.releaseIdleRuntime(thread.id))
        #expect(model.existingRuntime(for: thread.id) == nil)
        let reloaded = model.runtime(for: thread.id)
        await reloaded.ensureLoaded()
        #expect(reloaded !== runtime)
        // Dates lose their fraction in the file; ids and content come back as they went.
        let reloadedItems = reloaded.snapshotForPersistence().items
        #expect(reloadedItems.map(\.id) == snapshot.items.map(\.id))
        #expect(reloadedItems.map(\.content) == snapshot.items.map(\.content))
        #expect(reloaded.followUps.map(\.text) == ["Later"])
        #expect(reloaded.unsavedSnapshot() == nil)
    }

    @Test @MainActor func quitWritesOnlyThreadsThatChangedSinceTheirLastSave() async throws {
        let project = Project(name: "Quit", path: "/tmp/droppy-no-quit-project")
        let dirty = thread(in: project, age: 0)
        let clean = thread(in: project, age: 1)
        let model = try makeModel(projects: [project], threads: [dirty, clean])
        let untouched = model.runtime(for: clean.id)
        await untouched.ensureLoaded()
        untouched.rehearseTurn("Question")
        untouched.rehearse(.turnCompleted(status: .completed, error: nil))
        // The turn's end lands on its own task; the save below must see it.
        let deadline = ContinuousClock.now + .seconds(5)
        while untouched.isRunning, ContinuousClock.now < deadline { try await Task.sleep(for: .milliseconds(5)) }
        untouched.saveNow()
        DiskWriter.shared.waitForQueuedWrites()
        #expect(untouched.unsavedSnapshot() == nil)
        let cleanURL = Storage.threadURL(clean.id)
        let before = try Data(contentsOf: cleanURL)
        let attributes = try FileManager.default.attributesOfItem(atPath: cleanURL.path)
        let changed = model.runtime(for: dirty.id)
        await changed.ensureLoaded()
        changed.rehearseTurn("Question")
        #expect(changed.unsavedSnapshot() != nil)
        model.saveBeforeQuit()
        #expect(try Data(contentsOf: cleanURL) == before)
        #expect(try FileManager.default.attributesOfItem(atPath: cleanURL.path)[.modificationDate] as? Date == attributes[.modificationDate] as? Date)
        #expect(FileManager.default.fileExists(atPath: Storage.threadURL(dirty.id).path))
    }

    @Test @MainActor func deletedRuntimeCannotRecreateItsDocument() throws {
        let project = Project(name: "Delete", path: "/tmp/droppy-no-delete-project")
        let thread = thread(in: project, age: 0)
        let model = try makeModel(projects: [project], threads: [thread])
        let runtime = model.runtime(for: thread.id)
        runtime.rehearseTurn("Question")
        runtime.saveNow()
        model.delete(thread.id)
        runtime.rehearse(.messageCompleted(id: "late", text: "Late callback"))
        runtime.saveNow()
        let barrier = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: barrier) }
        DiskWriter.shared.writeSynchronously("barrier", to: barrier)
        #expect(!FileManager.default.fileExists(atPath: Storage.threadURL(thread.id).path))
    }

    @Test @MainActor func indexesPreserveProjectHelperAndSidebarOrderAfterMutation() throws {
        let project = Project(name: "First", path: "/tmp/droppy-no-project")
        let other = Project(name: "Second", path: "/tmp/droppy-no-other-project")
        let parent = thread(in: project, age: 10)
        var pinned = thread(in: project, age: 20)
        pinned.isPinned = true
        var settled = thread(in: project, age: 0)
        settled.isSettled = true
        var helper = thread(in: project, age: 1)
        helper.parentThreadID = parent.id
        var panel = thread(in: project, age: 2)
        panel.parentThreadID = parent.id
        panel.isInPanel = true
        var archived = thread(in: project, age: 3)
        archived.parentThreadID = parent.id
        archived.isArchived = true
        var orphan = thread(in: project, age: 30)
        orphan.parentThreadID = UUID()
        let foreign = thread(in: other, age: 0)
        let model = try makeModel(projects: [project, other], threads: [parent, settled, helper, pinned, panel, archived, orphan, foreign])
        #expect(model.threads(in: project).map(\.id) == [pinned.id, parent.id, orphan.id, settled.id])
        #expect(model.helpers(of: parent.id).map(\.id) == [helper.id])
        #expect(model.subagent(of: parent.id)?.id == panel.id)
        #expect(model.children(of: parent.id).map(\.id) == [helper.id, panel.id, archived.id])
        #expect(model.hydraHeads(of: parent.id).isEmpty)
        // Helpers sit folded under their chat until it is unfolded.
        #expect(model.sidebarThreads.map(\.id) == [pinned.id, parent.id, orphan.id, settled.id, foreign.id])
        model.updateThread(parent.id) { $0.foldsHelpers = false }
        #expect(model.sidebarThreads.map(\.id) == [pinned.id, parent.id, helper.id, orphan.id, settled.id, foreign.id])
        model.updateThread(helper.id) { $0.parentThreadID = pinned.id }
        #expect(model.helpers(of: parent.id).isEmpty)
        #expect(model.helpers(of: pinned.id).map(\.id) == [helper.id])
        model.updateThread(pinned.id) { $0.foldsHelpers = true }
        #expect(model.sidebarThreads.map(\.id) == [pinned.id, parent.id, orphan.id, settled.id, foreign.id])
        model.updateThread(orphan.id) { $0.projectID = other.id }
        #expect(model.threads(in: project).map(\.id) == [pinned.id, parent.id, settled.id])
        #expect(model.threads(in: other).map(\.id) == [foreign.id, orphan.id])
    }

    @Test @MainActor func largeReordersPreserveValuesAndAssignEveryPeerOnce() throws {
        let project = Project(name: "Reorder", path: "/tmp/droppy-no-project")
        let original = (0..<1_000).map { thread(in: project, age: $0) }
        let model = try makeModel(projects: [project], threads: original)
        let first = try #require(original.first)
        let last = try #require(original.last)
        #expect(model.moveThread(first.id, to: last.id, placeAfter: true))
        let expected = Array(original.dropFirst()) + [first]
        #expect(model.threads(in: project).map(\.id) == expected.map(\.id))
        for (position, thread) in expected.enumerated() {
            var wanted = thread
            wanted.sortOrder = Double(position)
            #expect(model.thread(thread.id) == wanted)
        }
        let peers = expected.map(\.id)
        #expect(model.moveInActivity(first.id, to: last.id, placeAfter: false, among: peers))
        let reordered = Array(expected.dropLast(2)) + [first, last]
        for (position, thread) in reordered.enumerated() {
            let actual = try #require(model.thread(thread.id))
            #expect(actual.activityOrder == Double(position))
            #expect(actual.activityOrderDay == Calendar.current.startOfDay(for: actual.updatedAt))
            #expect(actual.title == thread.title)
            #expect(actual.createdAt == thread.createdAt)
        }
    }
}

@MainActor
private final class AttachmentBox {
    var attachments: [DroppyCode.Attachment] = []
    var binding: Binding<[DroppyCode.Attachment]> {
        Binding(get: { self.attachments }, set: { self.attachments = $0 })
    }
}

private func writeTemporaryFiles(_ names: [String], bytes: Int = 16) throws -> [URL] {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    return try names.map { name in
        let url = directory.appendingPathComponent(name)
        try Data(repeating: 0x41, count: bytes).write(to: url)
        return url
    }
}

private func pngBytes(width: Int, height: Int, rotated: Bool) throws -> Data {
    let context = try #require(CGContext(
        data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0,
        space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
    ))
    context.setFillColor(CGColor(red: 0.2, green: 0.4, blue: 0.9, alpha: 1))
    context.fill(CGRect(x: 0, y: 0, width: width, height: height))
    let image = try #require(context.makeImage())
    let data = NSMutableData()
    let destination = try #require(CGImageDestinationCreateWithData(data, UTType.png.identifier as CFString, 1, nil))
    // Orientation 6 is a quarter turn: a viewer shows the image with width and height swapped.
    let properties: [CFString: Any] = rotated ? [kCGImagePropertyOrientation: 6] : [:]
    CGImageDestinationAddImage(destination, image, properties as CFDictionary)
    #expect(CGImageDestinationFinalize(destination))
    return data as Data
}

private func storedAttachmentFiles(withSuffix suffix: String) throws -> [String] {
    try FileManager.default.contentsOfDirectory(atPath: Storage.attachmentsDirectory.path).filter { $0.hasSuffix(suffix) }
}

@Test @MainActor func attachImportsInOrderAndStopsAtTheLimit() async throws {
    let box = AttachmentBox()
    let urls = try writeTemporaryFiles((0..<9).map { "file-\($0).txt" })
    let batch = try #require(Storage.attach(urls.map(AttachmentSource.file), to: box.binding))
    #expect(box.attachments.isEmpty)
    await batch.value
    #expect(box.attachments.map(\.name) == (0..<8).map { "file-\($0).txt" })
    for attachment in box.attachments {
        #expect(FileManager.default.fileExists(atPath: attachment.path))
        #expect(attachment.mimeType == "text/plain")
    }
    // With every slot taken nothing starts.
    #expect(Storage.attach([.file(urls[8])], to: box.binding) == nil)
    #expect(box.attachments.count == 8)
}

@Test @MainActor func attachLandsLaterAndDropsWhatNoLongerFits() async throws {
    let box = AttachmentBox()
    let large = try writeTemporaryFiles(["big.bin"], bytes: 64 * 1_024 * 1_024)
    let batch = try #require(Storage.attach([.file(large[0])], to: box.binding))
    // The call returns with nothing attached: the copy is still to come.
    #expect(box.attachments.isEmpty)
    // The slots fill while the copy runs: the copy lands past the limit and is removed again.
    let small = try writeTemporaryFiles((0..<8).map { "s\($0).txt" })
    box.attachments = small.map { DroppyCode.Attachment(name: $0.lastPathComponent, path: $0.path, mimeType: "text/plain") }
    await batch.value
    #expect(box.attachments.map(\.name) == small.map(\.lastPathComponent))
    #expect(try storedAttachmentFiles(withSuffix: ".bin").isEmpty)
}

@Test @MainActor func pastedImagesKeepPNGBytesAndTranscodeTheRestUpright() async throws {
    let png = try pngBytes(width: 30, height: 20, rotated: false)
    let attachment = try #require(Storage.importSource(.image(png, .png)))
    #expect(attachment.name == "Pasted image.png")
    #expect(try Data(contentsOf: attachment.url) == png)
    // A rotated image pasted as another type comes out as an upright PNG.
    let rotated = try pngBytes(width: 30, height: 20, rotated: true)
    let tiff = try #require(NSBitmapImageRep(data: rotated)?.tiffRepresentation)
    let source = try #require(Storage.importSource(.image(tiff, .tiff)))
    let stored = try #require(CGImageSourceCreateWithURL(source.url as CFURL, nil))
    #expect(CGImageSourceGetType(stored) as String? == UTType.png.identifier)
    let properties = try #require(CGImageSourceCopyPropertiesAtIndex(stored, 0, nil) as? [CFString: Any])
    #expect(properties[kCGImagePropertyOrientation] == nil)
    #expect(properties[kCGImagePropertyPixelWidth] as? Int == 20)
    #expect(properties[kCGImagePropertyPixelHeight] as? Int == 30)
}

@Test @MainActor func heicPhotosBecomeUprightJPEGs() async throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    let heic = directory.appendingPathComponent("photo.HEIC")
    let context = try #require(CGContext(
        data: nil, width: 40, height: 24, bitsPerComponent: 8, bytesPerRow: 0,
        space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue
    ))
    context.setFillColor(CGColor(red: 0.9, green: 0.3, blue: 0.1, alpha: 1))
    context.fill(CGRect(x: 0, y: 0, width: 40, height: 24))
    let destination = try #require(CGImageDestinationCreateWithURL(heic as CFURL, UTType.heic.identifier as CFString, 1, nil))
    CGImageDestinationAddImage(destination, try #require(context.makeImage()), [kCGImagePropertyOrientation: 6] as CFDictionary)
    #expect(CGImageDestinationFinalize(destination))
    let attachment = try #require(Storage.importSource(.file(heic)))
    #expect(attachment.name == "photo.jpg")
    #expect(attachment.mimeType == "image/jpeg")
    let stored = try #require(CGImageSourceCreateWithURL(attachment.url as CFURL, nil))
    #expect(CGImageSourceGetType(stored) as String? == UTType.jpeg.identifier)
    let properties = try #require(CGImageSourceCopyPropertiesAtIndex(stored, 0, nil) as? [CFString: Any])
    #expect(properties[kCGImagePropertyPixelWidth] as? Int == 24)
    #expect(properties[kCGImagePropertyPixelHeight] as? Int == 40)
    #expect(Storage.importSource(.file(directory)) == nil)
}

@Test @MainActor func tokenLedgerWritesLiveSpendAfterALullAndOnFlush() async throws {
    let ledger = TokenLedger.shared
    let defaults = try #require(WebsiteCaptures.defaults)
    let key = "droppycode.tokenActivity.liveDaily"
    func stored() -> Int { ((defaults.dictionary(forKey: key) as? [String: Int]) ?? [:]).values.reduce(0, +) }
    let day = Date()
    let before = stored()
    let total = ledger.dailyTotals.values.reduce(0, +)
    for _ in 0..<100 { ledger.record(spend: 3, on: day) }
    #expect(ledger.dailyTotals.values.reduce(0, +) == total + 300)
    // A hundred usage events write nothing while they arrive.
    #expect(stored() == before)
    ledger.flushLiveSpend()
    #expect(stored() == before + 300)
    // With nothing due a flush is a no-op, and the next record schedules its own write.
    ledger.flushLiveSpend()
    ledger.record(spend: 7, on: day)
    #expect(stored() == before + 300)
    let deadline = ContinuousClock.now + .seconds(5)
    while stored() != before + 307, ContinuousClock.now < deadline {
        try await Task.sleep(for: .milliseconds(100))
    }
    #expect(stored() == before + 307)
}

@main struct StoragePerformanceTests {
    static func main() async {
        // Finished turns ask the app whether it is active; no window is ever shown.
        await MainActor.run { _ = NSApplication.shared.setActivationPolicy(.prohibited) }
        let result: CInt = await Testing.__swiftPMEntryPoint()
        exit(result)
    }
}
