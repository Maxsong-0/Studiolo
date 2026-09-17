import Foundation

struct FileChangeSummary: Codable, Hashable, Sendable {
    struct File: Codable, Hashable, Sendable {
        var path: String
        var additions: Int
        var deletions: Int
        var isBinary: Bool
    }

    let files: [File]
    let additions: Int
    let deletions: Int

    init(files: [DiffFile]) {
        var entries: [File] = []
        entries.reserveCapacity(files.count)
        var additions = 0
        var deletions = 0
        for file in files {
            let added = file.additions
            let deleted = file.deletions
            entries.append(File(path: file.path, additions: added, deletions: deleted, isBinary: file.isBinary))
            additions += added
            deletions += deleted
        }
        self.files = entries
        self.additions = additions
        self.deletions = deletions
    }
}
