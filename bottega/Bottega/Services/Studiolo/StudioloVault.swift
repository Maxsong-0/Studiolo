// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import CoreServices
import Foundation
import Synchronization

/// The project archives of a Studiolo vault on this Mac, read from `项目/*/00-总览/INDEX.md`
/// and read again whenever anything under `项目/` changes, including what Obsidian Sync
/// brings in from another device.
///
/// Only the dashboards are read; logs, tasks and bug files never are. The whole folder is
/// read again on every change rather than working out which INDEX moved: two dozen small
/// files take a millisecond, and a rescan cannot miss a rename the way path matching can.
/// Listeners hear only when the result differs, so an agent writing a log every few
/// seconds costs a rescan and nothing on screen.
final class StudioloVault: Sendable {
    let root: URL

    /// The stream handle; FSEvents streams may be stopped and released from any thread.
    private struct Stream: @unchecked Sendable {
        let ref: FSEventStreamRef
    }

    private struct State {
        var stream: Stream?
        var last: [StudioloArchive]?
    }

    private let onChange: @Sendable ([StudioloArchive]) -> Void
    private let queue = DispatchQueue(label: "bottega.studiolo-vault", qos: .utility)
    private let state = Mutex(State())

    /// `onChange` is called on a private queue with the archives, sorted by name, once at
    /// `start()` and then after each change that alters them.
    init(root: URL, onChange: @escaping @Sendable ([StudioloArchive]) -> Void) {
        self.root = root
        self.onChange = onChange
    }

    /// Whether the folder looks like a Studiolo vault: it has the projects folder.
    static func isVault(_ root: URL) -> Bool {
        var isDirectory: ObjCBool = false
        let projects = root.appendingPathComponent(StudioloIndex.projectsFolder)
        return FileManager.default.fileExists(atPath: projects.path, isDirectory: &isDirectory) && isDirectory.boolValue
    }

    /// Every folder under `项目/`, hidden ones aside, with its dashboard when it has one.
    ///
    /// A symbolic link is no archive, even one to a folder. Studiolo's own vault scan skips
    /// links (`crates/vault-broker/src/readonly/scan.rs`), and followed here, a link to
    /// another archive listed that archive twice and a link out of the vault showed a
    /// folder as an archive Studiolo has never heard of. So each entry is judged as it is:
    /// resource values describe the link itself, not what it points to.
    static func scan(root: URL) -> [StudioloArchive] {
        let projects = root.appendingPathComponent(StudioloIndex.projectsFolder)
        let names = (try? FileManager.default.contentsOfDirectory(atPath: projects.path)) ?? []
        return names
            .filter { !$0.hasPrefix(".") }
            .filter { name in
                let entry = try? projects.appendingPathComponent(name)
                    .resourceValues(forKeys: [.isDirectoryKey, .isSymbolicLinkKey])
                return entry?.isDirectory == true && entry?.isSymbolicLink != true
            }
            .sorted { $0.localizedStandardCompare($1) == .orderedAscending }
            .map { name in
                let index = projects.appendingPathComponent(name).appendingPathComponent(StudioloIndex.indexFile)
                let text = try? String(contentsOf: index, encoding: .utf8)
                return StudioloArchive(name: name, card: text.map(StudioloIndex.parse))
            }
    }

    /// Starts watching the vault, then reads it once. Without a projects folder there is
    /// nothing to watch: the listener hears an empty list and no stream is made.
    func start() {
        // The first read goes on the stream's own queue after the stream has started: a
        // change landing between the read and the start would otherwise never be heard
        // (the stream reports only what happens once it runs), and on one serial queue the
        // read still comes before any event.
        defer { queue.async { self.rescan() } }
        guard Self.isVault(root), state.withLock({ $0.stream == nil }) else { return }
        // The stream holds the vault through `info` until it is released, so a callback
        // can never reach a vault that has gone; `stop()` is what lets it go.
        var context = FSEventStreamContext(
            version: 0,
            info: Unmanaged.passUnretained(self).toOpaque(),
            retain: { info in
                guard let info else { return nil }
                _ = Unmanaged<StudioloVault>.fromOpaque(info).retain()
                return info
            },
            release: { info in
                guard let info else { return }
                Unmanaged<StudioloVault>.fromOpaque(info).release()
            },
            copyDescription: nil
        )
        let watched = [root.appendingPathComponent(StudioloIndex.projectsFolder).path] as CFArray
        let flags = FSEventStreamCreateFlags(kFSEventStreamCreateFlagUseCFTypes | kFSEventStreamCreateFlagWatchRoot)
        guard let stream = FSEventStreamCreate(
            nil, Self.callback, &context, watched, FSEventStreamEventId(kFSEventStreamEventIdSinceNow), 0.5, flags
        ) else { return }
        FSEventStreamSetDispatchQueue(stream, queue)
        if FSEventStreamStart(stream) {
            state.withLock { $0.stream = Stream(ref: stream) }
        } else {
            FSEventStreamInvalidate(stream)
            FSEventStreamRelease(stream)
        }
    }

    /// Stops watching, on the stream's own queue so no callback is mid-flight when it goes.
    func stop() {
        guard let stream = state.withLock({ state in
            defer { state.stream = nil }
            return state.stream
        }) else { return }
        queue.async {
            FSEventStreamStop(stream.ref)
            FSEventStreamInvalidate(stream.ref)
            FSEventStreamRelease(stream.ref)
        }
    }

    private static let callback: FSEventStreamCallback = { _, info, _, _, _, _ in
        guard let info else { return }
        Unmanaged<StudioloVault>.fromOpaque(info).takeUnretainedValue().rescan()
    }

    /// Runs on `queue` only, so two rescans never race to be the last word.
    private func rescan() {
        let archives = Self.scan(root: root)
        let changed = state.withLock { state in
            defer { state.last = archives }
            return state.last != archives
        }
        if changed { onChange(archives) }
    }
}
