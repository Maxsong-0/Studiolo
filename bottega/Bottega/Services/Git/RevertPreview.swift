// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation

/// The files undoing a stretch of a thread's turns would touch, with the counts of what the
/// stretch did to them and what undoing it would do to each (see `Git.previewFileRevert`).
/// A file changed again since keeps those later changes: only the stretch's own come out,
/// and a file where the two cannot be told apart is left as it is.
struct RevertPreview: Sendable {
    struct File: Identifiable, Sendable {
        /// What undoing the stretch does to one file.
        enum Outcome: Sendable, Equatable {
            /// The stretch's changes come out; anything done since stays.
            case revert
            /// Changed again since in a way that overlaps the stretch's changes, or in a
            /// file whose content cannot be merged (a binary, a link, one created or deleted
            /// by the stretch and changed since): left exactly as it is.
            case conflict
            /// Already as it was before the stretch: nothing to do.
            case unchanged
        }

        let path: String
        let additions: Int
        let deletions: Int
        let isBinary: Bool
        var outcome: Outcome = .revert

        var id: String { path }
    }

    var files: [File]
    /// Where the checkout sits in its repository, as git names it: `sub/dir/` below the
    /// root, nothing at it. A turn records the files its agent changed from the checkout,
    /// while git names every file from the root; this is the difference (see `names`).
    var checkoutPrefix = ""

    init(files: [File], checkoutPrefix: String = "") {
        self.files = files
        self.checkoutPrefix = checkoutPrefix
    }

    /// Whether `path`, as git names it from the root, is the file `recorded` names, from the
    /// root or from the checkout. Never a file of the same name elsewhere: `README.md` in
    /// the checkout is not `docs/README.md`.
    func names(_ path: String, as recorded: String) -> Bool {
        path == recorded || (!checkoutPrefix.isEmpty && path == checkoutPrefix + recorded)
    }

    /// The files that will actually change.
    var restorable: [File] { files.filter { $0.outcome == .revert } }
    /// The files left alone because later changes overlap the stretch's.
    var conflicted: [File] { files.filter { $0.outcome == .conflict } }
    var additions: Int { restorable.reduce(0) { $0 + $1.additions } }
    var deletions: Int { restorable.reduce(0) { $0 + $1.deletions } }

    /// From `git diff --numstat -z`: added, removed and path per file, `-` counts for binaries.
    init(numstat: String, checkoutPrefix: String = "") throws {
        self.checkoutPrefix = checkoutPrefix
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
