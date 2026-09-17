import Foundation

struct NativeFileTools: Sendable {
    let workingDirectory: String

    private static let readLimit = 1_000_000
    private static let mutations = Mutations()

    private actor Mutations {
        func run(_ operation: @Sendable () throws -> Void) rethrows {
            try operation()
        }
    }
    private static let skippedDirectories: Set<String> = [".git", "node_modules", ".build", "build", "dist", "DerivedData", "Pods", "target", ".venv", "venv"]

    private func resolveURL(_ path: String) throws -> URL {
        let trimmed = path.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { throw ProviderError.failed("A file path is required.") }
        let root = URL(fileURLWithPath: workingDirectory).standardizedFileURL
        let candidate: URL = trimmed.hasPrefix("/") || trimmed.hasPrefix("~")
            ? URL(fileURLWithPath: (trimmed as NSString).expandingTildeInPath)
            : root.appendingPathComponent(trimmed)
        let standardized = candidate.standardizedFileURL
        guard standardized.path == root.path || standardized.path.hasPrefix(root.path + "/") else {
            throw ProviderError.failed("That path is outside the project. Stay inside \(workingDirectory).")
        }
        return standardized
    }

    func displayPath(_ path: String) -> String {
        let trimmed = path.trimmingCharacters(in: .whitespacesAndNewlines)
        if trimmed.isEmpty { return workingDirectory }
        if trimmed.hasPrefix(workingDirectory) { return ToolTitles.relativePath(trimmed, to: workingDirectory) }
        return trimmed
    }

    @concurrent func readFile(_ path: String, startLine: Int? = nil, lineCount: Int? = nil) async throws -> String {
        try Task.checkCancellation()
        let url = try resolveURL(path)
        guard FileManager.default.fileExists(atPath: url.path) else {
            throw ProviderError.failed("No such file: \(displayPath(path)).")
        }
        let handle = try FileHandle(forReadingFrom: url)
        defer { try? handle.close() }
        var data = Data()
        while data.count <= Self.readLimit {
            try Task.checkCancellation()
            let count = min(65_536, Self.readLimit + 1 - data.count)
            guard let chunk = try handle.read(upToCount: count), !chunk.isEmpty else { break }
            data.append(chunk)
        }
        try Task.checkCancellation()
        guard data.count <= Self.readLimit else {
            let byteCount = try handle.seekToEnd()
            throw ProviderError.failed("That file is too large to read (\(byteCount) bytes).")
        }
        guard let text = String(data: data, encoding: .utf8) ?? String(data: data, encoding: .isoLatin1) else {
            throw ProviderError.failed("That file is not readable as text.")
        }
        if startLine != nil || lineCount != nil {
            let lines = text.split(separator: "\n", omittingEmptySubsequences: false)
            let first = max(1, startLine ?? 1)
            guard first <= lines.count else { return "(the file has \(lines.count) lines; start_line \(first) is past the end)" }
            let count = max(1, lineCount ?? 200)
            let last = first - 1 + min(count, lines.count - first + 1)
            let slice = lines[(first - 1)..<last].joined(separator: "\n")
            return "(lines \(first)-\(last) of \(lines.count))\n" + slice
        }
        // Bytes bound characters, so a file under the limit skips the grapheme count.
        guard text.utf8.count > 60_000 else { return text }
        let characterCount = text.count
        if characterCount > 60_000 { return String(text.prefix(60_000)) + "\n…(truncated, \(characterCount) chars total; read the rest with start_line and line_count)" }
        return text
    }

    @concurrent func writeFile(path: String, content: String) async throws {
        try Task.checkCancellation()
        try await Self.mutations.run {
            try Task.checkCancellation()
            let url = try resolveURL(path)
            guard let data = content.data(using: .utf8) else { throw ProviderError.failed("Could not encode that content.") }
            try Task.checkCancellation()
            try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
            try Task.checkCancellation()
            try data.write(to: url, options: .atomic)
        }
    }

    @concurrent func editFile(path: String, old: String, new: String) async throws {
        try Task.checkCancellation()
        try await Self.mutations.run {
            try Task.checkCancellation()
            guard !old.isEmpty else { throw ProviderError.failed("old_string must not be empty. Read the file first, then edit an exact block.") }
            let url = try resolveURL(path)
            let current = try String(contentsOf: url, encoding: .utf8) as NSString
            var remaining = NSRange(location: 0, length: current.length)
            var firstMatch = NSRange(location: NSNotFound, length: 0)
            var occurrences = 0
            while remaining.length > 0 {
                try Task.checkCancellation()
                let match = current.range(of: old, options: [], range: remaining)
                guard match.location != NSNotFound else { break }
                if occurrences == 0 { firstMatch = match }
                occurrences += 1
                remaining.location = NSMaxRange(match)
                remaining.length = current.length - remaining.location
            }
            try Task.checkCancellation()
            guard occurrences == 1 else {
                if occurrences == 0 { throw ProviderError.failed("That exact text was not found in \(displayPath(path)). Read the file and copy it exactly.") }
                throw ProviderError.failed("That text appears \(occurrences) times in \(displayPath(path)). Include more context so it matches once.")
            }
            let replacement = current.replacingCharacters(in: firstMatch, with: new)
            try Task.checkCancellation()
            try replacement.write(to: url, atomically: true, encoding: .utf8)
        }
    }

    @concurrent func listFiles(_ path: String, recursive: Bool) async throws -> String {
        try Task.checkCancellation()
        let base: URL = path.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            ? URL(fileURLWithPath: workingDirectory)
            : try resolveURL(path)
        var isDirectory: ObjCBool = false
        guard FileManager.default.fileExists(atPath: base.path, isDirectory: &isDirectory), isDirectory.boolValue else {
            throw ProviderError.failed("No such directory: \(displayPath(path.isEmpty ? workingDirectory : path)).")
        }
        var results: [String] = []
        results.reserveCapacity(301)
        if recursive {
            guard let enumerator = FileManager.default.enumerator(at: base, includingPropertiesForKeys: [.isDirectoryKey], options: [.skipsHiddenFiles, .skipsPackageDescendants]) else {
                return "(empty)"
            }
            while let url = enumerator.nextObject() as? URL {
                try Task.checkCancellation()
                if Self.skippedDirectories.contains(url.lastPathComponent) { enumerator.skipDescendants(); continue }
                if (try? url.resourceValues(forKeys: [.isDirectoryKey]))?.isDirectory == true { continue }
                results.append(ToolTitles.relativePath(url.path, to: workingDirectory))
                if results.count >= 300 { results.append("…(truncated)"); break }
            }
        } else {
            let items = (try? FileManager.default.contentsOfDirectory(atPath: base.path)) ?? []
            try Task.checkCancellation()
            for item in items.sorted() {
                try Task.checkCancellation()
                guard !Self.skippedDirectories.contains(item), !item.hasPrefix(".DS_Store") else { continue }
                var isDir: ObjCBool = false
                let full = base.appendingPathComponent(item).path
                FileManager.default.fileExists(atPath: full, isDirectory: &isDir)
                results.append(ToolTitles.relativePath(full, to: workingDirectory) + (isDir.boolValue ? "/" : ""))
                if results.count >= 300 { results.append("…(truncated)"); break }
            }
        }
        try Task.checkCancellation()
        return results.isEmpty ? "(empty)" : results.joined(separator: "\n")
    }
}
