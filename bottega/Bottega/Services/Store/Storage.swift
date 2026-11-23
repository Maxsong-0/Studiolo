// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import AppKit
import ImageIO
import Synchronization
import SwiftUI
import UniformTypeIdentifiers

/// Just enough of a stored thread to read the files a forwarded continuation points at:
/// the sweep runs over whole files as text, so only this key is ever decoded.
private struct StoredContinuation: Decodable {
    struct Continuation: Decodable {
        var attachments: [Attachment]?

        // Declared, not synthesized: a Decodable-only type that decodes itself gets no
        // keys of its own, and the name would find the enclosing type's instead.
        private enum CodingKeys: String, CodingKey {
            case attachments
        }

        /// One by one, so a file this build cannot read does not leave the others it names
        /// unreferenced, and so up for sweeping.
        init(from decoder: Decoder) throws {
            let container = try decoder.container(keyedBy: CodingKeys.self)
            attachments = container.valuesIfPresent(.attachments)
        }
    }

    var continuation: Continuation?
}

/// Where the app keeps its library, thread histories and attachments.
enum Storage {
    static let root: URL = {
        let base = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
        let url = base.appendingPathComponent(AppInfo.storageName, isDirectory: true)
        try? FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        return url
    }()

    static let libraryURL: URL = root.appendingPathComponent("library.json")

    // Each directory is created on first use and its URL kept: the thread URL is taken on
    // every save, prefetch and delete, and a stat plus mkdir per call was pure waste.
    static let threadsDirectory: URL = directory("threads")

    static let attachmentsDirectory: URL = directory("attachments")

    /// The user's window wallpaper, one file at a time.
    static let wallpaperDirectory: URL = directory("wallpaper")

    /// The custom theme's theme.json, alone in its folder so the watch on it wakes for nothing
    /// else (see `CustomThemeStore`).
    static let themeDirectory: URL = directory("theme")

    static let worktreesDirectory: URL = stateDirectory("worktrees")

    /// Where a hand's work is kept as a patch when it would not land in the checkout.
    static let patchesDirectory: URL = stateDirectory("patches")

    /// Where every hand's report is kept as a Markdown file, one folder per lead (see
    /// `CrewReportFile`), so a report outlives the chat and the run that produced it.
    static let reportsDirectory: URL = stateDirectory("reports")

    /// Each thread's session log (see `SessionLog`), in app support with the thread files;
    /// the agent is handed its path, and its own tools read it there.
    static let sessionLogsDirectory: URL = directory("session-logs")

    private static func stateDirectory(_ name: String) -> URL {
        let url = URL(fileURLWithPath: LoginEnvironment.homeDirectory)
            .appendingPathComponent(AppInfo.stateDirectoryName + "/" + name, isDirectory: true)
        try? FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        return url
    }

    static func threadURL(_ id: UUID) -> URL {
        threadsDirectory.appendingPathComponent("\(id.uuidString).json")
    }

    private static func directory(_ name: String) -> URL {
        let url = root.appendingPathComponent(name, isDirectory: true)
        try? FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        return url
    }

    /// The library, for the app to start from and save back. No file is a first launch; one
    /// that does not read as a library is renamed aside and the app starts without it; one that
    /// cannot be read at all is left as it is, and the problem then blocks every save of the
    /// library this run (see `StoredFile`).
    static func loadLibrary() -> StoredFile.Loaded<Library> {
        StoredFile.read(libraryURL, subject: "The library of projects and threads") { data in
            try JSONDecoder.storage.decode(Library.self, from: data)
        }
    }

    /// Whether a library that did not read is still kept beside `libraryURL` (see
    /// `StoredFile.setAside`). It may name what the library in use does not, such as the
    /// copies of the checkout its hands worked in, so nothing is taken for an orphan while it
    /// is there. A folder that cannot be listed counts as holding one.
    static var keepsUnreadableLibrary: Bool {
        guard let names = try? FileManager.default.contentsOfDirectory(atPath: root.path) else { return true }
        let prefix = libraryURL.lastPathComponent + StoredFile.setAsideInfix
        return names.contains { $0.hasPrefix(prefix) }
    }

    /// Reads and decodes a thread file, for a look that saves nothing back. Safe from any
    /// thread: the decoder is made here.
    nonisolated static func decodeDocument(_ id: UUID) -> ThreadDocument? {
        readDocument(id)?.document
    }

    /// The document as the runtime wants it: a save queued for it has landed first, blank
    /// thinking blocks are gone and model prose is clean of em dashes, so the prefetch
    /// worker does that work rather than the click that opens the thread. Nil for a file that
    /// is not there or did not read, which is left exactly as it is: the runtime that opens
    /// the thread reads it again with `openDocument`, which tells those apart.
    nonisolated static func readDocument(_ id: UUID) -> (document: ThreadDocument, cost: Int)? {
        DiskWriter.shared.waitForQueuedWrites()
        guard let data = try? Data(contentsOf: threadURL(id)), let document = try? decodeThread(data) else { return nil }
        return (document, data.count)
    }

    /// The document for the runtime that shows the thread and saves it back, read as
    /// `readDocument` reads it, except that a file that did not read is never taken for an
    /// empty thread: one that is there but does not read as a thread is renamed aside and the
    /// thread goes on without it, and one that cannot be read at all is left as it is, the
    /// problem blocking every save of it this run (see `StoredFile`).
    nonisolated static func openDocument(_ id: UUID) -> (document: ThreadDocument, problem: StorageProblem?) {
        DiskWriter.shared.waitForQueuedWrites()
        let read = StoredFile.read(threadURL(id), subject: "This thread's history", decode: decodeThread)
        return (read.value ?? ThreadDocument(threadID: id), read.problem)
    }

    /// A thread file's bytes as the runtime wants the document (see `readDocument`).
    private nonisolated static func decodeThread(_ data: Data) throws -> ThreadDocument {
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        var document = try decoder.decode(ThreadDocument.self, from: data)
        document.items = document.items.filter { !$0.isEmptyReasoning }.map(\.cleanedOfEmDashes)
        return document
    }

    static func deleteDocument(_ id: UUID) {
        deleteDocuments([id])
    }

    /// Deletes several threads' files in one hop to the writer, so a bulk delete waits for
    /// the queued saves once rather than once per thread. A save of a thread may still be
    /// on its way to the writer; the removal takes its place, so the thread never comes back.
    static func deleteDocuments(_ ids: [UUID]) {
        guard !ids.isEmpty else { return }
        for id in ids { DocumentPrefetch.shared.forget(id) }
        // A thread's session log is its own too, and goes with it.
        DiskWriter.shared.removeSynchronously(ids.map(threadURL) + ids.map(SessionLog.url))
    }

    /// Copies a file into app storage so a message keeps its attachment after the original moves.
    static func importAttachment(from source: URL) throws -> Attachment {
        let id = UUID()
        let ext = source.pathExtension.isEmpty ? "bin" : source.pathExtension.lowercased()
        let destination = attachmentsDirectory.appendingPathComponent("\(id.uuidString).\(ext)")
        try FileManager.default.copyItem(at: source, to: destination)
        return Attachment(id: id, name: source.lastPathComponent, path: destination.path, mimeType: MimeType.of(ext))
    }

    static func importAttachment(data: Data, name: String, fileExtension: String) throws -> Attachment {
        let id = UUID()
        let destination = attachmentsDirectory.appendingPathComponent("\(id.uuidString).\(fileExtension)")
        try data.write(to: destination, options: .atomic)
        return Attachment(id: id, name: name, path: destination.path, mimeType: MimeType.of(fileExtension))
    }

    /// Removes attachment files no thread refers to any more: the photos of a deleted
    /// thread, a draft's that was never sent, a follow-up's that was dropped. A file is
    /// only an orphan once it is a day old, so a draft still open when the app relaunches
    /// keeps its files, and only files named the way `importAttachment` names them go.
    /// Runs off the main actor; the thread files are scanned as text, never decoded.
    nonisolated static func sweepOrphanedAttachments() async {
        let attachments = attachmentsDirectory
        let threads = threadsDirectory
        await Task.detached(priority: .utility) {
            let fileManager = FileManager.default
            guard let names = try? fileManager.contentsOfDirectory(atPath: attachments.path), !names.isEmpty,
                  let documents = try? fileManager.contentsOfDirectory(atPath: threads.path) else { return }
            var referenced = Set<String>()
            // A thread file kept aside because it did not read (see `StoredFile.setAside`)
            // still names its attachments: they stay for as long as it does, so the history
            // it holds can be put back whole. Its text is scanned like any other.
            for document in documents where document.hasSuffix(".json") || StoredFile.isKept(document) {
                // A thread file that cannot be read now (no permission, a disk error, renamed
                // aside or deleted since the listing) may still name attachments: taking it for
                // one that names none would sweep them. Nothing goes this time; the next launch
                // sweeps again.
                guard let data = try? Data(contentsOf: threads.appendingPathComponent(document)) else { return }
                let text = String(decoding: data, as: UTF8.self)
                // The encoder writes the path's slashes as `\/`.
                for match in text.matches(of: #/attachments\\?\/([0-9A-Fa-f-]{36}\.[A-Za-z0-9]+)/#) {
                    referenced.insert(String(match.output.1))
                }
                // A forwarded continuation keeps its own copy of the metadata, and the
                // files it names stay referenced while the thread holding it is on disk.
                if text.contains("\"continuation\""),
                   let stored = try? JSONDecoder.storage.decode(StoredContinuation.self, from: data) {
                    for attachment in stored.continuation?.attachments ?? [] {
                        referenced.insert(attachment.url.lastPathComponent)
                    }
                }
            }
            let cutoff = Date.now.addingTimeInterval(-24 * 60 * 60)
            for name in names where !referenced.contains(name) {
                guard name.wholeMatch(of: #/[0-9A-Fa-f-]{36}\.[A-Za-z0-9]+/#) != nil else { continue }
                let url = attachments.appendingPathComponent(name)
                guard let modified = (try? url.resourceValues(forKeys: [.contentModificationDateKey]))?.contentModificationDate,
                      modified < cutoff else { continue }
                try? fileManager.removeItem(at: url)
            }
        }.value
    }

    /// How many files a message carries.
    static let attachmentLimit = 8

    /// Imports what a drop, paste, picker or downloads pick handed over and appends the
    /// results, in that order, to the attachments. The free slots are counted here, so the
    /// call returns before any byte is copied, and counted again when the copies land,
    /// since more may have been attached meanwhile: anything past the limit by then is
    /// removed again. The copies and transcodes run off the main actor; a dropped video
    /// or a pasted screenshot never stalls the composer.
    @MainActor @discardableResult
    static func attach(_ sources: [AttachmentSource], to attachments: Binding<[Attachment]>) -> Task<Void, Never>? {
        let batch = Array(sources.prefix(max(0, attachmentLimit - attachments.wrappedValue.count)))
        guard !batch.isEmpty else { return nil }
        return Task.detached(priority: .userInitiated) {
            let imported = batch.compactMap(importSource)
            let kept = await MainActor.run {
                let room = max(0, attachmentLimit - attachments.wrappedValue.count)
                attachments.wrappedValue.append(contentsOf: imported.prefix(room))
                return room
            }
            for orphan in imported.dropFirst(kept) { try? FileManager.default.removeItem(at: orphan.url) }
        }
    }

    /// A file the user picked, dropped or pasted, copied into app storage: folders are
    /// skipped, HEIC photos become JPEGs, which every provider can read, and pasted
    /// images are stored as PNGs. Runs off the main actor; ImageIO does the transcodes.
    static func importSource(_ source: AttachmentSource) -> Attachment? {
        switch source {
        case .file(let url):
            var isDirectory: ObjCBool = false
            guard FileManager.default.fileExists(atPath: url.path, isDirectory: &isDirectory), !isDirectory.boolValue else { return nil }
            let fileExtension = url.pathExtension.lowercased()
            guard fileExtension == "heic" || fileExtension == "heif" else { return try? importAttachment(from: url) }
            guard let source = CGImageSourceCreateWithURL(url as CFURL, nil),
                  let data = transcode(source, to: .jpeg) else { return nil }
            return try? importAttachment(data: data, name: url.deletingPathExtension().lastPathComponent + ".jpg", fileExtension: "jpg")
        case .image(let data, let type):
            let png: Data?
            if type == .png {
                png = data
            } else {
                png = CGImageSourceCreateWithData(data as CFData, nil).flatMap { transcode($0, to: .png) }
            }
            guard let png else { return nil }
            return try? importAttachment(data: png, name: "Pasted image.png", fileExtension: "png")
        }
    }

    /// The first image of the source re-encoded at full size with its orientation
    /// applied, the way the photo looked where it came from.
    private static func transcode(_ source: CGImageSource, to type: UTType) -> Data? {
        guard let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any],
              let width = properties[kCGImagePropertyPixelWidth] as? Int,
              let height = properties[kCGImagePropertyPixelHeight] as? Int else { return nil }
        let options: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceThumbnailMaxPixelSize: max(width, height),
        ]
        guard let image = CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary) else { return nil }
        let data = NSMutableData()
        guard let destination = CGImageDestinationCreateWithData(data, type.identifier as CFString, 1, nil) else { return nil }
        let encoding: [CFString: Any] = type == .jpeg ? [kCGImageDestinationLossyCompressionQuality: 0.88] : [:]
        CGImageDestinationAddImage(destination, image, encoding as CFDictionary)
        guard CGImageDestinationFinalize(destination) else { return nil }
        return data as Data
    }
}

/// A file or image handed to a composer, imported by `Storage.attach`.
enum AttachmentSource: Sendable {
    case file(URL)
    /// Image bytes off the pasteboard, in the type they came as.
    case image(Data, UTType)
}

enum MimeType {
    static func of(_ fileExtension: String) -> String {
        switch fileExtension.lowercased() {
        case "png": "image/png"
        case "jpg", "jpeg": "image/jpeg"
        case "gif": "image/gif"
        case "webp": "image/webp"
        case "heic": "image/heic"
        case "pdf": "application/pdf"
        case "json": "application/json"
        case "md", "markdown": "text/markdown"
        default: "text/plain"
        }
    }
}

final class DocumentPrefetch: Sendable {
    static let shared = DocumentPrefetch()

    private struct Request {
        let id: UUID
        let token = UUID()
    }

    private struct State {
        var ready: RecentCache<UUID, ThreadDocument>
        var pending: [UUID: UUID] = [:]
        var queue: [Request] = []
        var worker: Task<Void, Never>?
    }

    private let state: Mutex<State>
    private let pendingLimit: Int
    private let load: @Sendable (UUID) -> (document: ThreadDocument, cost: Int)?

    init(
        limit: Int = 12,
        costLimit: Int = 16 * 1_024 * 1_024,
        pendingLimit: Int = 24,
        load: @escaping @Sendable (UUID) -> (document: ThreadDocument, cost: Int)? = Storage.readDocument
    ) {
        state = Mutex(State(ready: RecentCache(limit: limit, costLimit: costLimit)))
        self.pendingLimit = max(1, pendingLimit)
        self.load = load
    }

    /// Queues documents to decode. `urgent` ones go to the front, ahead of what launch
    /// queued: they are the rows beside the selection and under the pointer, which the next
    /// click is likely to open, while launch's dozen can wait their turn.
    @discardableResult
    func warm(_ ids: [UUID], urgent: Bool = false) -> Task<Void, Never>? {
        state.withLock { state -> Task<Void, Never>? in
            for id in ids.reversed() where urgent {
                guard !state.ready.contains(id), let index = state.queue.firstIndex(where: { $0.id == id }) else { continue }
                state.queue.insert(state.queue.remove(at: index), at: 0)
            }
            for id in ids {
                guard !state.ready.contains(id), state.pending[id] == nil else { continue }
                guard state.queue.count < pendingLimit else { break }
                let request = Request(id: id)
                if urgent { state.queue.insert(request, at: 0) } else { state.queue.append(request) }
                state.pending[id] = request.token
            }
            guard state.worker == nil, !state.queue.isEmpty else { return state.worker }
            let task = Task.detached(priority: .utility) { [self] in
                while let request = next() {
                    let result = load(request.id)
                    self.state.withLock { state in
                        guard state.pending[request.id] == request.token else { return }
                        state.pending[request.id] = nil
                        if let result, !Task.isCancelled {
                            state.ready.insert(result.document, for: request.id, cost: result.cost)
                        }
                    }
                }
            }
            state.worker = task
            return task
        }
    }

    private func next() -> Request? {
        state.withLock { state in
            if Task.isCancelled {
                state.queue.removeAll(keepingCapacity: true)
                state.pending.removeAll(keepingCapacity: true)
            }
            guard !state.queue.isEmpty else {
                state.worker = nil
                return nil
            }
            return state.queue.removeFirst()
        }
    }

    func take(_ id: UUID) -> ThreadDocument? {
        state.withLock { state in
            state.pending[id] = nil
            state.queue.removeAll { $0.id == id }
            return state.ready.removeValue(for: id)
        }
    }

    func forget(_ id: UUID) {
        _ = take(id)
    }
}

final class DiskWriter: Sendable {
    static let shared = DiskWriter()
    private let queue = DispatchQueue(label: "bottega.storage.write", qos: .utility)
    /// The newest snapshot waiting per file. A save queued behind a slow write is replaced
    /// by the next one for the same file, so a backlog never encodes and writes documents
    /// that are already stale when their turn comes.
    private let pending = Mutex<[URL: any Encodable & Sendable]>([:])

    func encodeAndWrite<Value: Encodable & Sendable>(_ value: Value, to url: URL) {
        let queued = pending.withLock { pending in
            let queued = pending[url] != nil
            pending[url] = value
            return queued
        }
        guard !queued else { return }
        queue.async { [self] in
            guard let value = pending.withLock({ $0.removeValue(forKey: url) }) else { return }
            Self.write(value, to: url)
        }
    }

    func writeSynchronously<Value: Encodable & Sendable>(_ value: Value, to url: URL) {
        pending.withLock { $0[url] = nil }
        queue.sync { Self.write(value, to: url) }
    }

    func removeSynchronously(_ url: URL) {
        removeSynchronously([url])
    }

    func removeSynchronously(_ urls: [URL]) {
        pending.withLock { pending in
            for url in urls { pending[url] = nil }
        }
        queue.sync {
            for url in urls { try? FileManager.default.removeItem(at: url) }
        }
    }

    /// Returns once every write queued so far has landed, so a read that follows sees them.
    func waitForQueuedWrites() {
        queue.sync {}
    }

    private static func write(_ value: any Encodable, to url: URL) {
        guard let data = try? JSONEncoder.storage.encode(value) else { return }
        do {
            try data.write(to: url, options: .atomic)
        } catch {
            NSLog("Bottega could not save %@: %@", url.lastPathComponent, error.localizedDescription)
        }
    }
}
