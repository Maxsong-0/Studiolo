// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation

/// A project's archive: the notes every formal project of the user's keeps in a folder of
/// fixed shape (an INDEX under `00-总览/`, the coordination rows in `04-协调/TASKS.md`, a
/// log a day under `02-日志/`), in the project itself or, as for most of them, in a notes
/// vault beside it. A lead closes its coordination rows and writes the day's log there,
/// and a hand edits its own row when its brief says so: that is work on the record, not
/// on any project, so a merge never reports it as files changed outside every project.
enum CrewCoordination {
    /// The folders every archive holds, under names the user's own rules fix: a folder
    /// holding both is an archive's root.
    static let skeletonFolders = ["00-总览", "04-协调"]

    /// The archive a path lies in: the nearest folder above it that holds the skeleton.
    /// Nil for a path in none, or for anything that is not a file path.
    static func archiveRoot(of path: String) -> String? {
        guard !path.contains("://") else { return nil }
        let full = ((path as NSString).expandingTildeInPath as NSString).standardizingPath
        guard full.hasPrefix("/") else { return nil }
        let fileManager = FileManager.default
        var folder = (full as NSString).deletingLastPathComponent
        while !folder.isEmpty, folder != "/" {
            let root = folder
            if skeletonFolders.allSatisfy({ isDirectory((root as NSString).appendingPathComponent($0), fileManager) }) {
                return root
            }
            folder = (folder as NSString).deletingLastPathComponent
        }
        return nil
    }

    /// The folders of an archive a hand may write in (see `CrewPrompts.vaultRule`): the
    /// day's log, the coordination rows, the bug pages.
    static let handWritableFolders = ["02-日志", "04-协调", "06-BUG"]

    /// The INDEX a lead refreshes as it closes a job out, relative to the archive's root.
    static let indexPath = "00-总览/INDEX.md"

    /// Whether a path is part of the record a Crew job keeps: in a folder a hand may write
    /// in (`handWritableFolders`), or the INDEX the lead closes out. A merge says nothing of
    /// these: they are work on the record, not on any project.
    static func isRecordPath(_ path: String) -> Bool {
        guard let relative = relativeInArchive(path) else { return false }
        return relative == indexPath || handWritableFolders.contains { relative.hasPrefix($0 + "/") }
    }

    /// Whether a path lies in one of an archive's own numbered folders (`00-总览` to
    /// `06-BUG`) outside the record: the framework notes, the memory, the versions. The
    /// user's rules keep hands out of some of them and a lead's close-out rarely goes there,
    /// so a merge names these rather than passing over them; a file of a repository that
    /// merely holds an archive (its code, beside the numbered folders) is none of these.
    static func isArchiveNotePath(_ path: String) -> Bool {
        guard let relative = relativeInArchive(path), !isRecordPath(path),
              let folder = relative.split(separator: "/").first, relative.contains("/") else { return false }
        return folder.count > 3 && folder.prefix(2).allSatisfy(\.isNumber) && folder.dropFirst(2).first == "-"
    }

    /// A path relative to the root of the archive it lies in; nil for a path in none.
    private static func relativeInArchive(_ path: String) -> String? {
        guard let root = archiveRoot(of: path) else { return nil }
        let full = ((path as NSString).expandingTildeInPath as NSString).standardizingPath
        guard full.hasPrefix(root + "/") else { return nil }
        return String(full.dropFirst(root.count + 1))
    }

    private static func isDirectory(_ path: String, _ fileManager: FileManager) -> Bool {
        var isDir: ObjCBool = false
        return fileManager.fileExists(atPath: path, isDirectory: &isDir) && isDir.boolValue
    }
}
