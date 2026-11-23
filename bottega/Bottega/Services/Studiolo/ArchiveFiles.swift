// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import CoreServices
import Foundation
import Synchronization

/// One entry of an archive's file tree: a Markdown note, or a folder holding some.
struct ArchiveFileNode: Identifiable, Hashable, Sendable {
    /// Relative to the archive folder, `/`-separated.
    let path: String
    let name: String
    let children: [ArchiveFileNode]?

    var id: String { path }
    var isFolder: Bool { children != nil }
}

/// Reading an archive's notes as files: the tree of its Markdown, the notes a wiki link
/// can mean, and the count its task board shows. Pure functions over the folder; the
/// page keeps them current with `ArchiveFolderWatch`.
enum ArchiveFiles {
    /// The vault's fixed skeleton, in the order the archive page lists it. Every other
    /// folder follows, by name.
    static let skeleton = ["00-总览", "01-框架", "02-日志", "03-记忆", "04-协调", "05-版本", "06-BUG"]
    static let tasksFile = "04-协调/TASKS.md"
    static let logsFolder = "02-日志"

    /// The archive's Markdown notes as a tree. Hidden entries and symbolic links are left
    /// out, as the vault scan leaves them out (see `StudioloVault.scan`); a folder with no
    /// note under it is left out too.
    static func tree(root: URL) -> [ArchiveFileNode] {
        nodes(in: root, relative: "")
    }

    private static func nodes(in folder: URL, relative: String) -> [ArchiveFileNode] {
        let names = (try? FileManager.default.contentsOfDirectory(atPath: folder.path)) ?? []
        var folders: [ArchiveFileNode] = []
        var files: [ArchiveFileNode] = []
        for name in names where !name.hasPrefix(".") {
            let url = folder.appendingPathComponent(name)
            guard let values = try? url.resourceValues(forKeys: [.isDirectoryKey, .isSymbolicLinkKey]), values.isSymbolicLink != true else { continue }
            let path = relative.isEmpty ? name : relative + "/" + name
            if values.isDirectory == true {
                let children = nodes(in: url, relative: path)
                if !children.isEmpty { folders.append(ArchiveFileNode(path: path, name: name, children: children)) }
            } else if name.lowercased().hasSuffix(".md") {
                files.append(ArchiveFileNode(path: path, name: name, children: nil))
            }
        }
        folders.sort { rank($0.name) < rank($1.name) || (rank($0.name) == rank($1.name) && $0.name.localizedStandardCompare($1.name) == .orderedAscending) }
        files.sort { $0.name.localizedStandardCompare($1.name) == .orderedAscending }
        return folders + files
    }

    private static func rank(_ name: String) -> Int {
        skeleton.firstIndex(of: name) ?? skeleton.count
    }

    /// Every note in the tree, in tree order.
    static func notes(in tree: [ArchiveFileNode]) -> [ArchiveFileNode] {
        tree.flatMap { node in node.children.map(notes) ?? [node] }
    }

    /// The note a wiki link means, as Obsidian would find it within this archive: the note
    /// at that very path first, else the shortest path to a note of that name, case aside.
    static func resolve(_ note: String, in tree: [ArchiveFileNode]) -> ArchiveFileNode? {
        let wanted = note.hasSuffix("/") ? String(note.dropLast()) : note
        let all = notes(in: tree)
        let target = (wanted.lowercased() + ".md")
        if let exact = all.first(where: { $0.path.lowercased() == target }) { return exact }
        let name = (wanted as NSString).lastPathComponent.lowercased() + ".md"
        return all
            .filter { $0.name.lowercased() == name }
            .min { $0.path.count < $1.path.count }
    }

    /// The newest daily log, by name: the logs are `YYYY-MM-DD.md`, with `-2` and on for a
    /// day that ran over, so the last name in order is the latest.
    static func latestLog(in tree: [ArchiveFileNode]) -> ArchiveFileNode? {
        guard let logs = tree.first(where: { $0.name == logsFolder })?.children else { return nil }
        return notes(in: logs).max { $0.name.localizedStandardCompare($1.name) == .orderedAscending }
    }

    /// How many rows the task board's "in progress" table holds: the rows of the first
    /// table under the heading that starts with `## ①`, its header and rule aside.
    static func openTaskCount(in tasks: String) -> Int {
        var inSection = false
        var rows = 0
        var tableLines = 0
        for line in tasks.split(separator: "\n", omittingEmptySubsequences: false) {
            let trimmed = line.trimmingCharacters(in: .whitespaces)
            if trimmed.hasPrefix("## ") {
                if inSection { break }
                inSection = trimmed.hasPrefix("## ①")
                continue
            }
            guard inSection, trimmed.hasPrefix("|") else {
                // The table ended; a second one in the section is not the board.
                if inSection, tableLines > 0 { break }
                continue
            }
            tableLines += 1
            // The first line is the header, the second the rule under it.
            if tableLines > 2 { rows += 1 }
        }
        return rows
    }
}

/// Watches an archive folder for anything changing under it, on a queue of its own, and
/// says so; the page reads the folder again. The stream holds the watch until it is
/// stopped, the way `StudioloVault` holds its own.
final class ArchiveFolderWatch: Sendable {
    let root: URL

    private struct Stream: @unchecked Sendable {
        let ref: FSEventStreamRef
    }

    private let onChange: @Sendable () -> Void
    private let queue = DispatchQueue(label: "bottega.archive-watch", qos: .utility)
    private let stream = Mutex<Stream?>(nil)

    init(root: URL, onChange: @escaping @Sendable () -> Void) {
        self.root = root
        self.onChange = onChange
    }

    func start() {
        guard stream.withLock({ $0 == nil }) else { return }
        var context = FSEventStreamContext(
            version: 0,
            info: Unmanaged.passUnretained(self).toOpaque(),
            retain: { info in
                guard let info else { return nil }
                _ = Unmanaged<ArchiveFolderWatch>.fromOpaque(info).retain()
                return info
            },
            release: { info in
                guard let info else { return }
                Unmanaged<ArchiveFolderWatch>.fromOpaque(info).release()
            },
            copyDescription: nil
        )
        let flags = FSEventStreamCreateFlags(kFSEventStreamCreateFlagUseCFTypes | kFSEventStreamCreateFlagWatchRoot)
        guard let stream = FSEventStreamCreate(nil, Self.callback, &context, [root.path] as CFArray, FSEventStreamEventId(kFSEventStreamEventIdSinceNow), 0.3, flags) else { return }
        FSEventStreamSetDispatchQueue(stream, queue)
        if FSEventStreamStart(stream) {
            self.stream.withLock { $0 = Stream(ref: stream) }
        } else {
            FSEventStreamInvalidate(stream)
            FSEventStreamRelease(stream)
        }
    }

    func stop() {
        guard let stream = stream.withLock({ stream in
            defer { stream = nil }
            return stream
        }) else { return }
        queue.async {
            FSEventStreamStop(stream.ref)
            FSEventStreamInvalidate(stream.ref)
            FSEventStreamRelease(stream.ref)
        }
    }

    private static let callback: FSEventStreamCallback = { _, info, _, _, _, _ in
        guard let info else { return }
        Unmanaged<ArchiveFolderWatch>.fromOpaque(info).takeUnretainedValue().onChange()
    }
}
