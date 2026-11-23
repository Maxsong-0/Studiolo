// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation
import os

/// Where `StoredFile` says what it did with bytes that did not read: every file kept aside,
/// every file left unread, every setting copied out, and why.
private let storageLog = Logger(subsystem: AppInfo.logSubsystem, category: "storage")

/// A stored file, or a setting's stored bytes, that did not read, and what became of them so
/// that nothing they held is lost (see `StoredFile`). Launch gathers the stores' problems into
/// one alert (`AppModel.reportStorageProblems`); a thread's own shows in its timeline.
struct StorageProblem: Sendable, Equatable {
    enum Outcome: Sendable, Equatable {
        /// The bytes are there but do not read as what they should hold. They are kept at this
        /// URL (the file renamed aside beside itself, or a setting's bytes copied into
        /// `StoredFile.recoveryDirectory`) and the store goes on without them.
        case keptAside(URL)
        /// They could not be read, or could not be kept aside: they stay exactly where they
        /// are, and the store writes nothing over them until the app is launched again.
        case leftInPlace
    }

    /// What the bytes held, as the user knows it: "The library", "Crew pairs".
    var subject: String
    /// Where they are, for the user to find: a file's path, or a setting's key.
    var location: String
    var outcome: Outcome
    /// Why, in the decoder's or the file system's own words.
    var reason: String

    /// Whether a save over `location` would lose what it holds: while the bytes are there
    /// unread, nothing may be written there this run.
    var blocksSaving: Bool {
        if case .leftInPlace = outcome { return true }
        return false
    }

    /// What the user is told: what happened, where the bytes are, and what happens now.
    var message: String {
        switch outcome {
        case .keptAside(let copy):
            let folder = (copy.deletingLastPathComponent().path as NSString).abbreviatingWithTildeInPath
            return "\(subject) could not be read (\(reason)). What it held is kept as \(copy.lastPathComponent) in \(folder), and Bottega went on without it. Quit Bottega before putting it back, or the next save writes over it. Nothing removes the kept copy: delete it yourself once it is no longer wanted."
        case .leftInPlace:
            return "\(subject) could not be read (\(reason)). It is left untouched at \(location), and nothing is written over it until Bottega is opened again, so changes to it made now are not kept."
        }
    }
}

/// Reads what a store saves back whole, so that no store writes over something it could not
/// read. A read that failed used to count as an empty store, and the next save put the empty
/// store in the file's place. Three cases are told apart instead:
///
/// - Nothing there (a first launch, a thread never saved): nothing to keep. The store starts
///   empty and saves as usual.
/// - There, but not readable as what it should hold (broken JSON, a required field gone, a
///   list that is not a list): the file is renamed aside, beside itself, as
///   `<name>.unreadable-<yyyyMMdd-HHmmss>`, in one rename that never replaces anything
///   (`setAside`). The store starts empty and saves as usual, into a new file, and the user is
///   told where the old one went.
/// - There, but not readable at all (no permission, a disk error, a file the system could not
///   bring down): left where it is, as it is, and the store saves nothing over it this run
///   (`StorageProblem.blocksSaving`). The next launch reads it again.
///
/// A Codable value in user defaults cannot be renamed; `readSetting` copies its bytes into
/// `recoveryDirectory` instead, before anything can write over them.
enum StoredFile {
    /// What a read found: the contents, and what went wrong when something did. Both nil for
    /// a file, or a setting, that is not there.
    struct Loaded<Value> {
        var value: Value? = nil
        var problem: StorageProblem? = nil
    }

    /// What goes between a kept file's name and the time it was kept.
    static let setAsideInfix = ".unreadable-"

    /// `recovery/` in the app's Application Support folder, the folder `Storage.root` names:
    /// where the bytes of a setting that did not read are copied. Found here rather than
    /// through `Storage`, a layer up, the way `HangWatchdog` finds the app's log folder.
    static let recoveryDirectory: URL = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
        .appendingPathComponent(AppInfo.storageName, isDirectory: true)
        .appendingPathComponent("recovery", isDirectory: true)

    /// Whether a file name is one `setAside` or `keepCopy` gave: a kept file, not a live one.
    static func isKept(_ name: String) -> Bool {
        name.contains(setAsideInfix)
    }

    /// Reads and decodes the file a store saves back, telling the three cases apart (see the
    /// type). `subject` names what it holds for the user. Safe from any thread.
    static func read<Value>(_ url: URL, subject: String, decode: (Data) throws -> Value) -> Loaded<Value> {
        let location = (url.path as NSString).abbreviatingWithTildeInPath
        let data: Data
        do {
            data = try Data(contentsOf: url)
        } catch {
            if isMissing(error) { return Loaded() }
            storageLog.error("Could not read \(url.path, privacy: .public); left as it is and not written over this run: \(String(describing: error), privacy: .public)")
            return Loaded(problem: StorageProblem(subject: subject, location: location, outcome: .leftInPlace, reason: error.localizedDescription))
        }
        do {
            let value = try decode(data)
            return Loaded(value: value)
        } catch {
            return Loaded(problem: keepAside(url, subject: subject, location: location, failure: error))
        }
    }

    /// Reads a Codable value a setting keeps in user defaults. Nothing under the key reads as
    /// missing. Bytes that do not decode (or a value that is not bytes at all) are copied into
    /// `recoveryDirectory`, and only once the copy is whole are they taken out of the defaults:
    /// the setting's version of renaming a file aside, so the next write loses nothing and the
    /// next launch reads the setting as never set. When the copy fails, the bytes stay, the
    /// problem blocks saving, and the caller leaves the key alone this run.
    static func readSetting<Value>(_ key: String, subject: String, in defaults: UserDefaults = .standard, decode: (Data) throws -> Value) -> Loaded<Value> {
        guard let stored = defaults.object(forKey: key) else { return Loaded() }
        let problem: StorageProblem
        if let data = stored as? Data {
            do {
                let value = try decode(data)
                return Loaded(value: value)
            } catch {
                storageLog.error("The \(key, privacy: .public) setting did not read: \(String(describing: error), privacy: .public)")
                problem = keepSetting(data, named: key + ".json", key: key, subject: subject, reason: reason(error))
            }
        } else {
            // Something other than bytes under the key: kept as the property list it is.
            let list = try? PropertyListSerialization.data(fromPropertyList: stored, format: .xml, options: 0)
            problem = keepSetting(list, named: key + ".plist", key: key, subject: subject, reason: "what is stored there is not data")
        }
        if !problem.blocksSaving { defaults.removeObject(forKey: key) }
        return Loaded(problem: problem)
    }

    /// Renames a file aside, beside itself, as `<name>.unreadable-<yyyyMMdd-HHmmss>` (`-2`,
    /// `-3`… after the stamp when a file kept in the same second has the name), and returns
    /// where it went. One rename in one folder, so never a copy (see `renameExclusively`).
    static func setAside(_ url: URL) throws -> URL {
        try claimName(for: url, prefix: url.lastPathComponent + setAsideInfix, in: url.deletingLastPathComponent())
    }

    /// Keeps `data` as `<name>.unreadable-<yyyyMMdd-HHmmss>` in `folder`, as a file of its own:
    /// written whole under a name nothing else uses, then renamed into place by
    /// `renameExclusively`, so a copy is all there or not there and never replaces another. A
    /// copy already kept with the same bytes stands for this one, so a setting that stays
    /// unread across launches is kept once, not once a launch.
    static func keepCopy(of data: Data, named name: String, in folder: URL) throws -> URL {
        let manager = FileManager.default
        try manager.createDirectory(at: folder, withIntermediateDirectories: true)
        let prefix = name + setAsideInfix
        for existing in ((try? manager.contentsOfDirectory(atPath: folder.path)) ?? []).sorted() where existing.hasPrefix(prefix) {
            let url = folder.appendingPathComponent(existing)
            if (try? Data(contentsOf: url)) == data { return url }
        }
        let staged = folder.appendingPathComponent(".\(name).\(UUID().uuidString).partial")
        try data.write(to: staged, options: .atomic)
        // Gone once it is renamed; still there, and removed, when the rename failed.
        defer { try? manager.removeItem(at: staged) }
        return try claimName(for: staged, prefix: prefix, in: folder)
    }

    // MARK: - Private

    /// A file that is there but does not decode: renamed aside, or, when even that fails,
    /// left where it is with saving blocked.
    private static func keepAside(_ url: URL, subject: String, location: String, failure: any Error) -> StorageProblem {
        do {
            let copy = try setAside(url)
            storageLog.error("\(url.path, privacy: .public) did not read and is kept as \(copy.lastPathComponent, privacy: .public): \(String(describing: failure), privacy: .public)")
            return StorageProblem(subject: subject, location: location, outcome: .keptAside(copy), reason: reason(failure))
        } catch {
            storageLog.error("\(url.path, privacy: .public) did not read and could not be renamed aside (\(String(describing: error), privacy: .public)); left as it is and not written over this run: \(String(describing: failure), privacy: .public)")
            return StorageProblem(subject: subject, location: location, outcome: .leftInPlace, reason: "\(reason(failure)); moving it aside failed too: \(error.localizedDescription)")
        }
    }

    /// A setting's bytes that did not read: copied into `recoveryDirectory`, or, when there
    /// are no bytes to copy or the copy fails, left under the key with saving blocked.
    private static func keepSetting(_ bytes: Data?, named name: String, key: String, subject: String, reason: String) -> StorageProblem {
        let location = "user defaults, key \(key)"
        guard let bytes else {
            storageLog.error("The \(key, privacy: .public) setting did not read and could not be copied aside; left as it is this run")
            return StorageProblem(subject: subject, location: location, outcome: .leftInPlace, reason: "\(reason); it could not be copied aside either")
        }
        do {
            let copy = try keepCopy(of: bytes, named: name, in: recoveryDirectory)
            storageLog.error("The \(key, privacy: .public) setting did not read; its bytes are kept at \(copy.path, privacy: .public)")
            return StorageProblem(subject: subject, location: location, outcome: .keptAside(copy), reason: reason)
        } catch {
            storageLog.error("The \(key, privacy: .public) setting did not read and could not be copied aside (\(String(describing: error), privacy: .public)); left as it is this run")
            return StorageProblem(subject: subject, location: location, outcome: .leftInPlace, reason: "\(reason); copying it aside failed too: \(error.localizedDescription)")
        }
    }

    /// Renames `source` to `<prefix><yyyyMMdd-HHmmss>` in `folder`, or with `-2`, `-3`… after
    /// the stamp while that name is taken, and returns where it went.
    private static func claimName(for source: URL, prefix: String, in folder: URL) throws -> URL {
        let stamp = timestamp(.now)
        for attempt in 1...99 {
            let destination = folder.appendingPathComponent(prefix + stamp + (attempt == 1 ? "" : "-\(attempt)"))
            if try renameExclusively(source, to: destination) { return destination }
        }
        throw CocoaError(.fileWriteFileExists)
    }

    /// One rename from `source` to `destination` that never replaces anything: false when
    /// `destination` is taken. Within one volume a rename is atomic, so the bytes are under one
    /// name or the other at every moment, whole; `RENAME_EXCL` refuses a taken name in that
    /// same step, where a look followed by a rename could replace a file made in between.
    private static func renameExclusively(_ source: URL, to destination: URL) throws -> Bool {
        let code: Int32 = source.path.withCString { from in
            destination.path.withCString { to in
                renamex_np(from, to, UInt32(RENAME_EXCL)) == 0 ? 0 : errno
            }
        }
        switch code {
        case 0:
            return true
        case EEXIST:
            return false
        case ENOTSUP:
            // A volume without exclusive renames: look, then move. `moveItem` refuses a taken
            // name as well, and nothing else writes names of this shape.
            guard !FileManager.default.fileExists(atPath: destination.path) else { return false }
            try FileManager.default.moveItem(at: source, to: destination)
            return true
        default:
            throw POSIXError(POSIXErrorCode(rawValue: code) ?? .EIO)
        }
    }

    /// Whether a failed read found no file there at all, as opposed to one it could not read.
    private static func isMissing(_ error: any Error) -> Bool {
        let failure = error as NSError
        if failure.domain == NSCocoaErrorDomain, failure.code == NSFileReadNoSuchFileError { return true }
        if failure.domain == NSPOSIXErrorDomain, failure.code == Int(ENOENT) { return true }
        guard let underlying = failure.userInfo[NSUnderlyingErrorKey] as? NSError else { return false }
        return underlying.domain == NSPOSIXErrorDomain && underlying.code == Int(ENOENT)
    }

    /// Why bytes did not decode, short enough for an alert: a decoding error's own sentence
    /// and where in the file it stopped, anything else's description.
    private static func reason(_ error: any Error) -> String {
        let context: DecodingError.Context? = switch error as? DecodingError {
        case .dataCorrupted(let found)?, .keyNotFound(_, let found)?, .typeMismatch(_, let found)?, .valueNotFound(_, let found)?:
            found
        default:
            nil
        }
        guard let context else { return error.localizedDescription }
        guard !context.codingPath.isEmpty else { return context.debugDescription }
        return "\(context.debugDescription) At \(readablePath(context.codingPath))."
    }

    /// The local time a file is kept at, as its name carries it: `20260923-153000`.
    private static func timestamp(_ date: Date) -> String {
        let formatter = DateFormatter()
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.calendar = Calendar(identifier: .gregorian)
        formatter.timeZone = .current
        formatter.dateFormat = "yyyyMMdd-HHmmss"
        return formatter.string(from: date)
    }
}

extension StoredFile.Loaded: Sendable where Value: Sendable {}
