// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation

extension FileChangeSummary {
    /// The summary of a parsed diff.
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
        self.init(files: entries, additions: additions, deletions: deletions)
    }
}
