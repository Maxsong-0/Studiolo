import Foundation

struct DiffFile: Identifiable, Hashable, Sendable {
    enum Change: Sendable {
        case added
        case deleted
        case modified
        case renamed
    }

    var path: String
    var oldPath: String?
    var change: Change
    var isBinary: Bool
    var hunks: [DiffHunk]
    var reportedAdditions: Int?
    var reportedDeletions: Int?

    var id: String { path }
    var additions: Int { reportedAdditions ?? hunks.reduce(0) { $0 + $1.lines.count(where: { $0.kind == .addition }) } }
    var deletions: Int { reportedDeletions ?? hunks.reduce(0) { $0 + $1.lines.count(where: { $0.kind == .deletion }) } }
    var name: String { (path as NSString).lastPathComponent }
    var directory: String {
        let parent = (path as NSString).deletingLastPathComponent
        return parent.isEmpty ? "" : parent + "/"
    }
}

struct DiffHunk: Identifiable, Hashable, Sendable {
    var id: Int
    var header: String
    var lines: [DiffLine]
}

struct DiffLine: Identifiable, Hashable, Sendable {
    enum Kind: Sendable {
        case context
        case addition
        case deletion
        case note
    }

    var id: Int
    var kind: Kind
    var text: String
    var oldNumber: Int?
    var newNumber: Int?
}

enum DiffParser {
    /// Parses `git diff` output into files, hunks and numbered lines.
    static func parse(_ patch: String) -> [DiffFile] {
        var files: [DiffFile] = []
        var current: DiffFile?
        var hunk: DiffHunk?
        var oldLine = 0
        var newLine = 0
        var lineID = 0

        func closeHunk() {
            if let finished = hunk { current?.hunks.append(finished) }
            hunk = nil
        }

        func closeFile() {
            closeHunk()
            if let finished = current { files.append(finished) }
            current = nil
        }

        for rawLine in patch.split(separator: "\n", omittingEmptySubsequences: false) {
            let line = String(rawLine)
            if line.hasPrefix("diff --git ") {
                closeFile()
                current = DiffFile(path: headerPath(line), change: .modified, isBinary: false, hunks: [])
                continue
            }
            guard current != nil else { continue }

            if hunk == nil || line.hasPrefix("@@") {
                if line.hasPrefix("new file mode") {
                    current?.change = .added
                } else if line.hasPrefix("deleted file mode") {
                    current?.change = .deleted
                } else if line.hasPrefix("rename from ") {
                    current?.change = .renamed
                    current?.oldPath = String(line.dropFirst("rename from ".count))
                } else if line.hasPrefix("rename to ") {
                    current?.path = String(line.dropFirst("rename to ".count))
                } else if line.hasPrefix("Binary files") || line.hasPrefix("GIT binary patch") {
                    current?.isBinary = true
                } else if line.hasPrefix("--- ") {
                    if line == "--- /dev/null" { current?.change = .added }
                } else if line.hasPrefix("+++ ") {
                    if line == "+++ /dev/null" {
                        current?.change = .deleted
                    } else {
                        current?.path = stripPrefix(String(line.dropFirst(4)))
                    }
                } else if line.hasPrefix("@@") {
                    closeHunk()
                    let numbers = hunkStarts(line)
                    oldLine = numbers.old
                    newLine = numbers.new
                    lineID += 1
                    hunk = DiffHunk(id: lineID, header: line, lines: [])
                }
                continue
            }

            lineID += 1
            if line.hasPrefix("+") {
                hunk?.lines.append(DiffLine(id: lineID, kind: .addition, text: String(line.dropFirst()), newNumber: newLine))
                newLine += 1
            } else if line.hasPrefix("-") {
                hunk?.lines.append(DiffLine(id: lineID, kind: .deletion, text: String(line.dropFirst()), oldNumber: oldLine))
                oldLine += 1
            } else if line.hasPrefix("\\") {
                hunk?.lines.append(DiffLine(id: lineID, kind: .note, text: String(line.dropFirst(2))))
            } else if line.hasPrefix(" ") || line.isEmpty {
                if line.isEmpty, rawLine.endIndex == patch.endIndex { continue }
                hunk?.lines.append(DiffLine(id: lineID, kind: .context, text: String(line.dropFirst()), oldNumber: oldLine, newNumber: newLine))
                oldLine += 1
                newLine += 1
            }
        }
        closeFile()
        return files
    }

    /// Parses bare hunks, such as a provider's per-file diff, for a known path.
    static func parseHunks(_ text: String, path: String) -> DiffFile {
        let body = text.hasPrefix("diff --git") ? text : "diff --git a/\(path) b/\(path)\n" + text
        var file = parse(body).first ?? DiffFile(path: path, change: .modified, isBinary: false, hunks: [])
        file.path = path
        return file
    }

    private static func headerPath(_ line: String) -> String {
        let rest = line.dropFirst("diff --git ".count)
        if let range = rest.range(of: " b/", options: .backwards) {
            return String(rest[range.upperBound...])
        }
        return stripPrefix(String(rest))
    }

    private static func stripPrefix(_ path: String) -> String {
        var value = path
        if value.hasPrefix("\""), value.hasSuffix("\""), value.count >= 2 {
            value = String(value.dropFirst().dropLast())
        }
        if value.hasPrefix("a/") || value.hasPrefix("b/") { value.removeFirst(2) }
        return value
    }

    private static func hunkStarts(_ header: String) -> (old: Int, new: Int) {
        let parts = header.split(separator: " ")
        func start(_ token: Substring?) -> Int {
            guard let token else { return 1 }
            let digits = token.dropFirst().split(separator: ",").first ?? "1"
            return Int(digits) ?? 1
        }
        let old = parts.first { $0.hasPrefix("-") }
        let new = parts.first { $0.hasPrefix("+") }
        return (start(old), start(new))
    }
}
