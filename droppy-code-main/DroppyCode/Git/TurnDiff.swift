import Foundation

enum TurnDiff {
    static func merge(snapshot: [DiffFile]?, providerPatch: String, edits: [FileEdit], touched: Set<String>?, root: String, repositoryRoot: String? = nil, providerFiles: [DiffFile]? = nil) -> [DiffFile] {
        let repositoryRoot = repositoryRoot ?? root
        let directory = URL(fileURLWithPath: root, isDirectory: true)
        func reportedPath(_ path: String) -> String {
            let absolute = URL(fileURLWithPath: path, relativeTo: directory).standardizedFileURL.path
            return TouchedPaths.normalize(absolute, root: repositoryRoot)
        }
        let touched = touched.map { Set($0.map(reportedPath)) }
        let reported = providerFiles ?? DiffParser.parse(providerPatch)
        // The snapshot is the whole checkout between two checkpoints, so only files the chat's
        // turns reported, the provider's own patch, or its tool edits name are taken from it.
        var known = touched
        if known != nil {
            for file in reported {
                known?.insert(reportedPath(file.path))
                if let old = file.oldPath { known?.insert(reportedPath(old)) }
            }
            for edit in edits where !edit.path.isEmpty { known?.insert(reportedPath(edit.path)) }
        }
        var files: [String: DiffFile] = [:]
        for var file in snapshot ?? reported {
            file.path = snapshot == nil ? reportedPath(file.path) : TouchedPaths.normalize(file.path, root: repositoryRoot)
            file.oldPath = file.oldPath.map { snapshot == nil ? reportedPath($0) : TouchedPaths.normalize($0, root: repositoryRoot) }
            if let known, !known.contains(file.path), file.oldPath.map(known.contains) != true { continue }
            append(file, to: &files)
        }
        if snapshot != nil {
            for var file in reported {
                file.path = reportedPath(file.path)
                file.oldPath = file.oldPath.map(reportedPath)
                guard file.path.hasPrefix("/"), files[file.path] == nil else { continue }
                append(file, to: &files)
            }
        }
        let authoritative = Set(files.keys)
        for edit in edits where !edit.path.isEmpty {
            let path = reportedPath(edit.path)
            guard !authoritative.contains(path), snapshot == nil || path.hasPrefix("/") else { continue }
            var file = DiffParser.parseHunks(edit.diff ?? "", path: path)
            if file.hunks.isEmpty {
                file.reportedAdditions = edit.additions
                file.reportedDeletions = edit.deletions
            }
            append(file, to: &files)
        }
        return files.values.sorted { $0.path < $1.path }
    }

    private static func append(_ file: DiffFile, to files: inout [String: DiffFile]) {
        guard var existing = files[file.path] else { files[file.path] = file; return }
        let additions = existing.additions + file.additions
        let deletions = existing.deletions + file.deletions
        for var hunk in file.hunks {
            hunk.id = (existing.hunks.last?.id ?? 0) + 1
            existing.hunks.append(hunk)
        }
        existing.reportedAdditions = additions
        existing.reportedDeletions = deletions
        existing.isBinary = existing.isBinary || file.isBinary
        files[file.path] = existing
    }
}
