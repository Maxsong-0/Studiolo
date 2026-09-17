import CoreServices
import Foundation
import Synchronization

/// Whether anything under a working directory has been written since a snapshot of it
/// was taken, from an FSEvents stream over the directory. One per directory, shared by
/// every thread working there.
///
/// A shell command's row gets its edits from two working-tree snapshots, and a turn gets
/// a checkpoint at each end; every one is `git add -A` over the whole tree, 46 ms on a
/// 4k-file repository and 200 ms at 60k, and most commands only read. The stream vouches
/// that nothing was written since the last snapshot, so the last tree is the current tree
/// and no git runs.
///
/// Events reach the stream a little after the write, and a flush only delivers what the
/// service already holds, so before trusting the count a cookie file is written in the
/// git directory and the answer waits for the cookie's own event: events arrive in
/// order, so everything written before it has been counted. Anything the stream could
/// have missed, dropped events, a cookie that never arrives or a volume that reports
/// nothing, means a fresh snapshot: the diff, and so a revert, never rests on the stream
/// alone.
final class WorkingTreeWatch: Sendable {
    /// The index file as last seen: its content is part of what `write-tree` writes for
    /// paths the snapshot does not walk, as when the user stages files outside the directory.
    private struct IndexStamp: Equatable {
        var modified: Date?
        var size: Int?

        init(path: String) {
            let attributes = try? FileManager.default.attributesOfItem(atPath: path)
            modified = attributes?[.modificationDate] as? Date
            size = attributes?[.size] as? Int
        }
    }

    private struct Snapshot {
        var tree: String
        var generation: UInt64
        var index: IndexStamp
        /// The checkpoint commit made of this tree, if one was, so a turn starting on an
        /// unchanged tree costs an `update-ref` and nothing else.
        var commit: String?
    }

    private struct State {
        /// Bumped by every event outside the git directory and by any event the stream may have lost.
        var generation: UInt64 = 1
        /// The paths, relative to the root, with a change since the last capture began: the
        /// directory holding each changed entry, or the entry itself at the root. Nil once
        /// anything may have been missed, when the whole tree is walked. Past `changedLimit`
        /// entries they collapse to their top-level directory, so a build writing thousands
        /// of files costs one walk of its output.
        var changed: Set<String>? = []
        /// Cookies written and the highest one whose event has arrived.
        var cookiesWritten: UInt64 = 0
        var cookieSeen: UInt64 = 0
        var snapshot: Snapshot?
        /// A capture under way and the generation it started at, for a caller at the same
        /// generation to share instead of walking the tree again.
        var capture: (generation: UInt64, task: Task<String?, Never>)?
        var stream: Stream?
        /// Callers waiting for a cookie's event, resumed by `note` when it lands or by
        /// their own timeout, whichever removes them first.
        var waiters: [(id: UInt64, cookie: UInt64, continuation: CheckedContinuation<Bool, Never>)] = []
        var waitersAdded: UInt64 = 0
    }

    /// The stream handle. FSEvents streams may be started, flushed, stopped and invalidated
    /// from any thread; the handle itself never changes, only the stream behind it.
    private struct Stream: @unchecked Sendable {
        let ref: FSEventStreamRef
    }

    private enum Lookup {
        case tree(String)
        case join(Task<String?, Never>)
        case own(Task<String?, Never>)
        /// A capture at another generation holds the scratch index; wait for it and ask again.
        case wait(Task<String?, Never>)
    }

    private static let changedLimit = 64

    /// Watches by directory, oldest first. Bounded by the directories a session works in;
    /// past the bound the oldest is torn down on its own queue, after any callback in flight.
    private static let watches = Mutex<[(directory: String, watch: WorkingTreeWatch)]>([])
    private static let limit = 32

    /// The watch for a directory on a local volume, with its git directory (a worktree's
    /// lies outside it) for the cookies; nil where FSEvents cannot vouch for the files, so
    /// every snapshot is taken fresh.
    static func shared(for directory: String, gitDirectory: String) -> WorkingTreeWatch? {
        if let existing = watches.withLock({ $0.first { $0.directory == directory }?.watch }) { return existing }
        guard (try? URL(fileURLWithPath: directory).resourceValues(forKeys: [.volumeIsLocalKey]))?.volumeIsLocal == true else {
            return nil
        }
        let watch = WorkingTreeWatch(directory: directory, gitDirectory: gitDirectory)
        let evicted: WorkingTreeWatch? = watches.withLock { watches in
            if let existing = watches.first(where: { $0.directory == directory })?.watch { return existing }
            watches.append((directory, watch))
            return watches.count > limit ? watches.removeFirst().watch : nil
        }
        if let evicted, evicted !== watch { evicted.tearDown() }
        return watches.withLock { $0.first { $0.directory == directory }?.watch }
    }

    let directory: String
    /// The root as events name it, the git directory likewise, and the cookie files' path
    /// up to their number.
    private let rootPrefix: String
    private let gitPrefix: String
    private let cookiePrefix: String
    /// Their lengths, once: a build reports thousands of paths per callback.
    private let rootPrefixBytes: Int
    private let cookiePrefixCharacters: Int
    /// The index the captures build on, kept between them so a capture walks only what changed.
    private let scratch = FileManager.default.temporaryDirectory.appendingPathComponent("droppy-code-index-\(UUID().uuidString)")
    private let queue = DispatchQueue(label: "droppy.working-tree-watch", qos: .utility)
    private let state = Mutex(State())

    /// Events name the real path, /var as /private/var. Foundation's symlink resolution
    /// strips /private again, so this is realpath(3).
    private static func real(_ path: String) -> String {
        path.withCString { path in realpath(path, nil).map { String(cString: $0) } } ?? path
    }

    private init(directory: String, gitDirectory: String) {
        self.directory = directory
        let root = Self.real(directory)
        let gitRoot = Self.real(gitDirectory)
        rootPrefix = root + "/"
        gitPrefix = gitRoot + "/"
        cookiePrefix = gitRoot + "/droppy-code-sync-"
        rootPrefixBytes = rootPrefix.utf8.count
        cookiePrefixCharacters = cookiePrefix.count
        var context = FSEventStreamContext(
            version: 0, info: Unmanaged.passUnretained(self).toOpaque(), retain: nil, release: nil, copyDescription: nil
        )
        let flags = FSEventStreamCreateFlags(kFSEventStreamCreateFlagUseCFTypes | kFSEventStreamCreateFlagNoDefer | kFSEventStreamCreateFlagFileEvents)
        let roots = gitRoot.hasPrefix(root + "/") ? [root] : [root, gitRoot]
        guard let stream = FSEventStreamCreate(
            nil, Self.callback, &context, roots as CFArray, FSEventStreamEventId(kFSEventStreamEventIdSinceNow), 0, flags
        ) else { return }
        FSEventStreamSetDispatchQueue(stream, queue)
        if FSEventStreamStart(stream) {
            let handle = Stream(ref: stream)
            state.withLock { $0.stream = handle }
        } else {
            FSEventStreamInvalidate(stream)
            FSEventStreamRelease(stream)
        }
    }

    /// Stops the stream on its own queue, so no callback can be running when the stream
    /// goes and the watch is released after it.
    private func tearDown() {
        guard let stream = state.withLock({ state in
            defer { state.stream = nil }
            return state.stream
        }) else { return }
        queue.async {
            FSEventStreamStop(stream.ref)
            FSEventStreamInvalidate(stream.ref)
            FSEventStreamRelease(stream.ref)
            try? FileManager.default.removeItem(at: self.scratch)
        }
    }

    private static let callback: FSEventStreamCallback = { _, info, count, eventPaths, eventFlags, _ in
        guard let info else { return }
        let watch = Unmanaged<WorkingTreeWatch>.fromOpaque(info).takeUnretainedValue()
        let paths = Unmanaged<CFArray>.fromOpaque(eventPaths).takeUnretainedValue() as? [String] ?? []
        watch.note(paths: paths, flags: UnsafeBufferPointer(start: eventFlags, count: count))
    }

    private func note(paths: [String], flags: UnsafeBufferPointer<FSEventStreamEventFlags>) {
        let lost = FSEventStreamEventFlags(kFSEventStreamEventFlagMustScanSubDirs | kFSEventStreamEventFlagKernelDropped | kFSEventStreamEventFlagUserDropped)
        var changed = false
        var everything = false
        var directories: [String] = []
        var cookie: UInt64 = 0
        for (index, path) in paths.enumerated() {
            if index < flags.count, flags[index] & lost != 0 {
                changed = true
                everything = true
                continue
            }
            if path.hasPrefix(cookiePrefix) {
                cookie = max(cookie, UInt64(path.dropFirst(cookiePrefixCharacters)) ?? 0)
                continue
            }
            // Git's own writes (objects, refs, the index) do not change the working files.
            if path.hasPrefix(gitPrefix) { continue }
            changed = true
            // The directory holding what changed, relative to the root, or the entry itself
            // when it sits at the root; a change to the root, or outside it, means the whole tree.
            guard path.hasPrefix(rootPrefix), path.utf8.count > rootPrefixBytes else {
                everything = true
                continue
            }
            let relative = path[rootPrefix.endIndex...]
            directories.append(String(relative.lastIndex(of: "/").map { relative[..<$0] } ?? relative))
        }
        guard changed || cookie > 0 else { return }
        let ready: [CheckedContinuation<Bool, Never>] = state.withLock { state in
            if changed { state.generation &+= 1 }
            state.cookieSeen = max(state.cookieSeen, cookie)
            if everything {
                state.changed = nil
            } else if var set = state.changed {
                for directory in directories { set.insert(set.count >= Self.changedLimit ? Self.topLevel(directory) : directory) }
                if set.count > Self.changedLimit { set = Set(set.map(Self.topLevel)) }
                state.changed = set
            }
            let seen = state.cookieSeen
            guard state.waiters.contains(where: { $0.cookie <= seen }) else { return [] }
            let ready = state.waiters.filter { $0.cookie <= seen }.map(\.continuation)
            state.waiters.removeAll { $0.cookie <= seen }
            return ready
        }
        // Outside the lock: a resumed caller takes it next.
        for continuation in ready { continuation.resume(returning: true) }
    }

    /// The changed paths sorted, without those inside another of them.
    private static func pathspecs(_ changed: Set<String>) -> [String] {
        var result: [String] = []
        for path in changed.sorted() where !(result.last.map { path.hasPrefix($0 + "/") } ?? false) {
            result.append(path)
        }
        return result
    }

    private static func topLevel(_ directory: String) -> String {
        directory.firstIndex(of: "/").map { String(directory[..<$0]) } ?? directory
    }

    /// The generation once every write that has already happened has been counted: a
    /// cookie is written in the git directory and the count is read when its event has
    /// arrived, as everything written before it has by then. Nil without a stream, when
    /// nothing can be reused. A cookie that never arrives means the volume is not
    /// reporting; the stream is torn down and every later snapshot is taken fresh, at once.
    private func settledGeneration() async -> UInt64? {
        guard state.withLock({ $0.stream }) != nil else { return nil }
        let (cookie, waiter) = state.withLock { state -> (UInt64, UInt64) in
            state.cookiesWritten += 1
            state.waitersAdded += 1
            return (state.cookiesWritten, state.waitersAdded)
        }
        let path = cookiePrefix + String(cookie)
        guard FileManager.default.createFile(atPath: path, contents: nil) else { return nil }
        defer { try? FileManager.default.removeItem(atPath: path) }
        // The event wakes the waiter; nothing polls. Whoever removes the waiter, the event
        // or the timeout, is the one that resumes it.
        let timeout = Task { [weak self] in
            try? await Task.sleep(for: .seconds(1))
            self?.expireWaiter(waiter)
        }
        let arrived = await withCheckedContinuation { (continuation: CheckedContinuation<Bool, Never>) in
            let seen = state.withLock { state -> Bool in
                if state.cookieSeen >= cookie { return true }
                state.waiters.append((waiter, cookie, continuation))
                return false
            }
            if seen { continuation.resume(returning: true) }
        }
        timeout.cancel()
        guard arrived else { tearDown(); return nil }
        return state.withLock { $0.generation }
    }

    private func expireWaiter(_ id: UInt64) {
        let expired = state.withLock { state -> CheckedContinuation<Bool, Never>? in
            guard let index = state.waiters.firstIndex(where: { $0.id == id }) else { return nil }
            return state.waiters.remove(at: index).continuation
        }
        expired?.resume(returning: false)
    }

    /// The tree of the directory as it stands: the last one captured when nothing has
    /// been written since, else a fresh capture, shared with anyone asking at the same
    /// moment. `indexPath` is the checkout's index file, part of the tree for paths the
    /// capture does not walk.
    /// `capture` is handed the scratch index and, when the last capture's index can be
    /// built on, the directories changed since it; nil means the whole tree.
    /// `scratch` nil means an index of the capture's own, used once and thrown away.
    func currentTree(
        indexPath: String,
        capture: @escaping @Sendable (_ scratch: URL?, _ paths: [String]?) async throws -> String
    ) async throws -> String {
        // Without a stream nothing may be reused, and the scratch index is not ours to
        // share: the caller walks the tree into an index of its own.
        guard let generation = await settledGeneration() else { return try await capture(nil, nil) }
        let index = IndexStamp(path: indexPath)
        while true {
            let lookup: Lookup = state.withLock { state in
                if let snapshot = state.snapshot, snapshot.generation == generation, snapshot.index == index {
                    return .tree(snapshot.tree)
                }
                if let capture = state.capture {
                    return capture.generation == generation ? .join(capture.task) : .wait(capture.task)
                }
                // The changed directories are taken here: what changes during the walk lands
                // in a fresh set, for the capture after this one.
                let paths = state.snapshot?.index == index ? state.changed.map(Self.pathspecs) : nil
                state.changed = []
                let task = Task.detached(priority: .utility) { [scratch] in try? await capture(scratch, paths) }
                state.capture = (generation, task)
                return .own(task)
            }
            switch lookup {
            case .tree(let tree):
                return tree
            case .join(let task):
                // A failed capture leaves the scratch index to be rebuilt, which only the
                // owner may do: ask again rather than walking it from here.
                if let tree = await task.value { return tree }
            case .wait(let task):
                _ = await task.value
            case .own(let task):
                let tree = await task.value
                state.withLock { state in
                    if state.capture?.task == task { state.capture = nil }
                    if let tree {
                        state.snapshot = Snapshot(tree: tree, generation: generation, index: index, commit: nil)
                    } else {
                        state.changed = nil
                    }
                }
                guard let tree else { throw CocoaError(.fileReadUnknown) }
                return tree
            }
        }
    }

    /// The checkpoint commit already made of `tree`, if the last snapshot is that tree.
    func commit(of tree: String) -> String? {
        state.withLock { state in
            guard let snapshot = state.snapshot, snapshot.tree == tree else { return nil }
            return snapshot.commit
        }
    }

    func remember(commit: String, of tree: String) {
        state.withLock { state in
            guard state.snapshot?.tree == tree else { return }
            state.snapshot?.commit = commit
        }
    }
}
