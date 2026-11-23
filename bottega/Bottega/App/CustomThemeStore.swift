// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import AppKit
import Observation

/// Keeps theme.json read: once at launch, and again a moment after anything saves it, however
/// the editor writes it (in place, or as a new file renamed over the old one).
///
/// The file lives in the app's support folder (`Storage`), which the UI layer does not depend
/// on, so the store sits in the app layer; the theme it reads and the slot it fills stay in UI
/// (`CustomTheme`, `CustomThemeSlot`).
@MainActor @Observable
final class CustomThemeStore {
    static let shared = CustomThemeStore()

    enum Status: Equatable {
        /// There is no theme.json yet.
        case missing
        /// The last read made a theme.
        case loaded
        /// The file could not be used; the sentence says why.
        case failed(String)
    }

    private(set) var status: Status = .missing

    /// The file, alone in its own folder of the app's support folder (see `Storage`), so the
    /// library's saves beside it never wake the watch.
    nonisolated static let fileURL = Storage.themeDirectory.appendingPathComponent("theme.json")

    @ObservationIgnored private var didStart = false
    @ObservationIgnored private var folderWatch: DispatchSourceFileSystemObject?
    @ObservationIgnored private var fileWatch: DispatchSourceFileSystemObject?
    @ObservationIgnored private var pendingRead: Task<Void, Never>?
    /// The bytes last read, so a save that changed nothing does nothing.
    @ObservationIgnored private var lastData: Data?

    private init() {}

    /// Reads the file and starts watching its folder. Called once, before the first frame, so
    /// a launch in the custom theme starts in its colours.
    func start() {
        guard !didStart else { return }
        didStart = true
        folderWatch = watch(Storage.themeDirectory, events: .write)
        read()
    }

    /// Opens theme.json in the Mac's editor for JSON, writing the starting file first when
    /// there is none, so there is always something to edit. True when it wrote that file.
    @discardableResult
    func edit() -> Bool {
        var created = false
        if !FileManager.default.fileExists(atPath: Self.fileURL.path) {
            do {
                try FileManager.default.createDirectory(at: Storage.themeDirectory, withIntermediateDirectories: true)
                try Data(CustomTheme.template.utf8).write(to: Self.fileURL, options: .atomic)
            } catch {
                status = .failed("theme.json could not be written: \(error.localizedDescription)")
                return false
            }
            created = true
            // The folder may have been made again just now: watch the one that is there.
            folderWatch?.cancel()
            folderWatch = watch(Storage.themeDirectory, events: .write)
            read()
        }
        NSWorkspace.shared.open(Self.fileURL)
        return created
    }

    /// Shows the file in Finder, or its folder while there is no file.
    func reveal() {
        let exists = FileManager.default.fileExists(atPath: Self.fileURL.path)
        NSWorkspace.shared.activateFileViewerSelecting([exists ? Self.fileURL : Storage.themeDirectory])
    }

    /// Reads a moment after the last event: an editor's save arrives as several (a temporary
    /// file, a rename, a delete), and a read in the middle of them catches half a file.
    private func scheduleRead() {
        pendingRead?.cancel()
        pendingRead = Task { [weak self] in
            try? await Task.sleep(for: .milliseconds(200))
            guard !Task.isCancelled else { return }
            self?.read()
        }
    }

    private func read() {
        // A save can leave a new file where the watched one was: watch whatever is there now.
        fileWatch?.cancel()
        fileWatch = watch(Self.fileURL, events: [.write, .extend, .delete, .rename])
        let path = Self.fileURL.path
        guard FileManager.default.fileExists(atPath: path) else {
            guard status != .missing else { return }
            lastData = nil
            status = .missing
            publish(nil)
            return
        }
        // Checked before the read, so a file that is plainly not a theme is never loaded.
        if let size = (try? FileManager.default.attributesOfItem(atPath: path))?[.size] as? Int, size > CustomTheme.byteLimit {
            lastData = nil
            fail("it is over \(CustomTheme.byteLimit / 1024) KB, and a theme takes a few")
            return
        }
        let data: Data
        do {
            data = try Data(contentsOf: Self.fileURL)
        } catch {
            lastData = nil
            fail(error.localizedDescription)
            return
        }
        guard data != lastData else { return }
        lastData = data
        do {
            let theme = try CustomTheme.decode(data)
            status = .loaded
            publish(theme)
        } catch let problem as CustomTheme.Problem {
            fail(problem.message)
        } catch {
            fail(error.localizedDescription)
        }
    }

    /// Says why the file could not be used. The last good read stays in use, so a save in the
    /// middle of an edit changes nothing on screen.
    private func fail(_ reason: String) {
        let kept = CustomThemeSlot.shared.theme != nil
            ? "The last version that read stays in use until the file is saved again"
            : "Until it reads, Custom wears the Bottega colours"
        status = .failed("theme.json: \(reason). \(kept)")
    }

    private func publish(_ theme: CustomTheme?) {
        CustomThemeSlot.shared.update(theme)
        // A save under the theme in use repaints at once, as picking a theme does.
        if ThemeManager.current == .custom { ThemeManager.refresh() }
    }

    /// A watch on a file or a folder, its events on the main queue; nil when there is nothing
    /// there to watch.
    private func watch(_ url: URL, events: DispatchSource.FileSystemEvent) -> DispatchSourceFileSystemObject? {
        let descriptor = open(url.path, O_EVTONLY)
        guard descriptor >= 0 else { return nil }
        let source = DispatchSource.makeFileSystemObjectSource(fileDescriptor: descriptor, eventMask: events, queue: .main)
        source.setEventHandler { [weak self] in
            MainActor.assumeIsolated { self?.scheduleRead() }
        }
        source.setCancelHandler { close(descriptor) }
        source.resume()
        return source
    }
}
