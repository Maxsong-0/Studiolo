// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation

/// A three-way merge of three versions of one file, done by `git merge-file` on copies in a
/// folder of its own: no working file, index or repository is touched, and the copies go as
/// soon as git has answered. What `ours` and `theirs` each changed against `base` is kept;
/// where both changed the same lines, the result has conflicts and is not to be written
/// anywhere as it stands.
///
/// Undoing a turn's changes is this merge read backwards (see `Git.revertFiles`): the base
/// is the file as the turn left it, ours the file as it is now, theirs the file as it was
/// before the turn, so the turn's own changes come out and everything done since stays. The
/// archive page's save is the plain way round: the base is the file as it was opened.
enum MergeFile {
    struct Result: Sendable {
        let merged: Data
        let hasConflicts: Bool
    }

    static func merge(base: Data, ours: Data, theirs: Data) async throws -> Result {
        let folder = FileManager.default.temporaryDirectory.appendingPathComponent("bottega-merge-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: folder) }
        let oursURL = folder.appendingPathComponent("ours")
        let baseURL = folder.appendingPathComponent("base")
        let theirsURL = folder.appendingPathComponent("theirs")
        try ours.write(to: oursURL)
        try base.write(to: baseURL)
        try theirs.write(to: theirsURL)
        let result = try await Shell.run(
            LoginEnvironment.git,
            ["merge-file", "-p", oursURL.path, baseURL.path, theirsURL.path],
            in: folder,
            environment: LoginEnvironment.current,
            timeout: 60
        )
        // The exit status is the number of conflicts, 127 at most; git failing exits with
        // a negative number, which the shell reports as 255.
        guard (0...127).contains(result.status) else { throw ShellError(result.failureMessage) }
        return Result(merged: result.stdout, hasConflicts: result.status > 0)
    }
}
