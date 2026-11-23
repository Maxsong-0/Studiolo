// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation

/// The rules file a Crew lead reads after the instructions typed in Settings › Crew
/// (`AppSettings.crewLeadInstructionsFile`): what it held the last time it was read, or why
/// it could not be read. It is read off the main thread, and again before each message a
/// lead gets (`ThreadRuntime.ensureSession`), so a file kept up to date elsewhere, a
/// CLAUDE.md or a note in a vault, reaches the lead as it is now rather than as it was
/// when it was chosen. Core only ever sees the text (`CrewLaunch.leadInstructions`).
struct CrewLeadInstructionsFile: Equatable, Sendable {
    enum Content: Equatable, Sendable {
        /// The file's text, trimmed; empty when the file is.
        case text(String)
        /// Why the file could not be read, in words for Settings.
        case unreadable(String)
    }

    /// The path as Settings holds it.
    var path: String
    var content: Content

    /// The most a rules file may hold. The rules ride in every lead's system prompt, and on
    /// a provider without one in front of every message, so a file picked by mistake (a
    /// log, a dump) must not fill the lead's context.
    static let byteLimit = 64 * 1024

    /// The file's text, when it has any.
    var text: String? {
        guard case .text(let text) = content, !text.isEmpty else { return nil }
        return text
    }

    /// Reads the file at `path` as UTF-8 text, or says why it cannot be read. Blocking:
    /// call it off the main thread.
    static func read(_ path: String) -> CrewLeadInstructionsFile {
        func unreadable(_ reason: String) -> CrewLeadInstructionsFile {
            CrewLeadInstructionsFile(path: path, content: .unreadable(reason))
        }
        let url = URL(fileURLWithPath: (path as NSString).expandingTildeInPath)
        guard let values = try? url.resourceValues(forKeys: [.isDirectoryKey, .fileSizeKey]) else {
            return unreadable("No file at this path")
        }
        if values.isDirectory == true { return unreadable("This is a folder, not a file") }
        // The size is checked before the read, so a huge file is never loaded to be refused.
        if let size = values.fileSize, size > byteLimit {
            return unreadable("Over \(byteLimit / 1024) KB, too long to send with every lead's prompt")
        }
        let data: Data
        do {
            data = try Data(contentsOf: url)
        } catch {
            return unreadable(error.localizedDescription)
        }
        guard data.count <= byteLimit else {
            return unreadable("Over \(byteLimit / 1024) KB, too long to send with every lead's prompt")
        }
        guard let text = String(data: data, encoding: .utf8) else { return unreadable("Not UTF-8 text") }
        return CrewLeadInstructionsFile(path: path, content: .text(text.trimmingCharacters(in: .whitespacesAndNewlines)))
    }
}
