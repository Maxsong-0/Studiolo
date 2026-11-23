// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation
import os

/// A thread's session log: one stamped line for each thing that happened to the thread's
/// files outside its agent's sight, such as a turn's changes undone from the timeline. The
/// agent hears of the lines it has not seen at the start of its next turn, with the path to
/// this file for the whole story (see `ThreadRuntime.startTurn`); the file is plain Markdown
/// in app support beside the thread files, and the agent's own tools read it by that path.
/// It goes with its thread (see `Storage.deleteDocuments`).
enum SessionLog {
    private static let log = Logger(subsystem: AppInfo.logSubsystem, category: "session-log")

    static func url(for thread: UUID) -> URL {
        Storage.sessionLogsDirectory.appendingPathComponent("\(thread.uuidString).md")
    }

    /// Adds `lines` to the thread's log, each with the time it happened, and answers with the
    /// stamped lines, the words the agent is to be told. A log that cannot be written is
    /// said so in the unified log; the lines still reach the agent with its next turn.
    @discardableResult
    static func append(_ lines: [String], to thread: UUID, at date: Date = .now) -> [String] {
        // Local time to the minute, the way the user reads a clock.
        let parts = Calendar.current.dateComponents([.year, .month, .day, .hour, .minute], from: date)
        let stamp = String(format: "%04d-%02d-%02d %02d:%02d", parts.year ?? 0, parts.month ?? 0, parts.day ?? 0, parts.hour ?? 0, parts.minute ?? 0)
        let stamped = lines.map { "- \(stamp) \($0)" }
        guard !stamped.isEmpty else { return [] }
        let file = url(for: thread)
        let text = stamped.joined(separator: "\n") + "\n"
        do {
            if FileManager.default.fileExists(atPath: file.path) {
                let handle = try FileHandle(forWritingTo: file)
                defer { try? handle.close() }
                try handle.seekToEnd()
                try handle.write(contentsOf: Data(text.utf8))
            } else {
                try Data((header + text).utf8).write(to: file, options: .atomic)
            }
        } catch {
            log.error("Could not write the session log of \(thread.uuidString, privacy: .public): \(error.localizedDescription, privacy: .public)")
        }
        return stamped
    }

    /// What the agent is told at the start of a turn: the lines it has not seen, and where the
    /// rest is.
    static func note(_ lines: [String], thread: UUID) -> String {
        """
        Since your last turn, outside your own tools (session log, newest last):
        \(lines.joined(separator: "\n"))
        The whole log is at \(url(for: thread).path). Read the files again before relying on what you last saw of them.
        """
    }

    private static let header = """
    # Session log

    Things that happened to this thread's files outside the agent's own tools, newest last.

    """
}
