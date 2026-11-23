// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation

/// Opens a new project archive in a Studiolo vault, the way the vault's own
/// `_模板/README.md` says to: the skeleton copied to `项目/<name>/`, the global rules
/// copied in beside it as `CLAUDE.md`, and the dashboard filled in.
///
/// The archive is assembled outside the vault and moved in with one rename that refuses to
/// replace anything, so the vault (and Obsidian Sync, and Studiolo's replica) sees either
/// nothing or the whole folder, never a half-copied one; and a name taken in the meantime,
/// by another window or another agent, fails instead of being written over. Whatever goes
/// wrong, the staging folder goes with it and the vault is as it was.
///
/// This is the user writing to their own vault, as Obsidian does (Studiolo ADR-010: people
/// write directly; agents go through Studiolo). No agent calls it.
enum StudioloArchiveWriter {
    struct Request: Sendable {
        var vault: URL
        var name: String
        var summary: String
        /// The project folder the archive is for; it goes in the repository row.
        var projectPath: String
        /// `YYYY-MM-DD`, for the first item under the dashboard's focus heading.
        var today: String
        /// The global rules to snapshot, `~/.claude/CLAUDE.md`.
        var globalRules: URL
    }

    struct Outcome: Sendable, Equatable {
        /// `项目/<name>`.
        let prefix: String
        /// The global rules file was not there, so the archive has no `CLAUDE.md` yet.
        let missingRules: Bool
    }

    enum Failure: Error, Equatable, LocalizedError {
        case notVault
        case invalidName(String)
        case exists(String)
        case templateMissing
        case copy(String)
        case fill(String)
        case move(String)

        var errorDescription: String? {
            switch self {
            case .notVault: "The folder has no 项目 folder, so it is not a Studiolo vault."
            case let .invalidName(reason): reason
            case let .exists(name): "An archive named \(name) already exists."
            case .templateMissing: "The vault has no archive skeleton at \(StudioloIndex.templateFolder)."
            case let .copy(reason): "Could not copy the skeleton: \(reason)"
            case let .fill(reason): "Could not fill in the dashboard: \(reason)"
            case let .move(reason): "Could not move the archive into the vault: \(reason)"
            }
        }
    }

    /// Characters that break a folder name in Finder or a wiki link in Obsidian.
    private static let forbidden = CharacterSet(charactersIn: "/\\:*?\"<>|#^[]")
    private static let nameLimit = 80

    static func archiveName(_ name: String) -> String {
        name.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    /// Why the name cannot be used, or nil when it can.
    static func validate(name: String, vault: URL) -> Failure? {
        guard StudioloVault.isVault(vault) else { return .notVault }
        let name = archiveName(name)
        if name.isEmpty { return .invalidName("The archive needs a name.") }
        if name.hasPrefix(".") { return .invalidName("The name cannot start with a dot.") }
        if name.count > nameLimit { return .invalidName("The name is longer than \(nameLimit) characters.") }
        if name.unicodeScalars.contains(where: { forbidden.contains($0) || CharacterSet.controlCharacters.contains($0) }) {
            return .invalidName("The name cannot contain / \\ : * ? \" < > | # ^ [ ]")
        }
        let destination = vault.appendingPathComponent(StudioloIndex.projectsFolder).appendingPathComponent(name)
        if FileManager.default.fileExists(atPath: destination.path) { return .exists(name) }
        return nil
    }

    static func create(_ request: Request) throws(Failure) -> Outcome {
        if let failure = validate(name: request.name, vault: request.vault) { throw failure }
        let name = archiveName(request.name)
        let fileManager = FileManager.default
        let template = request.vault.appendingPathComponent(StudioloIndex.templateFolder)
        var isDirectory: ObjCBool = false
        guard fileManager.fileExists(atPath: template.path, isDirectory: &isDirectory), isDirectory.boolValue else {
            throw .templateMissing
        }

        // A scratch folder on the vault's own volume, so the final move is a rename.
        let scratch: URL
        do {
            scratch = try fileManager.url(for: .itemReplacementDirectory, in: .userDomainMask, appropriateFor: request.vault, create: true)
        } catch {
            throw .copy(error.localizedDescription)
        }
        defer { try? fileManager.removeItem(at: scratch) }
        let staged = scratch.appendingPathComponent(name)

        do {
            try fileManager.copyItem(at: template, to: staged)
        } catch {
            throw .copy(error.localizedDescription)
        }

        // The rules file itself, not a link to it: the archive keeps a snapshot, and a link
        // copied as it stands would follow every later edit (or dangle on another device).
        let rules = request.globalRules.resolvingSymlinksInPath()
        var missingRules = true
        if fileManager.fileExists(atPath: rules.path) {
            do {
                try fileManager.copyItem(at: rules, to: staged.appendingPathComponent("CLAUDE.md"))
                missingRules = false
            } catch {
                throw .copy(error.localizedDescription)
            }
        }

        let index = staged.appendingPathComponent(StudioloIndex.indexFile)
        do {
            let text = try String(contentsOf: index, encoding: .utf8)
            guard let filled = StudioloIndex.filledTemplate(
                text, name: name, summary: request.summary, projectPath: request.projectPath, today: request.today
            ) else {
                throw Failure.fill("the project path contains a | or ` and cannot go in the dashboard table")
            }
            try filled.write(to: index, atomically: true, encoding: .utf8)
        } catch let failure as Failure {
            throw failure
        } catch {
            throw .fill(error.localizedDescription)
        }

        let destination = request.vault.appendingPathComponent(StudioloIndex.projectsFolder).appendingPathComponent(name)
        let moved = staged.path.withCString { from in
            destination.path.withCString { to in renamex_np(from, to, UInt32(RENAME_EXCL)) }
        }
        if moved != 0 {
            let code = errno
            if code == EEXIST { throw .exists(name) }
            throw .move(String(cString: strerror(code)))
        }
        return Outcome(prefix: StudioloIndex.projectsFolder + "/" + name, missingRules: missingRules)
    }
}
