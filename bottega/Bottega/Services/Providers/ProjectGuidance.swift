// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation

/// Root-level project guidelines for the native API sessions.
///
/// A native API session builds its system prompt from scratch, so the rules a
/// project keeps in `AGENTS.md` or `CLAUDE.md` would never reach it. This reads
/// exactly those two files at the working directory's root — never a parent,
/// never a repository scan — and renders them as one system-prompt block. The
/// block is re-rendered only when a file's size or modification date changed, so
/// a turn that resumes after an edit carries the new rules without the prompt
/// being rebuilt on every request.
///
/// A file is injected whole or not at all: where one is too large for the prompt
/// budget the block names it and says to read it with the tool, so no rule is
/// silently dropped.
struct ProjectGuidance: Sendable {
    /// The two files a project keeps its own guidance in, in injection order.
    static let fileNames = ["AGENTS.md", "CLAUDE.md"]
    /// The most bytes of any one file that are injected.
    static let perFileLimit = 16_000
    /// The most bytes all the files together contribute to the system prompt.
    static let totalLimit = 24_000

    struct FileState: Equatable, Sendable {
        var size: Int
        var modified: Date?
    }

    let workingDirectory: String
    private var states: [String: FileState] = [:]
    private var block: String?
    private var didLoad = false

    init(workingDirectory: String) {
        self.workingDirectory = workingDirectory
    }

    /// The system-prompt block for the current files, refreshed only when one of
    /// them changed since the last call. Nil when the project has neither file.
    mutating func systemPromptBlock() -> String? {
        let current = Self.states(in: workingDirectory)
        guard !didLoad || current != states else { return block }
        didLoad = true
        states = current
        block = Self.render(in: workingDirectory)
        return block
    }

    private static func states(in directory: String) -> [String: FileState] {
        var result: [String: FileState] = [:]
        for name in fileNames {
            guard let url = resolvedFile(named: name, in: directory),
                  let attributes = try? FileManager.default.attributesOfItem(atPath: url.path) else { continue }
            result[name] = FileState(
                size: (attributes[.size] as? NSNumber)?.intValue ?? 0,
                modified: attributes[.modificationDate] as? Date
            )
        }
        return result
    }

    /// The file's real URL inside the project, or nil when it is absent or a
    /// symlink whose target leaves the working directory. The repository is
    /// never scanned, only these names at the root are looked up.
    private static func resolvedFile(named name: String, in directory: String) -> URL? {
        let resolvedRoot = URL(fileURLWithPath: directory).standardizedFileURL.resolvingSymlinksInPath().path
        let candidate = URL(fileURLWithPath: directory).appendingPathComponent(name).resolvingSymlinksInPath()
        let rootPrefix = resolvedRoot.hasSuffix("/") ? resolvedRoot : resolvedRoot + "/"
        guard candidate.path == resolvedRoot || candidate.path.hasPrefix(rootPrefix) else { return nil }
        var isDirectory: ObjCBool = false
        guard FileManager.default.fileExists(atPath: candidate.path, isDirectory: &isDirectory), !isDirectory.boolValue,
              let attributes = try? FileManager.default.attributesOfItem(atPath: candidate.path),
              attributes[.type] as? FileAttributeType == .typeRegular else { return nil }
        return candidate
    }

    private static func render(in directory: String) -> String? {
        var sections: [String] = []
        var remaining = totalLimit
        for name in fileNames {
            guard let url = resolvedFile(named: name, in: directory) else { continue }
            guard let data = boundedData(at: url) else {
                sections.append("Could not load \(name). Read it with the read_file tool before changing the project.")
                continue
            }
            let size = (try? FileManager.default.attributesOfItem(atPath: url.path)[.size] as? NSNumber)?.intValue ?? data.count
            // Oversized or out of budget: the name and the instruction to read it
            // go in, so the model knows rules exist rather than never hearing of them.
            guard data.count <= perFileLimit, data.count <= remaining else {
                sections.append("\(name) is too large to include here (\(size) bytes). Read \(name) with the read_file tool; it may contain rules the project expects you to follow.")
                continue
            }
            guard let text = String(data: data, encoding: .utf8),
                  !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { continue }
            remaining -= data.count
            sections.append("--- \(name) ---\n\(text)")
        }
        guard !sections.isEmpty else { return nil }
        return """
        Project guidelines from \(directory). They supplement the rules above; where they conflict, the Bottega rules take priority.
        \(sections.joined(separator: "\n\n"))
        """
    }

    /// At most `perFileLimit + 1` bytes, so an oversized file is detected without
    /// reading it whole.
    private static func boundedData(at url: URL) -> Data? {
        guard let handle = try? FileHandle(forReadingFrom: url) else { return nil }
        defer { try? handle.close() }
        var data = Data()
        while data.count <= perFileLimit {
            do {
                guard let chunk = try handle.read(upToCount: min(65_536, perFileLimit + 1 - data.count)), !chunk.isEmpty else { break }
                data.append(chunk)
            } catch { return nil }
        }
        return data
    }
}
