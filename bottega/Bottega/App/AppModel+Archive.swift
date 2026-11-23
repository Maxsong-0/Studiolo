// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import AppKit
import Foundation

/// One archive as its page shows it (see `ArchivePage`): the tree of its notes, the note
/// open, the draft being typed and what the folder did meanwhile. Kept on the model by
/// prefix, so leaving the page for a thread and coming back finds the draft as it was.
///
/// The notes are the user's own files, edited by Obsidian and by agents while the page is
/// up: a note that changes on disk is read again while nothing is being typed into it,
/// and merged with the draft when something is (see `save`).
@MainActor
@Observable
final class ArchivePageModel {
    /// The note open, as it was read: the base a save merges against.
    struct Document: Equatable {
        let path: String
        var base: String
    }

    /// A save that met changes on disk overlapping the draft's own lines (see `save`).
    struct Conflict {
        let disk: String
    }

    let prefix: String
    /// The archive folder.
    let root: URL

    private(set) var tree: [ArchiveFileNode] = []
    /// Rows of the task board's "in progress" table; nil without a board.
    private(set) var openTasks: Int?
    private(set) var document: Document?
    var isEditing = false
    /// The editor's text; the note as read until the user types.
    var draft = ""
    /// Why the note could not be read or written, if it could not.
    private(set) var problem: String?
    /// The open note changed on disk while the draft had changes of its own: the save
    /// merges them (see `save`).
    private(set) var changedOnDisk = false
    /// A note asked for while the draft has unsaved changes: the page asks before it goes.
    var pendingOpen: String?
    private(set) var conflict: Conflict?
    private(set) var isSaving = false

    @ObservationIgnored private var watch: ArchiveFolderWatch?

    init(prefix: String, root: URL) {
        self.prefix = prefix
        self.root = root
        rescan()
        let watch = ArchiveFolderWatch(root: root) { [weak self] in
            Task { @MainActor in self?.folderChanged() }
        }
        self.watch = watch
        watch.start()
    }

    /// Lets the folder go: the watch stops (it held itself until now, the way an FSEvents
    /// stream does), and nothing here is read again. For a page the vault no longer has.
    func stop() {
        watch?.stop()
        watch = nil
    }

    var selectedPath: String? { document?.path }
    var isDirty: Bool { document.map { draft != $0.base } ?? false }

    // MARK: - The folder

    func rescan() {
        tree = ArchiveFiles.tree(root: root)
        openTasks = (try? String(contentsOf: root.appendingPathComponent(ArchiveFiles.tasksFile), encoding: .utf8)).map(ArchiveFiles.openTaskCount)
    }

    private func folderChanged() {
        rescan()
        guard let document else { return }
        guard let disk = try? String(contentsOf: root.appendingPathComponent(document.path), encoding: .utf8) else {
            // Gone, or unreadable now: what is on screen stays, and a save says what it met.
            return
        }
        guard disk != document.base else {
            changedOnDisk = false
            return
        }
        if isDirty {
            changedOnDisk = true
        } else {
            // Nothing typed: the note follows the disk, as a viewer would.
            self.document?.base = disk
            draft = disk
        }
    }

    // MARK: - Opening notes

    /// Opens the note at `path` (relative to the archive), unless the draft has unsaved
    /// changes: the page then asks what to do with them first (`pendingOpen`).
    func open(_ path: String) {
        guard path != document?.path else { return }
        if isDirty {
            pendingOpen = path
            return
        }
        load(path)
    }

    /// The pending note, once the user has chosen: the draft saved first, or let go of.
    func openPending(discardingDraft: Bool) async {
        guard let path = pendingOpen else { return }
        pendingOpen = nil
        if discardingDraft {
            discardDraft()
        } else {
            await save()
            guard !isDirty else { return }
        }
        load(path)
    }

    /// The note a wiki link means; false when the archive has none of that name.
    @discardableResult
    func open(link target: WikiLinks.Target) -> Bool {
        guard let note = ArchiveFiles.resolve(target.note, in: tree) else { return false }
        open(note.path)
        return true
    }

    /// The image a note refers to, within the archive: by the path as written, from the
    /// note's own folder or from the archive's, else by its name anywhere in the archive
    /// (the way Obsidian finds an embed). Nothing outside the archive is ever shown, and
    /// nothing fetched: a URL or an absolute path is no image here.
    func imageURL(for reference: String) -> URL? {
        let reference = reference.removingPercentEncoding ?? reference
        guard !reference.isEmpty, !reference.contains("://"), !reference.hasPrefix("/"), !reference.hasPrefix("~") else { return nil }
        let inside = root.standardizedFileURL.path + "/"
        func within(_ url: URL) -> URL? {
            let path = url.standardizedFileURL.path
            guard path.hasPrefix(inside), FileManager.default.fileExists(atPath: path) else { return nil }
            return URL(fileURLWithPath: path)
        }
        var bases = [root]
        if let path = document?.path { bases.insert(root.appendingPathComponent(path).deletingLastPathComponent(), at: 0) }
        for base in bases {
            if let found = within(base.appendingPathComponent(reference)) { return found }
        }
        let name = (reference as NSString).lastPathComponent
        guard let walk = FileManager.default.enumerator(at: root, includingPropertiesForKeys: [.isRegularFileKey], options: [.skipsHiddenFiles, .skipsPackageDescendants]) else { return nil }
        for case let url as URL in walk where url.lastPathComponent == name {
            if (try? url.resourceValues(forKeys: [.isRegularFileKey]))?.isRegularFile == true, let found = within(url) { return found }
        }
        return nil
    }

    private func load(_ path: String) {
        conflict = nil
        changedOnDisk = false
        do {
            let text = try String(contentsOf: root.appendingPathComponent(path), encoding: .utf8)
            document = Document(path: path, base: text)
            draft = text
            problem = nil
        } catch {
            document = nil
            draft = ""
            problem = "\(path) could not be read: \(error.localizedDescription)"
        }
    }

    func discardDraft() {
        guard let document else { return }
        draft = document.base
        changedOnDisk = false
        conflict = nil
    }

    // MARK: - Saving

    /// Writes the draft to the note. A note changed on disk since it was read (by Obsidian,
    /// an agent, a sync) is not written over: the two sets of changes are merged
    /// (`MergeFile`, the note as read being the base), and where they overlap the draft
    /// stays unsaved and the page asks whether to let it go or keep it as a copy
    /// (`conflict`).
    func save() async {
        guard let document, isDirty, !isSaving else { return }
        isSaving = true
        defer { isSaving = false }
        let url = root.appendingPathComponent(document.path)
        let disk: String
        do {
            disk = try String(contentsOf: url, encoding: .utf8)
        } catch {
            problem = "\(document.path) could not be read before saving: \(error.localizedDescription)"
            return
        }
        // What is being saved; typing that lands meanwhile stays in the editor.
        let typed = draft
        var text = typed
        if disk != document.base {
            do {
                let merged = try await MergeFile.merge(base: Data(document.base.utf8), ours: Data(typed.utf8), theirs: Data(disk.utf8))
                guard !merged.hasConflicts else {
                    conflict = Conflict(disk: disk)
                    return
                }
                text = String(decoding: merged.merged, as: UTF8.self)
            } catch {
                problem = "\(document.path) could not be merged with the changes on disk: \(error.localizedDescription)"
                return
            }
        }
        do {
            try await Self.write(text, to: url)
        } catch {
            problem = "\(document.path) could not be saved: \(error.localizedDescription)"
            return
        }
        // Read back as written, so the merge result shows and the next save starts from it;
        // words typed while the save ran are kept, over that base.
        self.document?.base = text
        if draft == typed { draft = text }
        changedOnDisk = false
        conflict = nil
        problem = nil
    }

    /// The conflict put by for now: the draft stays as it is, unsaved, and the note is
    /// still marked as changed on disk. The next save looks again.
    func keepEditingAfterConflict() {
        conflict = nil
    }

    /// At quit, without asking: the draft saved, or, where the disk changed in the same
    /// lines, kept as a copy beside the note (see `keepDraftAsCopy`), so nothing typed
    /// here is lost with the app.
    func saveOrKeepBeforeQuit() async {
        guard isDirty else { return }
        await save()
        if conflict != nil { await keepDraftAsCopy() }
    }

    /// The conflicting draft kept beside the note, as `<name> (mine).md`, and the note
    /// itself left as the disk has it.
    func keepDraftAsCopy() async {
        guard let document, let conflict else { return }
        let stem = (document.path as NSString).deletingPathExtension
        var copy = stem + " (mine).md"
        var attempt = 2
        while FileManager.default.fileExists(atPath: root.appendingPathComponent(copy).path) {
            copy = stem + " (mine \(attempt)).md"
            attempt += 1
        }
        do {
            try await Self.write(draft, to: root.appendingPathComponent(copy))
        } catch {
            problem = "The copy could not be saved: \(error.localizedDescription)"
            return
        }
        self.document?.base = conflict.disk
        draft = conflict.disk
        self.conflict = nil
        changedOnDisk = false
        problem = nil
        rescan()
    }

    /// The conflicting draft let go of: the note as the disk has it.
    func discardConflictingDraft() {
        guard let conflict else { return }
        document?.base = conflict.disk
        draft = conflict.disk
        self.conflict = nil
        changedOnDisk = false
    }

    /// Whole, through a temporary file renamed into place: a reader never sees half a note.
    private static func write(_ text: String, to url: URL) async throws {
        try await Task.detached(priority: .userInitiated) {
            try Data(text.utf8).write(to: url, options: .atomic)
        }.value
    }
}

extension AppModel {
    /// Fills the main area with the archive's page (see `ArchivePage`).
    func showArchive(_ prefix: String) {
        selectedArchivePrefix = prefix
    }

    /// The archive's page model, made on first sight and kept: a draft outlives leaving
    /// the page. Nil without a vault; a page of another vault's archive (the vault was
    /// changed in Settings since) goes, and the archive is read afresh.
    func archivePage(for prefix: String) -> ArchivePageModel? {
        guard let root = studiolo.root else { return nil }
        let folder = root.appendingPathComponent(prefix)
        if let page = archivePages[prefix] {
            if page.root == folder { return page }
            page.stop()
            archivePages[prefix] = nil
        }
        let page = ArchivePageModel(prefix: prefix, root: folder)
        archivePages[prefix] = page
        return page
    }

    /// Lets every archive page go, drafts and all: the vault they read is no longer the one
    /// in Settings.
    func dropArchivePages() {
        for page in archivePages.values { page.stop() }
        archivePages.removeAll()
        selectedArchivePrefix = nil
    }

    /// Whether any archive page holds a draft not yet saved.
    var hasUnsavedArchiveDrafts: Bool {
        archivePages.values.contains(where: \.isDirty)
    }

    /// Every unsaved draft saved, or kept as a copy, before the app goes (see
    /// `ArchivePageModel.saveOrKeepBeforeQuit`).
    func saveArchiveDraftsBeforeQuit() async {
        for page in archivePages.values where page.isDirty {
            await page.saveOrKeepBeforeQuit()
        }
    }

    /// The thread picked while the page had an unsaved draft is shown now (see
    /// `archiveLeave`): the draft saved first, or let go of. A save that could not finish
    /// (the note changed on disk in the same lines, or could not be written) keeps the page
    /// up, with its own word on why.
    func leaveArchive(saving: Bool) async {
        guard let request = archiveLeave, let page = archivePages[request.prefix] else { return }
        if saving {
            await page.save()
            guard !page.isDirty else { return }
        } else {
            page.discardDraft()
        }
        archiveLeave = nil
        selectedArchivePrefix = nil
    }

    /// The page stays up and the thread picked meanwhile is unselected again.
    func stayOnArchive() {
        archiveLeave = nil
        selectedThreadID = nil
    }
}

/// A thread picked while the archive page showing had an unsaved draft (see
/// `AppModel.archiveLeave`).
struct ArchiveLeaveRequest: Equatable {
    let prefix: String
    let threadID: UUID
}
