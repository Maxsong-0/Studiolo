import Foundation

enum AppInfo {
    static let name = Bundle.main.object(forInfoDictionaryKey: "CFBundleDisplayName") as? String ?? "Droppy Code"
    static let isDevelopment = Bundle.main.bundleIdentifier == "iordv.droppycode.dev"
    static let stateDirectoryName = isDevelopment ? ".droppy-code-dev" : ".droppy-code"
    static let version = Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "1.0.0"
    static let build = Bundle.main.object(forInfoDictionaryKey: "CFBundleVersion") as? String ?? "1"
}

enum TextCleanup {
    static func stripANSI(_ text: String) -> String {
        text.replacing(#/\x1B\[[0-9;?]*[ -/]*[@-~]/#, with: "")
    }

    /// The last meaningful lines of process output, for error messages.
    static func lastLines(_ text: String, count: Int = 4) -> String? {
        let lines = stripANSI(text)
            .split(whereSeparator: \.isNewline)
            .map { $0.trimmingCharacters(in: .whitespaces) }
            .filter { !$0.isEmpty }
        guard !lines.isEmpty else { return nil }
        return lines.suffix(count).joined(separator: "\n")
    }

    static func singleLine(_ text: String, limit: Int = 120) -> String {
        let remaining = text.drop(while: \.isNewline)
        let end = remaining.firstIndex(where: \.isNewline) ?? remaining.endIndex
        let line = remaining[..<end].trimmingCharacters(in: .whitespaces)
        return line.count > limit ? String(line.prefix(limit - 1)) + "…" : line
    }

    /// Model prose never ships an em dash. It is the tell of a machine-written
    /// paragraph, so every string a model writes for the thread passes through here:
    /// replies as they stream and as they land, thinking, plans, generated titles and
    /// commit messages. The dash becomes a plain hyphen with the spacing around it kept,
    /// so "works — every time" reads "works - every time" and "ships—sometimes" reads
    /// "ships-sometimes".
    ///
    /// The en dash stays: it is the correct mark in a range, as in "lines 10–20".
    static func withoutEmDashes(_ text: String) -> String {
        guard mayContainEmDash(text), text.contains(where: { isEmDash($0) }) else { return text }
        return String(text.map { isEmDash($0) ? "-" : $0 })
    }

    /// Whether any dash scalar is in the bytes, checked without walking graphemes: every
    /// streamed delta and every stored message passes through here, and model prose is
    /// full of other E2-led scalars (curly quotes, ellipses, en dashes) that made the
    /// lead-byte test alone let most of it through to the slow path.
    private static func mayContainEmDash(_ text: String) -> Bool {
        var utf8 = text.utf8.makeIterator()
        while let byte = utf8.next() {
            guard byte == 0xE2, let second = utf8.next() else { continue }
            if second == 0x80 {
                // U+2014 and U+2015: E2 80 94, E2 80 95.
                guard let third = utf8.next() else { return false }
                if third == 0x94 || third == 0x95 { return true }
            } else if second == 0xB8 {
                // U+2E3A and U+2E3B: E2 B8 BA, E2 B8 BB.
                guard let third = utf8.next() else { return false }
                if third == 0xBA || third == 0xBB { return true }
            }
        }
        return false
    }

    /// The em dash and its longer relatives: the horizontal bar, and the two- and
    /// three-em dashes.
    private static func isEmDash(_ character: Character) -> Bool {
        switch character {
        case "\u{2014}", "\u{2015}", "\u{2E3A}", "\u{2E3B}": true
        default: false
        }
    }
}

enum SimpleDiff {
    /// Past this much text a reported edit keeps its line counts and drops the hunk: the diff
    /// of a whole generated file would otherwise sit in the thread document forever and be
    /// encoded with every save. The same cap bounds command diffs (`ThreadRuntime.fileEdits`).
    static let diffLimit = 200_000

    /// A compact unified hunk between two texts, trimming the unchanged head and tail.
    static func unified(old: String, new: String) -> FileEdit.Stats {
        let oldLines = old.isEmpty ? [] : old.components(separatedBy: "\n")
        let newLines = new.isEmpty ? [] : new.components(separatedBy: "\n")
        var prefix = 0
        while prefix < oldLines.count, prefix < newLines.count, oldLines[prefix] == newLines[prefix] {
            prefix += 1
        }
        var suffix = 0
        while suffix < oldLines.count - prefix, suffix < newLines.count - prefix,
              oldLines[oldLines.count - 1 - suffix] == newLines[newLines.count - 1 - suffix] {
            suffix += 1
        }
        let removed = oldLines[prefix..<(oldLines.count - suffix)]
        let added = newLines[prefix..<(newLines.count - suffix)]
        let leading = oldLines[max(0, prefix - 3)..<prefix]
        let trailingEnd = min(oldLines.count, oldLines.count - suffix + 3)
        let trailing = oldLines[(oldLines.count - suffix)..<trailingEnd]
        guard old.utf8.count + new.utf8.count <= diffLimit else {
            return FileEdit.Stats(diff: nil, additions: added.count, deletions: removed.count)
        }
        let oldStart = prefix - leading.count + 1
        var lines = ["@@ -\(oldStart),\(leading.count + removed.count + trailing.count) +\(oldStart),\(leading.count + added.count + trailing.count) @@"]
        lines += leading.map { " " + $0 }
        lines += removed.map { "-" + $0 }
        lines += added.map { "+" + $0 }
        lines += trailing.map { " " + $0 }
        return FileEdit.Stats(diff: lines.joined(separator: "\n"), additions: added.count, deletions: removed.count)
    }
}

extension FileEdit {
    struct Stats {
        var diff: String?
        var additions: Int
        var deletions: Int
    }

    init(path: String, old: String, new: String) {
        let stats = SimpleDiff.unified(old: old, new: new)
        self.init(path: path, diff: stats.diff, additions: stats.additions, deletions: stats.deletions)
    }
}

extension String {
    var nilIfEmpty: String? { isEmpty ? nil : self }
}
