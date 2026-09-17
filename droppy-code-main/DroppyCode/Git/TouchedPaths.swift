import Foundation

/// Matches the files an agent reported editing against the files a diff contains.
enum TouchedPaths {
    /// The repository-relative form of a path an agent reported, whether absolute or relative.
    nonisolated static func normalize(_ path: String, root: String) -> String {
        guard !path.isEmpty else { return "" }
        let base = URL(fileURLWithPath: root, isDirectory: true).standardizedFileURL
        var path = URL(fileURLWithPath: path, relativeTo: base).standardizedFileURL.path
        let prefix = base.path.hasSuffix("/") ? base.path : base.path + "/"
        if path.hasPrefix(prefix) { path = String(path.dropFirst(prefix.count)) }
        return path
    }

    /// The repository-relative form of a path an agent reported, or nil when the path is
    /// no part of the checkout: another folder on the machine, a climb out of the root, a
    /// URL. A file written outside the checkout is not the thread's work, and git refuses
    /// to stage it, which used to take the whole merge down with it.
    nonisolated static func relative(_ path: String, root: String) -> String? {
        var path = path.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !path.isEmpty, !path.contains("://") else { return nil }
        if path.hasPrefix("~") { path = (path as NSString).expandingTildeInPath }
        let root = (root as NSString).standardizingPath
        let absolute = path.hasPrefix("/") ? path : (root as NSString).appendingPathComponent(path)
        let full = (absolute as NSString).standardizingPath
        let prefix = root.hasSuffix("/") ? root : root + "/"
        guard full.hasPrefix(prefix) else { return nil }
        return String(full.dropFirst(prefix.count)).nilIfEmpty
    }

    /// Whether a repository-relative path is build output or a tool cache that no one
    /// wants landed or merged, even where the project's .gitignore missed it: a head's
    /// build once left 2,700 compiler cache records under `DroppyCode.xcodeproj/-Xcc`
    /// in its copy, and every one rode along into the lead's checkout and the merge.
    /// Only names that are unmistakably caches count; a source folder called `build`
    /// or `dist` stays the project's business.
    nonisolated static func isBuildOutput(_ path: String) -> Bool {
        let components = path.split(separator: "/", omittingEmptySubsequences: true)
        return components.contains { component in
            let name = String(component)
            if name == ".DS_Store" || name.hasSuffix(".noindex") { return true }
            return buildOutputNames.contains(name)
        }
    }

    private static let buildOutputNames: Set<String> = [
        "-Xcc", "DerivedData", ".build", "node_modules", "__pycache__", ".pytest_cache", ".mypy_cache",
        ".ruff_cache", "xcuserdata", ".swiftpm", "ModuleCache", ".gradle", ".turbo", ".parcel-cache",
    ]

    /// Whether a diff file is one of the touched paths, allowing for a path reported relative to a subfolder.
    nonisolated static func matches(_ file: DiffFile, touched: Set<String>) -> Bool {
        [file.path, file.oldPath].compactMap { $0 }.contains { path in
            touched.contains(path) || touched.contains { $0.hasSuffix("/" + path) || path.hasSuffix("/" + $0) }
        }
    }
}
