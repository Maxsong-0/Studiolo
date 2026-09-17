import Foundation

/// The files undoing a thread's changes would put back, with the counts of what it did to
/// them. A file the checkout has changed again since the thread last left it is somebody
/// else's work now: it is listed, marked kept, and left alone.
struct RevertPreview: Sendable {
    struct File: Identifiable, Sendable {
        let path: String
        let additions: Int
        let deletions: Int
        let isBinary: Bool
        var isKept = false

        var id: String { path }
    }

    var files: [File]

    init(files: [File]) {
        self.files = files
    }

    /// The files that will actually be put back.
    var restorable: [File] { files.filter { !$0.isKept } }
    var additions: Int { restorable.reduce(0) { $0 + $1.additions } }
    var deletions: Int { restorable.reduce(0) { $0 + $1.deletions } }

    /// From `git diff --numstat -z`: added, removed and path per file, `-` counts for binaries.
    init(numstat: String) throws {
        var files: [File] = []
        for entry in numstat.split(separator: "\0") {
            let fields = entry.split(separator: "\t", maxSplits: 2, omittingEmptySubsequences: false)
            guard fields.count == 3, !fields[2].isEmpty else { throw CocoaError(.fileReadCorruptFile) }
            let binary = fields[0] == "-" && fields[1] == "-"
            guard let added = binary ? 0 : Int(fields[0]), let removed = binary ? 0 : Int(fields[1]), added >= 0, removed >= 0 else {
                throw CocoaError(.fileReadCorruptFile)
            }
            files.append(File(path: String(fields[2]), additions: added, deletions: removed, isBinary: binary))
        }
        self.files = files
    }
}
